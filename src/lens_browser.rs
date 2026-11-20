//! Google Lens results in a browser side panel.
//!
//! Google's result page only renders in a real browser, and the result URL
//! is bound to the cookies of the session that uploaded the image. So the
//! browser does the upload itself: rlens writes a small local page that holds
//! the selection as an embedded PNG and posts it to Lens on load.
//!
//! A Chromium-family browser opens the page as a narrow app window at the
//! left or right edge of the screen. Without one, the page opens in the
//! default browser.

use anyhow::{Context, Result};
use base64::Engine;
use image::RgbaImage;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// Side-panel width in logical pixels.
const PANEL_W: i32 = 460;
const GAP: i32 = 12;

const CHROMIUM_FAMILY: &[&str] = &[
    "chromium",
    "brave",
    "brave-browser",
    "google-chrome-stable",
    "google-chrome",
    "microsoft-edge-stable",
    "vivaldi-stable",
    "vivaldi",
    "thorium-browser",
];

fn runtime_dir() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("rlens")
}

/// Remove upload pages older than ten minutes.
fn cleanup(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > Duration::from_secs(600));
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Write the self-submitting upload page and return its path.
pub fn upload_page(img: &RgbaImage, hl: &str, viewport: (i32, i32)) -> Result<PathBuf> {
    let png = crate::net::encode_png(img)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    let hl: String = hl
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    let action = format!(
        "https://lens.google.com/v3/upload?hl={hl}&re=df&stcs={ts}&vpw={}&vph={}&ep=subb",
        viewport.0, viewport.1
    );
    let html = format!(
        r#"<!doctype html>
<html><head><meta charset="utf-8"><title>rlens · Google Lens</title>
<style>
html,body{{margin:0;height:100%;background:#202124;color:#e8eaed;font:15px system-ui,sans-serif}}
body{{display:flex;align-items:center;justify-content:center}}
.s{{width:28px;height:28px;border:3px solid #8ab4f8;border-top-color:transparent;border-radius:50%;animation:r .8s linear infinite;margin-right:14px}}
@keyframes r{{to{{transform:rotate(360deg)}}}}
</style></head>
<body><div class="s"></div>Searching with Google Lens…
<form id="f" method="POST" enctype="multipart/form-data" action="{action}" style="display:none">
<input type="file" name="encoded_image" id="i"></form>
<script>
const b = atob("{b64}");
const u = new Uint8Array(b.length);
for (let k = 0; k < b.length; k++) u[k] = b.charCodeAt(k);
const dt = new DataTransfer();
dt.items.add(new File([u], "image.png", {{type: "image/png"}}));
document.getElementById("i").files = dt.files;
document.getElementById("f").submit();
</script></body></html>"#
    );
    let dir = runtime_dir();
    std::fs::create_dir_all(&dir)?;
    cleanup(&dir);
    let path = dir.join(format!("lens-{ts}.html"));
    std::fs::write(&path, html)?;
    Ok(path)
}

fn is_running(name: &str) -> bool {
    Command::new("pgrep")
        .args(["-x", name])
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The configured browser, else a running Chromium-family browser (its
/// Google cookies avoid captchas), else the first one installed.
fn find_browser(preferred: &str) -> Option<PathBuf> {
    if !preferred.is_empty() && preferred != "auto" {
        return which::which(preferred).ok();
    }
    CHROMIUM_FAMILY
        .iter()
        .filter(|b| is_running(b))
        .chain(CHROMIUM_FAMILY.iter())
        .find_map(|b| which::which(b).ok())
}

/// Hyprland address of the current Lens panel window.
static PANEL: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Monitor geometry in logical pixels: x, y, width, height.
pub fn monitor_geometry() -> Option<(i32, i32, i32, i32)> {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        let out = Command::new("hyprctl")
            .args(["monitors", "-j"])
            .output()
            .ok()?;
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        let m = v
            .as_array()?
            .iter()
            .find(|m| m["focused"].as_bool() == Some(true))?;
        let scale = m["scale"].as_f64().unwrap_or(1.0);
        let (w, h) = (m["width"].as_f64()?, m["height"].as_f64()?);
        // Rotated outputs (transform 1, 3, 5, 7) swap width and height.
        let rot = m["transform"].as_i64().unwrap_or(0) % 2 == 1;
        let (w, h) = if rot { (h, w) } else { (w, h) };
        return Some((
            m["x"].as_i64()? as i32,
            m["y"].as_i64()? as i32,
            (w / scale) as i32,
            (h / scale) as i32,
        ));
    }
    if std::env::var_os("SWAYSOCK").is_some() {
        let out = Command::new("swaymsg")
            .args(["-t", "get_outputs", "-r"])
            .output()
            .ok()?;
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        let r = &v
            .as_array()?
            .iter()
            .find(|m| m["focused"].as_bool() == Some(true))?["rect"];
        return Some((
            r["x"].as_i64()? as i32,
            r["y"].as_i64()? as i32,
            r["width"].as_i64()? as i32,
            r["height"].as_i64()? as i32,
        ));
    }
    None
}

/// Panel rectangle (x, y, w, h) on the given monitor.
fn panel_rect(mon: (i32, i32, i32, i32), side: Side) -> (i32, i32, i32, i32) {
    let (mx, my, mw, mh) = mon;
    let w = PANEL_W.min(mw - 2 * GAP);
    let h = mh - 2 * GAP;
    let x = match side {
        Side::Left => mx + GAP,
        Side::Right => mx + mw - w - GAP,
    };
    (x, my + GAP, w, h)
}

fn hypr_clients() -> Option<Vec<serde_json::Value>> {
    let out = Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()
        .ok()?;
    serde_json::from_slice::<serde_json::Value>(&out.stdout)
        .ok()?
        .as_array()
        .cloned()
}

/// Run a Hyprland dispatch. Hyprland 0.55+ (Lua config) takes `hl.dsp.*`
/// calls; older versions take the classic dispatcher syntax.
fn hypr_dispatch(lua: &str, classic: &str) {
    let ok = Command::new("hyprctl")
        .args(["dispatch", lua])
        .output()
        .is_ok_and(|o| {
            let t = String::from_utf8_lossy(&o.stdout);
            o.status.success() && !t.contains("error")
        });
    if !ok {
        let _ = Command::new("hyprctl")
            .args(["dispatch"])
            .args(classic.split(' '))
            .output();
    }
}

/// Which window to place.
#[derive(Clone)]
pub enum Target {
    Pid(u32),
    /// Substring of the window class / app_id.
    Class(String),
}

impl Target {
    fn matches(&self, c: &serde_json::Value) -> bool {
        match self {
            Target::Pid(pid) => c["pid"].as_u64() == Some(*pid as u64),
            Target::Class(sub) => c["class"]
                .as_str()
                .is_some_and(|k| k.contains(sub.as_str())),
        }
    }

    fn sway_criteria(&self) -> String {
        match self {
            Target::Pid(pid) => format!("[pid={pid}]"),
            Target::Class(sub) => format!("[app_id=\"{}\"]", sub.replace('.', "\\.")),
        }
    }
}

/// Float, size and move a window (compositors with an IPC). `pin` keeps it
/// above other floating windows on every workspace.
pub fn place(target: Target, rect: (i32, i32, i32, i32), pin: bool) {
    let (x, y, w, h) = rect;
    let deadline = Instant::now() + Duration::from_secs(10);
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        while Instant::now() < deadline {
            let addr = hypr_clients().and_then(|v| {
                v.iter()
                    .find(|c| target.matches(c))
                    .and_then(|c| c["address"].as_str().map(str::to_string))
            });
            if let Some(addr) = addr {
                let t = format!("address:{addr}");
                // Chromium can map its window maximized or fullscreen first.
                std::thread::sleep(Duration::from_millis(250));
                for mode in ["fullscreen", "maximized"] {
                    hypr_dispatch(
                        &format!(
                            r#"hl.dsp.window.fullscreen({{ mode = "{mode}", action = "unset", window = "{t}" }})"#
                        ),
                        &format!("fullscreenstate 0 -1,{t}"),
                    );
                }
                hypr_dispatch(
                    &format!(r#"hl.dsp.window.float({{ action = "on", window = "{t}" }})"#),
                    &format!("setfloating {t}"),
                );
                hypr_dispatch(
                    &format!(r#"hl.dsp.window.resize({{ x = {w}, y = {h}, window = "{t}" }})"#),
                    &format!("resizewindowpixel exact {w} {h},{t}"),
                );
                hypr_dispatch(
                    &format!(r#"hl.dsp.window.move({{ x = {x}, y = {y}, window = "{t}" }})"#),
                    &format!("movewindowpixel exact {x} {y},{t}"),
                );
                if pin {
                    hypr_dispatch(
                        &format!(r#"hl.dsp.window.pin({{ action = "on", window = "{t}" }})"#),
                        &format!("pin {t}"),
                    );
                }
                hypr_dispatch(
                    &format!(r#"hl.dsp.focus({{ window = "{t}" }})"#),
                    &format!("focuswindow {t}"),
                );
                if pin {
                    *PANEL.lock().unwrap() = Some(t);
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        return;
    }
    if std::env::var_os("SWAYSOCK").is_some() {
        let sticky = if pin { ", sticky enable" } else { "" };
        while Instant::now() < deadline {
            let cmd = format!(
                "{} floating enable, border none, resize set {w} {h}, move absolute position {x} {y}{sticky}, focus",
                target.sway_criteria()
            );
            let ok = Command::new("swaymsg")
                .arg(&cmd)
                .output()
                .is_ok_and(|o| o.status.success());
            if ok {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// True on compositors where rlens places windows itself.
pub fn can_place() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
        || std::env::var_os("SWAYSOCK").is_some()
}

/// Upload `img` to Google Lens in a browser side panel.
/// Returns the thread that places the panel, when there is one.
pub fn open(
    img: &RgbaImage,
    side: Side,
    hl: &str,
    browser: &str,
) -> Result<Option<std::thread::JoinHandle<()>>> {
    tracing::info!(
        "google lens: {}x{} px selection, panel on the {side:?}",
        img.width(),
        img.height()
    );
    let mon = monitor_geometry().unwrap_or((0, 0, 1920, 1080));
    let rect = panel_rect(mon, side);
    let page = upload_page(img, hl, (rect.2, rect.3))?;
    let url = url::Url::from_file_path(&page)
        .map_err(|_| anyhow::anyhow!("bad page path"))?
        .to_string();

    let Some(bin) = find_browser(browser) else {
        tracing::info!("no Chromium-family browser; opening the default browser");
        crate::net::open_url(&url)?;
        return Ok(None);
    };
    close_panel();
    let stem = page
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("lens")
        .to_string();
    Command::new(&bin)
        .arg(format!("--app={url}"))
        .arg(format!("--window-size={},{}", rect.2, rect.3))
        .arg(format!("--window-position={},{}", rect.0, rect.1))
        .args(["--no-first-run", "--no-default-browser-check"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("start {}", bin.display()))?;
    // The app window's class holds the page file name, for example
    // `chrome-__run_user_1000_rlens_lens-1795195279645.html-Default`.
    Ok(Some(std::thread::spawn(move || {
        place(Target::Class(stem), rect, true)
    })))
}

/// Close the previous Lens panel window.
pub fn close_panel() {
    let Some(t) = PANEL.lock().unwrap().take() else {
        return;
    };
    hypr_dispatch(
        &format!(r#"hl.dsp.window.close({{ window = "{t}" }})"#),
        &format!("closewindow {t}"),
    );
}

/// Unpin the Lens panel so it behaves as a normal window after the overlay
/// closes.
pub fn release_panel() {
    let Some(t) = PANEL.lock().unwrap().take() else {
        return;
    };
    hypr_dispatch(
        &format!(r#"hl.dsp.window.pin({{ action = "off", window = "{t}" }})"#),
        &format!("pin {t}"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_sides() {
        let mon = (0, 0, 1920, 1080);
        assert_eq!(
            panel_rect(mon, Side::Right),
            (1920 - 460 - 12, 12, 460, 1056)
        );
        assert_eq!(panel_rect(mon, Side::Left), (12, 12, 460, 1056));
    }

    #[test]
    fn page_embeds_image_and_form() {
        let img = RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255]));
        let p = upload_page(&img, "en", (460, 1000)).unwrap();
        let html = std::fs::read_to_string(&p).unwrap();
        assert!(html.contains("lens.google.com/v3/upload?hl=en"));
        assert!(html.contains("encoded_image"));
        assert!(html.contains("vpw=460&vph=1000"));
        let _ = std::fs::remove_file(p);
    }
}
