//! Add system fonts as fallbacks so OCR and translated text in non-Latin
//! scripts renders. Uses fontconfig (`fc-list`), present on almost every
//! Linux desktop. Missing fontconfig only means fewer scripts render.

use eframe::egui;
use std::collections::HashSet;
use std::process::Command;

const LANGS: &[&str] = &[
    "hi", "bn", "ta", "te", "ml", "kn", "gu", "pa", "ar", "he", "th", "ru", "el", "ja", "zh-cn",
    "ko",
];
const MAX_FONT_BYTES: u64 = 40 * 1024 * 1024;

fn font_for(lang: &str) -> Option<(String, u32)> {
    let out = Command::new("fc-list")
        .arg(format!(":lang={lang}"))
        .args(["file", "index", "style"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines: Vec<&str> = text.lines().collect();
    // Prefer Noto Sans faces; they cover scripts well and look consistent.
    // Each line looks like: `/path/font.ttf: style=Regular:index=0`.
    lines.retain(|l| {
        let style = l.split("style=").nth(1).unwrap_or("");
        let first = style.split([',', ':']).next().unwrap_or("");
        first == "Regular" || first == "Book"
    });
    lines.sort_by_key(|l| {
        (
            !l.contains("NotoSans"),
            l.contains("Mono"),
            l.contains("UI"),
            l.len(),
        )
    });
    let line = lines.first()?;
    let (file, rest) = line.split_once(':')?;
    let index = rest
        .split("index=")
        .nth(1)
        .and_then(|i| i.split(':').next())
        .and_then(|i| i.trim().parse().ok())
        .unwrap_or(0);
    Some((file.trim().to_string(), index))
}

pub fn install(ctx: &egui::Context) {
    super::icons::install(ctx);
    let mut seen = HashSet::new();
    for lang in LANGS {
        let Some((file, index)) = font_for(lang) else {
            continue;
        };
        if !seen.insert(file.clone()) {
            continue;
        }
        let too_big = std::fs::metadata(&file)
            .map(|m| m.len() > MAX_FONT_BYTES)
            .unwrap_or(true);
        if too_big {
            continue;
        }
        let Ok(bytes) = std::fs::read(&file) else {
            continue;
        };
        tracing::debug!("fallback font for {lang}: {file}");
        let data = egui::FontData::from_blob(std::sync::Arc::new(bytes), index);
        let families = [egui::FontFamily::Proportional, egui::FontFamily::Monospace]
            .into_iter()
            .map(|family| egui::epaint::text::InsertFontFamily {
                family,
                priority: egui::epaint::text::FontPriority::Lowest,
            })
            .collect();
        ctx.add_font(egui::epaint::text::FontInsert::new(&file, data, families));
    }
}
