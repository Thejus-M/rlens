//! Network actions: Google Lens upload, translation, local Ollama vision.
//! All functions block; the UI calls them from worker threads.

use anyhow::{Context, Result, bail};
use base64::Engine;
use image::RgbaImage;
use std::io::{BufRead, BufReader};
use std::time::Duration;

const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
                          (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

pub fn encode_png(img: &RgbaImage) -> Result<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)?;
    Ok(buf.into_inner())
}

// ---------------------------------------------------------------- translate

pub fn translate(text: &str, target: &str, cfg: &crate::config::Config) -> Result<String> {
    let t = &cfg.translate;
    match t.engine.as_str() {
        "google" => translate_google(text, target),
        "libretranslate" => {
            translate_libre(text, target, &t.libretranslate_url, &t.libretranslate_key)
        }
        "ollama" => translate_ollama(text, target, &cfg.ollama.url, &t.ollama_model),
        other => bail!("unknown translate engine `{other}` (use google, libretranslate or ollama)"),
    }
}

fn http() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(BROWSER_UA)
        .timeout(Duration::from_secs(30))
        .build()?)
}

/// Translate several texts with one request where the engine allows it.
/// When the online engine fails (for example Google rate-limits the
/// network with HTTP 429) and a local Ollama model is available, fall back
/// to it. The second value is a note for the user when a fallback was used.
pub fn translate_batch(
    texts: &[String],
    target: &str,
    cfg: &crate::config::Config,
) -> Result<(Vec<String>, Option<String>)> {
    let t = &cfg.translate;
    let primary = match t.engine.as_str() {
        "google" => google_batch(texts, target),
        "ollama" => ollama_batch(texts, target, cfg),
        _ => texts.iter().map(|x| translate(x, target, cfg)).collect(),
    };
    let err = match primary {
        Ok(v) => return Ok((v, None)),
        Err(e) => e,
    };
    let limited = format!("{err:#}").contains("429");
    if t.engine != "ollama" && ollama_has_model(&cfg.ollama.url, &t.ollama_model) {
        tracing::warn!(
            "translate: {err:#}; falling back to Ollama {}",
            t.ollama_model
        );
        let v = ollama_batch(texts, target, cfg)?;
        let why = if limited {
            "Google is rate-limiting this network"
        } else {
            "online translation failed"
        };
        return Ok((
            v,
            Some(format!("{why}; translated locally with {}", t.ollama_model)),
        ));
    }
    if limited {
        bail!(
            "Google Translate is rate-limiting this network (HTTP 429). Try again in a few minutes, or set [translate] engine = \"ollama\" in the config."
        );
    }
    Err(err)
}

/// All texts in one Google request, one per line; the reply keeps the line
/// breaks. Falls back to one request per text when the lines do not match.
fn google_batch(texts: &[String], target: &str) -> Result<Vec<String>> {
    let clean: Vec<String> = texts.iter().map(|t| t.replace(['\n', '\r'], " ")).collect();
    let joined = translate_google(&clean.join("\n"), target)?;
    let lines: Vec<String> = joined.split('\n').map(|l| l.trim().to_string()).collect();
    if lines.len() == clean.len() {
        return Ok(lines);
    }
    tracing::debug!(
        "google batch: {} lines for {} texts; per text",
        lines.len(),
        clean.len()
    );
    let mut out = Vec::new();
    for t in &clean {
        out.push(translate_google(t, target)?);
        std::thread::sleep(Duration::from_millis(150));
    }
    Ok(out)
}

fn ollama_batch(
    texts: &[String],
    target: &str,
    cfg: &crate::config::Config,
) -> Result<Vec<String>> {
    let (url, model) = (&cfg.ollama.url, &cfg.translate.ollama_model);
    if texts.len() == 1 {
        return Ok(vec![translate_ollama(&texts[0], target, url, model)?]);
    }
    // One prompt with numbered lines; parse the numbers back.
    let mut prompt = format!(
        "Translate each numbered line into the language with code `{target}`. \
         Reply with the same numbers, one line each, translation only.\n"
    );
    for (i, t) in texts.iter().enumerate() {
        prompt.push_str(&format!("{}. {}\n", i + 1, t.replace(['\n', '\r'], " ")));
    }
    let mut out = String::new();
    ollama_generate(url, model, &prompt, None, |c| out.push_str(c))?;
    match parse_numbered(&out, texts.len()) {
        Some(v) => Ok(v),
        None => texts
            .iter()
            .map(|t| translate_ollama(t, target, url, model))
            .collect(),
    }
}

/// Parse "1. text" lines into `n` strings, in order.
fn parse_numbered(reply: &str, n: usize) -> Option<Vec<String>> {
    let mut out = vec![None; n];
    for line in reply.lines() {
        let line = line.trim();
        let Some((num, rest)) = line.split_once(['.', ')', ':']) else {
            continue;
        };
        if let Ok(k) = num.trim().parse::<usize>()
            && (1..=n).contains(&k)
        {
            out[k - 1] = Some(rest.trim().to_string());
        }
    }
    out.into_iter().collect()
}

/// True when Ollama answers at `url` and has `model` pulled.
fn ollama_has_model(url: &str, model: &str) -> bool {
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(1500))
        .build()
    else {
        return false;
    };
    let Ok(resp) = client
        .get(format!("{}/api/tags", url.trim_end_matches('/')))
        .send()
    else {
        return false;
    };
    let Ok(v) = resp.json::<serde_json::Value>() else {
        return false;
    };
    v["models"]
        .as_array()
        .is_some_and(|m| m.iter().any(|x| x["name"].as_str() == Some(model)))
}

fn translate_google(text: &str, target: &str) -> Result<String> {
    let resp: serde_json::Value = http()?
        .post("https://translate.googleapis.com/translate_a/single")
        .query(&[
            ("client", "gtx"),
            ("sl", "auto"),
            ("tl", target),
            ("dt", "t"),
        ])
        .form(&[("q", text)])
        .send()?
        .error_for_status()?
        .json()?;
    parse_google_translate(&resp).context("unexpected Google Translate response")
}

pub fn parse_google_translate(v: &serde_json::Value) -> Option<String> {
    let parts = v.get(0)?.as_array()?;
    let mut out = String::new();
    for p in parts {
        if let Some(s) = p.get(0).and_then(|s| s.as_str()) {
            out.push_str(s);
        }
    }
    (!out.is_empty()).then_some(out)
}

fn translate_libre(text: &str, target: &str, url: &str, key: &str) -> Result<String> {
    let mut body = serde_json::json!({
        "q": text, "source": "auto", "target": target, "format": "text"
    });
    if !key.is_empty() {
        body["api_key"] = key.into();
    }
    let resp: serde_json::Value = http()?
        .post(format!("{}/translate", url.trim_end_matches('/')))
        .json(&body)
        .send()?
        .json()?;
    if let Some(s) = resp.get("translatedText").and_then(|s| s.as_str()) {
        return Ok(s.to_string());
    }
    bail!("LibreTranslate: {}", resp.get("error").unwrap_or(&resp))
}

fn translate_ollama(text: &str, target: &str, url: &str, model: &str) -> Result<String> {
    let prompt = format!(
        "Translate the following text into the language with code `{target}`. \
         Reply with the translation only.\n\n{text}"
    );
    let mut out = String::new();
    ollama_generate(url, model, &prompt, None, |chunk| out.push_str(chunk))?;
    Ok(out.trim().to_string())
}

// ---------------------------------------------------------------- Ollama vision

/// Stream a description of the image from a local Ollama vision model.
pub fn ollama_vision(
    img: &RgbaImage,
    url: &str,
    model: &str,
    prompt: &str,
    on_chunk: impl FnMut(&str),
) -> Result<()> {
    let png = encode_png(img)?;
    tracing::info!(
        "ollama: sending {}x{} px to {model}",
        img.width(),
        img.height()
    );
    let b64 = base64::engine::general_purpose::STANDARD.encode(png);
    ollama_generate(url, model, prompt, Some(b64), on_chunk)
}

fn ollama_generate(
    url: &str,
    model: &str,
    prompt: &str,
    image_b64: Option<String>,
    mut on_chunk: impl FnMut(&str),
) -> Result<()> {
    // `think: false`: reasoning models (gemma4, qwen3) answer 10× faster.
    let mut body =
        serde_json::json!({ "model": model, "prompt": prompt, "stream": true, "think": false });
    if let Some(b64) = image_b64 {
        body["images"] = serde_json::json!([b64]);
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(600))
        .build()?;
    let resp = client
        .post(format!("{}/api/generate", url.trim_end_matches('/')))
        .json(&body)
        .send()
        .with_context(|| format!("connect to Ollama at {url} (is `ollama serve` running?)"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        bail!("Ollama HTTP {status}: {}", text.trim());
    }
    for line in BufReader::new(resp).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(&line)?;
        if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
            bail!("Ollama: {err}");
        }
        if let Some(s) = v.get("response").and_then(|s| s.as_str()) {
            on_chunk(s);
        }
        if v.get("done").and_then(|d| d.as_bool()) == Some(true) {
            break;
        }
    }
    Ok(())
}

pub fn open_url(url: &str) -> Result<()> {
    std::process::Command::new("xdg-open")
        .arg(url)
        .spawn()
        .context("run xdg-open")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_translate_parse() {
        let v: serde_json::Value = serde_json::from_str(
            r#"[[["Hello ","Hola ",null,null,10],["world","mundo",null,null,10]],null,"es"]"#,
        )
        .unwrap();
        assert_eq!(parse_google_translate(&v).unwrap(), "Hello world");
    }

    #[test]
    #[ignore = "network"]
    fn live_translate() {
        let path = format!("{}/tests/fixtures/text.png", env!("CARGO_MANIFEST_DIR"));
        let img = image::open(path).unwrap().to_rgba8();
        let _ = img;
        let cfg = crate::config::Config::default();
        let t = translate("Hola mundo", "en", &cfg).unwrap();
        println!("translate: {t}");
        assert!(t.to_lowercase().contains("hello"));
    }

    #[test]
    #[ignore = "needs local Ollama"]
    fn live_ollama_vision() {
        let path = format!("{}/tests/fixtures/text.png", env!("CARGO_MANIFEST_DIR"));
        let img = image::open(path).unwrap().to_rgba8();
        let cfg = crate::config::OllamaConfig::default();
        let mut out = String::new();
        ollama_vision(
            &img,
            &cfg.url,
            &cfg.vision_model,
            "Read the text in this image.",
            |c| out.push_str(c),
        )
        .unwrap();
        println!("ollama: {out}");
        assert!(!out.trim().is_empty());
    }

    #[test]
    fn numbered_reply() {
        let r = "1. Humans can live!\n2) - Bill, April 2015\n";
        assert_eq!(
            parse_numbered(r, 2).unwrap(),
            vec!["Humans can live!", "- Bill, April 2015"]
        );
        assert!(parse_numbered("1. only one", 2).is_none());
    }
}
