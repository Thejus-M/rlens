//! OCR through the `tesseract` command-line tool (TSV output gives word boxes).

use anyhow::{Context, Result, bail};
use image::{DynamicImage, RgbaImage, imageops::FilterType};
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    /// Box in source-image pixels: x, y, width, height.
    pub rect: [f32; 4],
    pub conf: f32,
    pub block: u32,
    pub par: u32,
    pub line: u32,
}

#[derive(Debug, Clone, Default)]
pub struct OcrResult {
    pub words: Vec<Word>,
    pub text: String,
}

/// Run OCR on an image. Small images are scaled up first for accuracy.
pub fn recognize(img: &RgbaImage, langs: &str) -> Result<OcrResult> {
    if which::which("tesseract").is_err() {
        bail!(
            "tesseract not found. Install it (package `tesseract` plus a language pack such as `tesseract-data-eng` / `tesseract-ocr-eng`)."
        );
    }
    let scale: f32 = if img.height() < 400 || img.width() < 400 {
        2.0
    } else {
        1.0
    };
    let work = if scale > 1.0 {
        DynamicImage::ImageRgba8(img.clone()).resize(
            (img.width() as f32 * scale) as u32,
            (img.height() as f32 * scale) as u32,
            FilterType::Lanczos3,
        )
    } else {
        DynamicImage::ImageRgba8(img.clone())
    };
    let langs = resolve_langs(langs);

    // Two passes at the same time: the image as it is, and a clean
    // black-on-white version. Busy or dark backgrounds (video frames,
    // glowing text) defeat Tesseract's own layout analysis; the clean
    // version fixes that. Keep the pass with more confident text.
    let plain = image::DynamicImage::ImageRgb8(work.to_rgb8());
    let clean = image::DynamicImage::ImageLuma8(binarize(&work.to_luma8()));
    if let Ok(path) = std::env::var("RLENS_OCR_DUMP") {
        let _ = clean.save(&path);
    }
    let (a, b) = std::thread::scope(|s| {
        let a = s.spawn(|| run_tesseract(&plain, &langs));
        let b = s.spawn(|| run_tesseract(&clean, &langs));
        (a.join(), b.join())
    });
    let a = a.map_err(|_| anyhow::anyhow!("OCR thread panicked"))??;
    let b = b.map_err(|_| anyhow::anyhow!("OCR thread panicked"))??;
    tracing::debug!("ocr score: plain {} clean {}", score(&a), score(&b));
    let mut words = if score(&b) > score(&a) { b } else { a };

    for w in &mut words {
        for v in &mut w.rect {
            *v /= scale;
        }
    }
    let text = join_words(&words);
    Ok(OcrResult { words, text })
}

fn run_tesseract(img: &DynamicImage, langs: &str) -> Result<Vec<Word>> {
    let tmp = tempfile::Builder::new()
        .prefix("rlens-ocr-")
        .suffix(".png")
        .tempfile()?;
    img.save(tmp.path())?;
    let out = Command::new("tesseract")
        .arg(tmp.path())
        .arg("-")
        .args(["-l", langs, "--psm", "3"])
        // TSV through flags, not the `tsv` config file: that file is missing
        // from some tessdata folders (custom TESSDATA_PREFIX).
        .args(["-c", "tessedit_create_tsv=1", "-c", "tessedit_create_txt=0"])
        .output()
        .context("run tesseract")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("tesseract failed: {}", err.trim());
    }
    Ok(split_wide_gaps(parse_tsv(&String::from_utf8_lossy(
        &out.stdout,
    ))))
}

/// Confidence-weighted count of letters and digits. Glowing or stylised
/// text often scores only 30–60, so low-confidence words still count, less.
fn score(words: &[Word]) -> f32 {
    words
        .iter()
        .filter(|w| w.conf > 0.0)
        .map(|w| w.text.chars().filter(|c| c.is_alphanumeric()).count() as f32 * w.conf / 100.0)
        .sum()
}

/// Otsu threshold of a grey image.
fn otsu(img: &image::GrayImage) -> u8 {
    let mut hist = [0u64; 256];
    for p in img.pixels() {
        hist[p[0] as usize] += 1;
    }
    let total: u64 = hist.iter().sum();
    let sum_all: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, &c)| i as f64 * c as f64)
        .sum();
    let (mut w_b, mut sum_b, mut best, mut best_t) = (0u64, 0f64, 0f64, 128u8);
    for (t, &c) in hist.iter().enumerate() {
        w_b += c;
        if w_b == 0 {
            continue;
        }
        let w_f = total - w_b;
        if w_f == 0 {
            break;
        }
        sum_b += t as f64 * c as f64;
        let (m_b, m_f) = (sum_b / w_b as f64, (sum_all - sum_b) / w_f as f64);
        let between = w_b as f64 * w_f as f64 * (m_b - m_f).powi(2);
        if between > best {
            best = between;
            best_t = t as u8;
        }
    }
    best_t
}

/// Black text on white. Dark images (light text on a dark background) are
/// inverted, and their threshold is kept high so only the bright text stays,
/// not background lines or glow.
pub fn binarize(gray: &image::GrayImage) -> image::GrayImage {
    let n = (gray.width() as u64 * gray.height() as u64).max(1);
    let mean = gray.pixels().map(|p| p[0] as u64).sum::<u64>() / n;
    let dark = mean < 110;
    let t = if dark {
        otsu(gray).max(140)
    } else {
        otsu(gray)
    };
    image::GrayImage::from_fn(gray.width(), gray.height(), |x, y| {
        let v = gray.get_pixel(x, y)[0];
        let text = if dark { v > t } else { v <= t };
        image::Luma([if text { 0 } else { 255 }])
    })
}

/// Language packs that are not scripts for normal text.
const NOT_TEXT: &[&str] = &["osd", "equ", "snum"];
/// Most extra languages `auto` adds; each one makes OCR slower.
const AUTO_MAX_EXTRA: usize = 3;

/// The languages tesseract has data for.
pub fn installed_langs() -> Vec<String> {
    static CACHE: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            let Ok(out) = Command::new("tesseract").arg("--list-langs").output() else {
                return Vec::new();
            };
            // The list goes to stdout on new versions and stderr on old ones.
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.contains(' ') && !l.ends_with(':'))
                .map(str::to_string)
                .collect()
        })
        .clone()
}

/// Turn the configured language setting into a tesseract `-l` value.
/// `auto` means English plus up to three other installed language packs.
pub fn resolve_langs(setting: &str) -> String {
    if setting.trim() != "auto" {
        return setting.to_string();
    }
    pick_auto(&installed_langs())
}

fn pick_auto(installed: &[String]) -> String {
    let mut out = vec!["eng".to_string()];
    out.extend(
        installed
            .iter()
            .filter(|l| *l != "eng" && !NOT_TEXT.contains(&l.as_str()))
            .take(AUTO_MAX_EXTRA)
            .cloned(),
    );
    out.join("+")
}

/// Split a Tesseract line where the gap between two words is wider than
/// 2.5× the text height. Such a gap means two separate pieces of text (for
/// example a caption and a stray word on a picture) that Tesseract joined.
/// Each piece gets its own paragraph number, so it is grouped, translated
/// and placed on its own.
pub fn split_wide_gaps(mut words: Vec<Word>) -> Vec<Word> {
    let orig = words.clone();
    // Fragments of the current Tesseract paragraph: (left x, id).
    let mut segs: Vec<(f32, u32)> = Vec::new();
    let mut cur = 0u32;
    for i in 0..orig.len() {
        let w = &orig[i];
        let h = w.rect[3].max(1.0);
        let prev = i.checked_sub(1).map(|k| &orig[k]);
        let new_par = prev.is_none_or(|p| (p.block, p.par) != (w.block, w.par));
        let new_line = new_par || prev.is_some_and(|p| p.line != w.line);
        if new_par {
            segs.clear();
        }
        let wide_gap =
            !new_line && prev.is_some_and(|p| w.rect[0] - (p.rect[0] + p.rect[2]) > h * 2.5);
        if new_line || wide_gap {
            // Continue the fragment whose left edge is nearest, else start one.
            let near = segs
                .iter()
                .filter(|(x, _)| (x - w.rect[0]).abs() < h * 3.0)
                .min_by(|a, b| (a.0 - w.rect[0]).abs().total_cmp(&(b.0 - w.rect[0]).abs()))
                .map(|&(_, id)| id);
            cur = match near {
                Some(id) => id,
                None => {
                    let id = segs.len() as u32;
                    segs.push((w.rect[0], id));
                    id
                }
            };
        }
        words[i].par = w.par + 10_000 * cur;
    }
    words
}

/// Parse tesseract TSV. Keep only word rows (level 5) with text.
pub fn parse_tsv(tsv: &str) -> Vec<Word> {
    let mut words = Vec::new();
    for line in tsv.lines().skip(1) {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 12 || cols[0] != "5" {
            continue;
        }
        let text = cols[11].trim();
        if text.is_empty() {
            continue;
        }
        let num = |i: usize| cols[i].parse::<f32>().unwrap_or(0.0);
        words.push(Word {
            text: text.to_string(),
            rect: [num(6), num(7), num(8), num(9)],
            conf: num(10),
            block: num(2) as u32,
            par: num(3) as u32,
            line: num(4) as u32,
        });
    }
    words
}

/// Rebuild plain text: spaces between words, newline between lines,
/// blank line between paragraphs.
pub fn join_words(words: &[Word]) -> String {
    let mut out = String::new();
    let mut prev: Option<&Word> = None;
    for w in words {
        if let Some(p) = prev {
            if p.block != w.block || p.par != w.par {
                out.push_str("\n\n");
            } else if p.line != w.line {
                out.push('\n');
            } else {
                out.push(' ');
            }
        }
        out.push_str(&w.text);
        prev = Some(w);
    }
    out
}

/// One OCR paragraph: the union box of its words and its text.
#[derive(Debug, Clone, PartialEq)]
pub struct Para {
    /// x, y, width, height in source-image pixels.
    pub rect: [f32; 4],
    pub text: String,
    /// Average height of one text line, for font sizing.
    pub line_height: f32,
    /// Index range of its words in the OCR word list.
    pub words: std::ops::Range<usize>,
    /// Mean Tesseract confidence of its words (0–100).
    pub conf: f32,
}

impl Para {
    /// True when the paragraph looks like real text: enough letters and a
    /// fair OCR confidence. Icons and noise read as text fail this.
    pub fn is_text_like(&self) -> bool {
        let chars: Vec<char> = self.text.chars().filter(|c| !c.is_whitespace()).collect();
        if chars.len() < 2 {
            return false;
        }
        // A lone short word with weak confidence is usually a stray read on
        // a picture or an icon.
        if self.words.len() == 1 && chars.len() <= 4 && self.conf < 90.0 {
            return false;
        }
        let letters = chars.iter().filter(|c| c.is_alphabetic()).count();
        // Tesseract scores many correct words of some scripts (Tamil, Hindi)
        // at 0, so confidence only counts for short paragraphs.
        letters as f32 / chars.len() as f32 >= 0.5 && (self.words.len() >= 3 || self.conf >= 35.0)
    }
}

/// One OCR text line.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// x, y, width, height in source-image pixels.
    pub rect: [f32; 4],
    /// Index range of its words in the OCR word list.
    pub words: std::ops::Range<usize>,
    /// Index of its paragraph in `paragraphs()`.
    pub para: usize,
}

fn union(words: &[Word]) -> [f32; 4] {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for w in words {
        x0 = x0.min(w.rect[0]);
        y0 = y0.min(w.rect[1]);
        x1 = x1.max(w.rect[0] + w.rect[2]);
        y1 = y1.max(w.rect[1] + w.rect[3]);
    }
    [x0, y0, x1 - x0, y1 - y0]
}

/// Text lines (same block, paragraph and line number), each tagged with the
/// paragraph from `paras` that holds its first word.
pub fn lines(words: &[Word], paras: &[Para]) -> Vec<Line> {
    let mut out = Vec::new();
    let mut start = 0;
    for i in 1..=words.len() {
        let end = i == words.len()
            || words[i].block != words[start].block
            || words[i].par != words[start].par
            || words[i].line != words[start].line;
        if !end {
            continue;
        }
        let para = paras
            .iter()
            .position(|p| p.words.contains(&start))
            .unwrap_or(0);
        out.push(Line {
            rect: union(&words[start..i]),
            words: start..i,
            para,
        });
        start = i;
    }
    out
}

/// Spread translated `text` over lines with the given widths, in reading
/// order, so each line gets a share of words in proportion to its width.
/// Text without spaces (Chinese, Japanese) is split by characters.
pub fn split_to_lines(text: &str, widths: &[f32]) -> Vec<String> {
    let n = widths.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![text.trim().to_string()];
    }
    let spaced = text.split_whitespace().count() > 1;
    let tokens: Vec<String> = if spaced {
        text.split_whitespace().map(str::to_string).collect()
    } else {
        text.trim().chars().map(|c| c.to_string()).collect()
    };
    let sep = if spaced { " " } else { "" };
    let total_w: f32 = widths.iter().sum::<f32>().max(1.0);
    let total_c: usize = tokens
        .iter()
        .map(|t| t.chars().count() + 1)
        .sum::<usize>()
        .max(1);

    let mut out = vec![Vec::<String>::new(); n];
    let mut line = 0;
    let mut acc_w = widths[0] / total_w;
    let mut acc_c = 0usize;
    for (k, t) in tokens.iter().enumerate() {
        let len = t.chars().count() + 1;
        let mid = (acc_c as f32 + len as f32 / 2.0) / total_c as f32;
        // Move to the next line when this token's middle passes the current
        // line's share, but keep enough tokens for the lines that are left.
        let left = tokens.len() - k;
        while line + 1 < n && (mid > acc_w || left < n - line) && !out[line].is_empty() {
            line += 1;
            acc_w += widths[line] / total_w;
        }
        out[line].push(t.clone());
        acc_c += len;
    }
    out.into_iter().map(|v| v.join(sep)).collect()
}

/// Paragraphs for translation: Tesseract paragraphs, with split sentences
/// merged back together.
#[cfg(test)]
pub fn paragraphs(words: &[Word]) -> Vec<Para> {
    paragraphs_where(words, |_, _| true)
}

/// Like `paragraphs`, but two paragraphs merge only when `can_merge`
/// agrees (for example, when their background colours match).
pub fn paragraphs_where(words: &[Word], can_merge: impl Fn(&Para, &Para) -> bool) -> Vec<Para> {
    merge_close(group(words, &can_merge), can_merge)
}

/// One paragraph-shaped record for a run of words.
fn para_of(words: &[Word], range: std::ops::Range<usize>) -> Para {
    let group = &words[range.clone()];
    let rect = union(group);
    let lines = {
        let mut l: Vec<(u32, u32, u32)> = group.iter().map(|w| (w.block, w.par, w.line)).collect();
        l.dedup();
        l.len().max(1) as f32
    };
    let c: Vec<f32> = group.iter().map(|w| w.conf).filter(|c| *c >= 0.0).collect();
    Para {
        rect,
        text: join_words(group).replace('\n', " "),
        line_height: rect[3] / lines,
        words: range,
        conf: if c.is_empty() {
            0.0
        } else {
            c.iter().sum::<f32>() / c.len() as f32
        },
    }
}

/// Append `p` (the next lines in reading order) to `last`.
fn join(last: &mut Para, p: Para) {
    let (na, nb) = (last.words.len() as f32, p.words.len() as f32);
    last.conf = (last.conf * na + p.conf * nb) / (na + nb).max(1.0);
    let x0 = last.rect[0].min(p.rect[0]);
    let y0 = last.rect[1].min(p.rect[1]);
    let x1 = (last.rect[0] + last.rect[2]).max(p.rect[0] + p.rect[2]);
    let y1 = (last.rect[1] + last.rect[3]).max(p.rect[1] + p.rect[3]);
    let lines = (last.rect[3] / last.line_height) + (p.rect[3] / p.line_height);
    last.text = format!("{} {}", last.text, p.text);
    last.rect = [x0, y0, x1 - x0, y1 - y0];
    last.line_height = (y1 - y0) / lines.max(1.0);
    last.words = last.words.start..p.words.end;
}

/// Group words into paragraphs: lines of the same Tesseract block and
/// paragraph, split where a line does not continue the one above (it jumps
/// left or right, changes size, leaves a big gap, or `can_join` refuses,
/// for example over a different background).
fn group(words: &[Word], can_join: &dyn Fn(&Para, &Para) -> bool) -> Vec<Para> {
    let mut out: Vec<Para> = Vec::new();
    let mut prev_line: Option<Para> = None;
    let mut start = 0;
    for i in 1..=words.len() {
        let end = i == words.len()
            || words[i].block != words[start].block
            || words[i].par != words[start].par
            || words[i].line != words[start].line;
        if !end {
            continue;
        }
        let line = para_of(words, start..i);
        let same_par = start > 0
            && words[start - 1].block == words[start].block
            && words[start - 1].par == words[start].par;
        let continues = same_par
            && prev_line.as_ref().is_some_and(|prev| {
                let lh = prev.line_height.max(line.line_height);
                let gap = line.rect[1] - (prev.rect[1] + prev.rect[3]);
                (line.rect[0] - prev.rect[0]).abs() < lh * 2.0
                    && (line.line_height - prev.line_height).abs() < lh * 0.4
                    && gap < lh * 1.5
                    && can_join(prev, &line)
            });
        prev_line = Some(line.clone());
        match out.last_mut() {
            Some(last) if continues => join(last, line),
            _ => out.push(line),
        }
        start = i;
    }
    out
}

/// Tesseract often splits one sentence into several paragraphs when the
/// line gap is large. Merge a paragraph into the one above when the gap is
/// small and the left edges line up, so each sentence is translated whole.
fn merge_close(paras: Vec<Para>, can_merge: impl Fn(&Para, &Para) -> bool) -> Vec<Para> {
    let mut out: Vec<Para> = Vec::new();
    for p in paras {
        if let Some(last) = out.last_mut() {
            let lh = last.line_height.max(p.line_height);
            let gap = p.rect[1] - (last.rect[1] + last.rect[3]);
            let aligned = (p.rect[0] - last.rect[0]).abs() < lh * 1.5;
            let similar = (p.line_height - last.line_height).abs() < lh * 0.35;
            let ends_sentence = last.text.trim_end().ends_with(['.', '!', '?', ':', '。']);
            if gap >= 0.0
                && gap < lh * 1.8
                && aligned
                && similar
                && !ends_sentence
                && can_merge(last, &p)
            {
                join(last, p);
                continue;
            }
        }
        out.push(p);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TSV: &str = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext
1\t1\t0\t0\t0\t0\t0\t0\t200\t100\t-1\t
5\t1\t1\t1\t1\t1\t10\t10\t40\t12\t96.5\tHello
5\t1\t1\t1\t1\t2\t55\t10\t50\t12\t95.1\tworld
5\t1\t1\t1\t2\t1\t10\t30\t30\t12\t91.0\tline2
5\t1\t2\t1\t1\t1\t10\t60\t30\t12\t90.0\tpara
5\t1\t2\t1\t1\t2\t50\t60\t30\t12\t-1\t ";

    #[test]
    fn auto_langs() {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(pick_auto(&v(&["eng", "osd"])), "eng");
        assert_eq!(pick_auto(&v(&["eng", "osd", "tam"])), "eng+tam");
        assert_eq!(
            pick_auto(&v(&["ben", "eng", "hin", "mal", "osd", "tam"])),
            "eng+ben+hin+mal"
        );
        assert_eq!(resolve_langs("eng+hin"), "eng+hin");
    }

    #[test]
    fn parses_words_and_layout() {
        let words = parse_tsv(TSV);
        assert_eq!(words.len(), 4);
        assert_eq!(words[0].rect, [10.0, 10.0, 40.0, 12.0]);
        assert_eq!(join_words(&words), "Hello world\nline2\n\npara");
    }

    #[test]
    fn groups_paragraphs() {
        let paras = group(&parse_tsv(TSV), &|_, _| true);
        assert_eq!(paras.len(), 2);
        assert_eq!(paras[0].text, "Hello world line2");
        assert_eq!(paras[0].rect, [10.0, 10.0, 95.0, 32.0]);
        assert_eq!(paras[0].line_height, 16.0);
        assert_eq!(paras[1].text, "para");
    }

    fn para(y: f32, text: &str) -> Para {
        Para {
            rect: [10.0, y, 300.0, 20.0],
            text: text.into(),
            line_height: 20.0,
            words: 0..1,
            conf: 90.0,
        }
    }

    #[test]
    fn merges_split_sentence() {
        let out = merge_close(
            vec![
                para(0.0, "Please be quiet in the"),
                para(30.0, "reading room."),
            ],
            |_, _| true,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "Please be quiet in the reading room.");
        assert_eq!(out[0].rect, [10.0, 0.0, 300.0, 50.0]);
    }

    #[test]
    fn keeps_far_or_finished_paragraphs() {
        let out = merge_close(
            vec![para(0.0, "Title."), para(30.0, "Body text")],
            |_, _| true,
        );
        assert_eq!(out.len(), 2);
        let out = merge_close(vec![para(0.0, "One"), para(200.0, "Two")], |_, _| true);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn reads_fixture_text() {
        if which::which("tesseract").is_err() {
            eprintln!("tesseract missing; skipped");
            return;
        }
        let path = format!("{}/tests/fixtures/text.png", env!("CARGO_MANIFEST_DIR"));
        let img = image::open(path).unwrap().to_rgba8();
        let res = recognize(&img, "eng").unwrap();
        assert!(res.text.contains("quick brown fox"), "{}", res.text);
        assert!(res.text.contains("lazy dog 2026"), "{}", res.text);
        assert!(res.words.iter().all(|w| w.rect[0] < img.width() as f32));
    }

    #[test]
    fn lines_follow_tesseract_lines() {
        let words = parse_tsv(TSV);
        let paras = group(&words, &|_, _| true);
        let ls = lines(&words, &paras);
        assert_eq!(ls.len(), 3);
        assert_eq!(ls[0].words, 0..2);
        assert_eq!(ls[0].rect, [10.0, 10.0, 95.0, 12.0]);
        assert_eq!(ls[1].para, 0);
        assert_eq!(ls[2].para, 1);
    }

    #[test]
    fn split_keeps_order_and_balance() {
        let t = "please be silent in the reading room and turn off your phone";
        let parts = split_to_lines(t, &[300.0, 300.0]);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts.join(" "), t);
        let (a, b) = (parts[0].len() as i32, parts[1].len() as i32);
        assert!((a - b).abs() < 15, "{parts:?}");
    }

    #[test]
    fn split_gives_every_line_a_word() {
        let parts = split_to_lines("one two three", &[500.0, 10.0, 10.0]);
        assert_eq!(parts, vec!["one", "two", "three"]);
        let parts = split_to_lines("你好世界", &[100.0, 100.0]);
        assert_eq!(parts, vec!["你好", "世界"]);
    }

    #[test]
    fn merge_respects_predicate() {
        let out = merge_close(vec![para(0.0, "Please be"), para(30.0, "quiet")], |_, _| {
            false
        });
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn text_likeness() {
        assert!(para(0.0, "Hello world").is_text_like());
        assert!(!para(0.0, "#5223k @ 0/3").is_text_like());
        let mut p = para(0.0, "WWWWWF");
        p.conf = 20.0;
        assert!(!p.is_text_like());
    }

    #[test]
    fn binarize_makes_black_text_on_white() {
        // Dark image with a bright bar: the bar becomes black, the rest white.
        let g = image::GrayImage::from_fn(40, 20, |x, _| {
            image::Luma([if (15..25).contains(&x) { 230 } else { 20 }])
        });
        let b = binarize(&g);
        assert_eq!(b.get_pixel(20, 10)[0], 0);
        assert_eq!(b.get_pixel(2, 10)[0], 255);
        // Light image with a dark bar: the bar stays black.
        let g = image::GrayImage::from_fn(40, 20, |x, _| {
            image::Luma([if (10..30).contains(&x) { 30 } else { 240 }])
        });
        let b = binarize(&g);
        assert_eq!(b.get_pixel(20, 10)[0], 0);
        assert_eq!(b.get_pixel(2, 10)[0], 255);
    }

    fn w(x: f32, line: u32, text: &str) -> Word {
        Word {
            text: text.into(),
            rect: [x, 0.0, 30.0, 20.0],
            conf: 90.0,
            block: 1,
            par: 1,
            line,
        }
    }

    #[test]
    fn wide_gap_splits_line() {
        // "stray" far left, then a real line, then the next line.
        let words = vec![
            w(0.0, 1, "stray"),
            w(200.0, 1, "real"),
            w(240.0, 1, "text"),
            w(200.0, 2, "next"),
        ];
        let out = split_wide_gaps(words);
        assert_eq!(out[0].par, 1);
        assert_eq!(out[1].par, 10_001);
        assert_eq!(out[2].par, 10_001);
        // The split lasts only to the end of the Tesseract line.
        assert_eq!(out[3].par, 10_001);
    }
}
