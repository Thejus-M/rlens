//! The rlens overlay: a frozen copy of the screen with a moving glow on the
//! edges. Drag to select an area; a result panel then opens on the same
//! screen with Search, Text and Translate tabs.

use super::icons;
use crate::{barcode, clipboard, config::Config, inpaint, lens_browser, net, ocr};
use eframe::egui::{
    self, Color32, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, epaint::Mesh, pos2,
    vec2,
};
use image::RgbaImage;
use std::sync::{
    Arc, Mutex,
    mpsc::{Receiver, Sender, channel},
};

const TARGET_LANGS: &[(&str, &str)] = &[
    ("en", "English"),
    ("hi", "Hindi"),
    ("bn", "Bengali"),
    ("ta", "Tamil"),
    ("te", "Telugu"),
    ("mr", "Marathi"),
    ("kn", "Kannada"),
    ("ml", "Malayalam"),
    ("gu", "Gujarati"),
    ("pa", "Punjabi"),
    ("ur", "Urdu"),
    ("es", "Spanish"),
    ("fr", "French"),
    ("de", "German"),
    ("it", "Italian"),
    ("pt", "Portuguese"),
    ("ru", "Russian"),
    ("ar", "Arabic"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("zh-CN", "Chinese (Simplified)"),
];

/// Glow colours, in order around the screen edge.
const GLOW: [Color32; 4] = [
    Color32::from_rgb(66, 133, 244),
    Color32::from_rgb(234, 67, 53),
    Color32::from_rgb(251, 188, 5),
    Color32::from_rgb(52, 168, 83),
];
const ACCENT: Color32 = Color32::from_rgb(138, 180, 248);
const ACCENT_TEXT: Color32 = Color32::from_rgb(32, 33, 36);
const PANEL_BG: Color32 = Color32::from_rgb(32, 33, 36);
const PANEL_W: f32 = 400.0;
const MIN_SEL: f32 = 12.0;

enum Msg {
    Ocr(u64, Result<ocr::OcrResult, String>),
    Codes(u64, Vec<barcode::Code>),
    /// (request key, paragraph index or None for one whole-text request, result)
    Translated(u64, Option<usize>, Result<String, String>),
    /// A message for the user (shown as a toast).
    Note(String),
    AiChunk(String),
    AiDone(Result<(), String>),
}

/// What the side panel shows.
#[derive(Clone, Copy, PartialEq)]
enum PanelKind {
    None,
    Translate,
    Ai,
}

#[derive(Clone, Copy, PartialEq)]
enum Icon {
    Copy,
    Text,
    Translate,
    Search,
    Link,
    Sparkle,
    Image,
    Qr,
}

/// One translated word painted on the image, in screen coordinates.
struct TWord {
    rect: Rect,
    text: String,
    galley: Arc<egui::Galley>,
}

/// One translated line, painted over its original line.
struct TLine {
    /// Soft shadow colour when the ink is close to the background.
    shadow: Option<Color32>,
    words: Vec<TWord>,
}

/// An OCR paragraph with the colours used to paint its translation.
struct ParaView {
    para: ocr::Para,
    bg: Color32,
    fg: Color32,
    /// The original text colour, used for the translation.
    ink: Color32,
    translated: Option<String>,
    /// Noise (icons, symbols, low confidence): not translated, no overlay.
    skip: bool,
}

enum Drag {
    None,
    /// Start corner of a new area, in frame pixels.
    Region(Pos2),
    /// Index of the first word of a text selection.
    Text(usize),
}

pub struct LensApp {
    cfg: Config,
    frame: RgbaImage,
    frame_tex: Option<egui::TextureHandle>,
    /// Quick mode: return the crop and close at once.
    quick: Option<Arc<Mutex<Option<RgbaImage>>>>,
    t0: Option<f64>,
    glow: f32,

    /// Selected area, in frame pixels.
    sel: Option<Rect>,
    drag: Drag,
    crop: Option<RgbaImage>,
    crop_tex: Option<egui::TextureHandle>,
    /// The crop with translated text erased, and the state it was built for.
    clean_tex: Option<egui::TextureHandle>,
    clean_key: (u64, usize),
    generation: u64,

    ocr: Option<ocr::OcrResult>,
    ocr_busy: bool,
    /// When the OCR result arrived (for the highlight fade-in).
    ocr_done_at: f64,
    lines: Vec<ocr::Line>,
    /// Per OCR word: true when it sits on a light background.
    word_light: Vec<bool>,
    /// `words_sel` indexes the translated words (true) or the OCR words.
    sel_on_translation: bool,
    /// Texts of the translated words painted in the last frame.
    tword_texts: Vec<String>,
    /// Debug builds: select and copy three translated words when ready.
    debug_select: bool,
    /// The selection is the whole screen set at start (no drag yet).
    full_default: bool,
    text: String,
    words_sel: Option<(usize, usize)>,
    codes: Vec<barcode::Code>,
    codes_busy: bool,

    panel: PanelKind,
    /// "T" mode: drag over words to select text instead of a new area.
    text_mode: bool,
    target: String,
    translated: String,
    translated_from: String,
    paras: Vec<ParaView>,
    /// Paint translations over the original text.
    show_translation: bool,
    translate_key: u64,
    translate_pending: usize,
    translate_busy: bool,
    ai_prompt: String,
    ai_out: String,
    ai_busy: bool,

    toast: Option<(String, f64)>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl LensApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        cfg: Config,
        frame: RgbaImage,
        quick: Option<Arc<Mutex<Option<RgbaImage>>>>,
        area: Option<[f32; 4]>,
    ) -> Self {
        super::fonts::install(&cc.egui_ctx);
        apply_style(&cc.egui_ctx);
        let (tx, rx) = channel();
        // Whole screen selected at start (not in quick modes, not with --area).
        let full_default = area.is_none() && quick.is_none() && cfg.ui.select_all;
        let sel = match area {
            Some([x, y, w, h]) => Some(Rect::from_min_size(pos2(x, y), vec2(w, h))),
            None if full_default => Some(Rect::from_min_size(
                Pos2::ZERO,
                vec2(frame.width() as f32, frame.height() as f32),
            )),
            None => None,
        };
        Self {
            full_default,
            target: cfg.translate.target.clone(),
            ai_prompt: cfg.ollama.prompt.clone(),
            cfg,
            frame,
            frame_tex: None,
            quick,
            t0: None,
            glow: 0.0,
            sel,
            drag: Drag::None,
            crop: None,
            crop_tex: None,
            clean_tex: None,
            clean_key: (0, 0),
            generation: 0,
            ocr: None,
            ocr_busy: false,
            ocr_done_at: 0.0,
            lines: Vec::new(),
            word_light: Vec::new(),
            sel_on_translation: false,
            tword_texts: Vec::new(),
            debug_select: false,
            text: String::new(),
            words_sel: None,
            codes: Vec::new(),
            codes_busy: false,
            panel: PanelKind::None,
            text_mode: false,
            translated: String::new(),
            translated_from: String::new(),
            paras: Vec::new(),
            show_translation: false,
            translate_key: 0,
            translate_pending: 0,
            translate_busy: false,
            ai_out: String::new(),
            ai_busy: false,
            toast: None,
            tx,
            rx,
        }
    }

    // ------------------------------------------------------------ actions

    fn toast(&mut self, ctx: &egui::Context, msg: impl Into<String>) {
        self.toast = Some((msg.into(), ctx.input(|i| i.time)));
    }

    fn clear_results(&mut self) {
        self.generation += 1;
        self.crop = None;
        self.crop_tex = None;
        self.ocr = None;
        self.text.clear();
        self.words_sel = None;
        self.codes.clear();
        self.translated.clear();
        self.translated_from.clear();
        self.paras.clear();
        self.lines.clear();
        self.word_light.clear();
        self.tword_texts.clear();
        self.sel_on_translation = false;
        self.show_translation = false;
        self.translate_key += 1;
        self.translate_busy = false;
        self.ai_out.clear();
        self.ocr_busy = false;
        self.codes_busy = false;
        self.panel = PanelKind::None;
        self.text_mode = false;
    }

    /// Crop the frame to the selection and start OCR and code detection.
    fn analyze(&mut self, ctx: &egui::Context) {
        let Some(sel) = self.sel else { return };
        self.clear_results();
        let x = sel.min.x.max(0.0) as u32;
        let y = sel.min.y.max(0.0) as u32;
        let w = (sel.width().round() as u32).min(self.frame.width().saturating_sub(x));
        let h = (sel.height().round() as u32).min(self.frame.height().saturating_sub(y));
        let crop = image::imageops::crop_imm(&self.frame, x, y, w, h).to_image();
        tracing::info!("selection: {w}x{h} px at {x},{y}");

        if let Some(out) = &self.quick {
            *out.lock().unwrap() = Some(crop);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let generation = self.generation;
        self.ocr_busy = true;
        self.codes_busy = true;
        let (img, langs, tx, c) = (
            crop.clone(),
            self.cfg.ocr.langs.clone(),
            self.tx.clone(),
            ctx.clone(),
        );
        std::thread::spawn(move || {
            let r = ocr::recognize(&img, &langs).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Ocr(generation, r));
            c.request_repaint();
        });
        let (img, tx, c) = (crop.clone(), self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Codes(generation, barcode::detect(&img)));
            c.request_repaint();
        });
        self.crop = Some(crop);
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Ocr(g, r) if g == self.generation => {
                    self.ocr_busy = false;
                    match r {
                        Ok(res) => {
                            self.text = res.text.clone();
                            self.paras = self.build_paras(&res);
                            let paras: Vec<ocr::Para> =
                                self.paras.iter().map(|p| p.para.clone()).collect();
                            self.lines = ocr::lines(&res.words, &paras);
                            self.word_light = (0..res.words.len())
                                .map(|i| {
                                    self.paras
                                        .iter()
                                        .find(|p| p.para.words.contains(&i))
                                        .is_some_and(|p| p.fg.r() < 128)
                                })
                                .collect();
                            self.ocr_done_at = ctx.input(|i| i.time);
                            self.ocr = Some(res);
                            if self.panel == PanelKind::Translate {
                                self.start_translate(ctx);
                            }
                        }
                        Err(e) => self.toast(ctx, format!("OCR: {e}")),
                    }
                }
                Msg::Codes(g, codes) if g == self.generation => {
                    self.codes_busy = false;
                    self.codes = codes;
                }
                Msg::Ocr(..) | Msg::Codes(..) => {}
                Msg::Translated(key, idx, r) if key == self.translate_key => {
                    self.translate_pending = self.translate_pending.saturating_sub(1);
                    self.translate_busy = self.translate_pending > 0;
                    match (idx, r) {
                        (None, Ok(t)) => self.translated = t,
                        (Some(i), Ok(t)) => {
                            if let Some(p) = self.paras.get_mut(i) {
                                p.translated = Some(t);
                            }
                            self.translated = self
                                .paras
                                .iter()
                                .filter_map(|p| p.translated.as_deref())
                                .collect::<Vec<_>>()
                                .join("\n\n");
                        }
                        (_, Err(e)) => {
                            tracing::warn!("translate: {e}");
                            self.toast(ctx, format!("Translate: {e}"))
                        }
                    }
                }
                Msg::Translated(..) => {}
                Msg::Note(n) => self.toast(ctx, n),
                Msg::AiChunk(s) => self.ai_out.push_str(&s),
                Msg::AiDone(r) => {
                    self.ai_busy = false;
                    if let Err(e) = r {
                        self.toast(ctx, format!("Local AI: {e}"));
                    }
                }
            }
        }
    }

    /// Text to copy: the selected words (original or translated), else the
    /// translation while it shows, else all OCR text.
    fn chosen_text(&self) -> String {
        if self.sel_on_translation {
            if let Some((a, b)) = self.words_sel {
                let b = b.min(self.tword_texts.len().saturating_sub(1));
                if a <= b && !self.tword_texts.is_empty() {
                    return self.tword_texts[a..=b].join(" ");
                }
            }
            if !self.translated.is_empty() {
                return self.translated.clone();
            }
        }
        self.original_text()
    }

    /// The selected OCR words, or all OCR text.
    fn original_text(&self) -> String {
        if self.sel_on_translation {
            return self.text.clone();
        }
        if let (Some(res), Some((a, b))) = (&self.ocr, self.words_sel) {
            let b = b.min(res.words.len().saturating_sub(1));
            if a <= b {
                return ocr::join_words(&res.words[a..=b]);
            }
        }
        self.text.clone()
    }

    fn copy_text(&mut self, ctx: &egui::Context) {
        let t = self.chosen_text();
        if t.trim().is_empty() {
            self.toast(ctx, "No text found");
            return;
        }
        match clipboard::set_text(&t) {
            Ok(()) => self.toast(ctx, "Text copied"),
            Err(e) => self.toast(ctx, format!("Copy failed: {e:#}")),
        }
    }

    fn copy_image(&mut self, ctx: &egui::Context) {
        let Some(img) = &self.crop else { return };
        let msg = match clipboard::set_image(img) {
            Ok(()) => format!("Image copied ({}×{} px)", img.width(), img.height()),
            Err(e) => format!("Copy failed: {e:#}"),
        };
        self.toast(ctx, msg);
    }

    /// Side of the screen that does not hold the selection.
    fn free_side(&self, screen: Rect) -> lens_browser::Side {
        match self.sel_screen(screen) {
            Some(r) if r.center().x > screen.center().x => lens_browser::Side::Left,
            _ => lens_browser::Side::Right,
        }
    }

    fn start_lens(&mut self, ctx: &egui::Context, screen: Rect) {
        let Some(img) = &self.crop else { return };
        let side = self.free_side(screen);
        match lens_browser::open(img, side, &self.cfg.lens.hl, &self.cfg.lens.browser) {
            Ok(_) => self.toast(ctx, "Opening Google Lens…"),
            Err(e) => self.toast(ctx, format!("Google Lens: {e:#}")),
        }
    }

    /// Paragraphs of the OCR result, with background and text colours
    /// sampled from the crop.
    fn build_paras(&self, res: &ocr::OcrResult) -> Vec<ParaView> {
        let Some(crop) = &self.crop else {
            return Vec::new();
        };
        // Merge split paragraphs only over the same background, so text in a
        // different box (a caption, a toolbar) stays separate.
        let same_bg = |a: &ocr::Para, b: &ocr::Para| {
            let (x, y) = (ring_median(crop, a.rect), ring_median(crop, b.rect));
            let d = |p: u8, q: u8| (p as i32 - q as i32).abs();
            d(x.r(), y.r()) + d(x.g(), y.g()) + d(x.b(), y.b()) < 60
        };
        ocr::paragraphs_where(&res.words, same_bg)
            .into_iter()
            .map(|para| {
                let bg = ring_median(crop, para.rect);
                let lum = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
                let fg = if lum > 140.0 {
                    Color32::from_gray(20)
                } else {
                    Color32::from_gray(245)
                };
                let ink = inpaint::text_color(crop, para.rect, [bg.r(), bg.g(), bg.b()])
                    .map(|[r, g, b]| Color32::from_rgb(r, g, b))
                    .unwrap_or(fg);
                ParaView {
                    skip: !para.is_text_like(),
                    para,
                    bg,
                    fg,
                    ink,
                    translated: None,
                }
            })
            .collect()
    }

    /// Rebuild the erased-text texture when new translations arrive.
    fn ensure_clean(&mut self, ctx: &egui::Context) {
        let done = self.paras.iter().filter(|p| p.translated.is_some()).count();
        let key = (self.translate_key, done);
        if done == 0 || key == self.clean_key {
            return;
        }
        let Some(crop) = &self.crop else { return };
        let mut img = crop.clone();
        for (pi, p) in self.paras.iter().enumerate() {
            if p.translated.is_none() {
                continue;
            }
            let bg = [p.bg.r(), p.bg.g(), p.bg.b()];
            for line in self.lines.iter().filter(|l| l.para == pi) {
                inpaint::erase_text(&mut img, line.rect, bg);
            }
        }
        let size = [img.width() as usize, img.height() as usize];
        let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
        self.clean_tex = Some(ctx.load_texture("clean", color, egui::TextureOptions::LINEAR));
        self.clean_key = key;
    }

    /// Translate the text. Without a word selection, each paragraph is
    /// translated on its own so it can be painted over the original.
    fn start_translate(&mut self, ctx: &egui::Context) {
        let text = self.text.clone();
        if text.trim().is_empty() {
            return;
        }
        self.translate_key += 1;
        let key = self.translate_key;
        self.translate_busy = true;
        self.translated.clear();
        self.translated_from = format!("{text}\u{0}{}", self.target);
        for p in &mut self.paras {
            p.translated = None;
        }
        if self.sel_on_translation {
            self.words_sel = None;
        }
        // The translation always goes over the text, paragraph by paragraph;
        // a word selection does not change what is translated.
        self.words_sel = None;
        let jobs: Vec<(Option<usize>, String)> = if self.paras.is_empty() {
            self.show_translation = false;
            vec![(None, text)]
        } else {
            self.show_translation = true;
            self.paras
                .iter()
                .enumerate()
                .filter(|(_, p)| !p.skip)
                .take(60)
                .map(|(i, p)| (Some(i), p.para.text.clone()))
                .collect()
        };
        if jobs.is_empty() {
            tracing::info!(
                "translate: all {} paragraphs look like noise: {:?}",
                self.paras.len(),
                self.paras
                    .iter()
                    .map(|p| (p.para.conf as i32, p.para.text.clone()))
                    .collect::<Vec<_>>()
            );
            self.translate_busy = false;
            self.show_translation = false;
            let hint = if crate::ocr::installed_langs()
                .iter()
                .all(|l| l == "eng" || l == "osd")
            {
                "No text found. For other scripts, install a Tesseract language pack (e.g. tesseract-data-tam)."
            } else {
                "No text to translate"
            };
            self.toast(ctx, hint);
            return;
        }
        tracing::info!(
            "translate: {} of {} paragraphs to {}",
            jobs.len(),
            self.paras.len(),
            self.target
        );
        self.translate_pending = jobs.len();
        // One batched request for all paragraphs: many parallel requests
        // make Google rate-limit the network.
        let (target, cfg) = (self.target.clone(), self.cfg.clone());
        let (tx, c) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let texts: Vec<String> = jobs.iter().map(|(_, t)| t.clone()).collect();
            match net::translate_batch(&texts, &target, &cfg) {
                Ok((out, note)) => {
                    if let Some(n) = note {
                        let _ = tx.send(Msg::Note(n));
                    }
                    for ((idx, _), t) in jobs.into_iter().zip(out) {
                        let _ = tx.send(Msg::Translated(key, idx, Ok(t)));
                    }
                }
                Err(e) => {
                    let e = format!("{e:#}");
                    for (idx, _) in jobs {
                        let _ = tx.send(Msg::Translated(key, idx, Err(e.clone())));
                    }
                }
            }
            c.request_repaint();
        });
    }

    /// Lay out each translated paragraph over its original lines: the
    /// words are spread over the lines in proportion to their widths, and
    /// each line is drawn at its original position and height.
    fn translation_layout(&self, painter: &egui::Painter, fr: Rect) -> Vec<TLine> {
        let Some(sel) = self.sel else {
            return Vec::new();
        };
        let s = fr.width() / self.frame.width() as f32;
        let to_screen = |r: [f32; 4]| {
            Rect::from_min_size(
                fr.min + (sel.min.to_vec2() + vec2(r[0], r[1])) * s,
                vec2(r[2], r[3]) * s,
            )
        };
        let mut out = Vec::new();
        for (pi, p) in self.paras.iter().enumerate() {
            let Some(text) = &p.translated else { continue };
            let lines: Vec<&ocr::Line> = self.lines.iter().filter(|l| l.para == pi).collect();
            if lines.is_empty() {
                continue;
            }
            let widths: Vec<f32> = lines.iter().map(|l| l.rect[2]).collect();
            let parts = ocr::split_to_lines(text, &widths);
            let spaced = text.split_whitespace().count() > 1;
            for (line, part) in lines.iter().zip(parts) {
                let r = to_screen(line.rect);
                let font = |size: f32| egui::FontId::proportional(size);
                let mut size = (r.height() * 0.85).clamp(8.0, 96.0);
                let full = painter
                    .layout_no_wrap(part.clone(), font(size), p.ink)
                    .size()
                    .x;
                if full > r.width() && full > 0.0 {
                    size = (size * r.width() / full).max((r.height() * 0.5).max(8.0));
                }
                let gap = if spaced { size * 0.28 } else { 0.0 };
                let tokens: Vec<String> = if spaced {
                    part.split_whitespace().map(str::to_string).collect()
                } else {
                    part.chars().map(|c| c.to_string()).collect()
                };
                let mut x = r.min.x;
                let mut words = Vec::new();
                for t in tokens {
                    let g = painter.layout_no_wrap(t.clone(), font(size), p.ink);
                    let rect =
                        Rect::from_min_size(pos2(x, r.center().y - g.size().y / 2.0), g.size());
                    x += g.size().x + gap;
                    words.push(TWord {
                        rect,
                        text: t,
                        galley: g,
                    });
                }
                let lum =
                    |c: Color32| 0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32;
                let shadow = ((lum(p.ink) - lum(p.bg)).abs() < 90.0).then(|| {
                    if lum(p.ink) > 128.0 {
                        Color32::from_black_alpha(150)
                    } else {
                        Color32::from_white_alpha(150)
                    }
                });
                out.push(TLine { shadow, words });
            }
        }
        out
    }

    fn start_ai(&mut self, ctx: &egui::Context) {
        let Some(img) = self.crop.clone() else { return };
        self.ai_busy = true;
        self.ai_out.clear();
        let (url, model) = (
            self.cfg.ollama.url.clone(),
            self.cfg.ollama.vision_model.clone(),
        );
        let prompt = self.ai_prompt.clone();
        let (tx, c) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let (tx2, c2) = (tx.clone(), c.clone());
            let r = net::ollama_vision(&img, &url, &model, &prompt, |chunk| {
                let _ = tx2.send(Msg::AiChunk(chunk.to_string()));
                c2.request_repaint();
            })
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::AiDone(r));
            c.request_repaint();
        });
    }

    // ------------------------------------------------------------ geometry

    /// Where the frame is drawn: fitted and centred in the screen.
    fn frame_rect(&self, screen: Rect) -> Rect {
        let (fw, fh) = (self.frame.width() as f32, self.frame.height() as f32);
        let scale = (screen.width() / fw).min(screen.height() / fh);
        Rect::from_center_size(screen.center(), vec2(fw * scale, fh * scale))
    }

    /// The selection in screen coordinates.
    fn sel_screen(&self, screen: Rect) -> Option<Rect> {
        let sel = self.sel?;
        let fr = self.frame_rect(screen);
        let s = fr.width() / self.frame.width() as f32;
        Some(Rect::from_min_max(
            fr.min + sel.min.to_vec2() * s,
            fr.min + sel.max.to_vec2() * s,
        ))
    }

    /// Screen rectangles of the OCR words.
    fn word_rects(&self, fr: Rect) -> Vec<Rect> {
        let (Some(res), Some(sel)) = (&self.ocr, self.sel) else {
            return Vec::new();
        };
        let s = fr.width() / self.frame.width() as f32;
        res.words
            .iter()
            .map(|w| {
                Rect::from_min_size(
                    fr.min + (sel.min.to_vec2() + vec2(w.rect[0], w.rect[1])) * s,
                    vec2(w.rect[2], w.rect[3]) * s,
                )
            })
            .collect()
    }

    // ------------------------------------------------------------ drawing

    fn paint_scene(
        &mut self,
        ui: &mut egui::Ui,
        screen: Rect,
        now: f64,
        tlines: &[TLine],
        words: &[Rect],
    ) {
        let ctx = ui.ctx().clone();
        let painter = ui.painter().clone();
        let fr = self.frame_rect(screen);
        let tex = self.frame_tex.get_or_insert_with(|| {
            let size = [self.frame.width() as usize, self.frame.height() as usize];
            let img = egui::ColorImage::from_rgba_unmultiplied(size, self.frame.as_raw());
            ctx.load_texture("frame", img, egui::TextureOptions::LINEAR)
        });
        let tex_id = tex.id();
        painter.rect_filled(screen, 0.0, Color32::BLACK);
        painter.image(
            tex.id(),
            fr,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );

        match self.sel_screen(screen) {
            None => {
                painter.rect_filled(screen, 0.0, Color32::from_black_alpha(70));
            }
            Some(r) => {
                let full = r.width() >= fr.width() - 1.0 && r.height() >= fr.height() - 1.0;
                for o in outside(screen, r) {
                    painter.rect_filled(o, 0.0, Color32::from_black_alpha(150));
                }
                if self.ocr_busy {
                    shimmer(&painter, r, now);
                }
                let sel_words = self.words_sel;
                let picked = |i: usize| sel_words.is_some_and(|(a, b)| i >= a && i <= b);
                let blue = Color32::from_rgba_unmultiplied(66, 133, 244, 120);
                if !tlines.is_empty() {
                    // The selection with its translated text erased, then the
                    // translation drawn in the original ink colour.
                    if let Some(clean) = &self.clean_tex {
                        painter.image(
                            clean.id(),
                            r,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    let mut k = 0;
                    for line in tlines {
                        for w in &line.words {
                            if picked(k) {
                                painter.rect_filled(w.rect.expand(1.5), 3.0, blue);
                            }
                            if let Some(sh) = line.shadow {
                                painter.galley_with_override_text_color(
                                    w.rect.min + vec2(0.0, 1.0),
                                    w.galley.clone(),
                                    sh,
                                );
                            }
                            painter.galley(w.rect.min, w.galley.clone(), Color32::WHITE);
                            k += 1;
                        }
                    }
                } else if !words.is_empty() {
                    // Text focus (T): dim the area, then redraw the words sharp.
                    if self.text_mode {
                        painter.rect_filled(r, 0.0, Color32::from_black_alpha(120));
                        for wr in words {
                            let a = wr.expand(2.0);
                            let uv = Rect::from_min_max(
                                pos2(
                                    (a.min.x - fr.min.x) / fr.width(),
                                    (a.min.y - fr.min.y) / fr.height(),
                                ),
                                pos2(
                                    (a.max.x - fr.min.x) / fr.width(),
                                    (a.max.y - fr.min.y) / fr.height(),
                                ),
                            );
                            painter.image(tex_id, a, uv, Color32::WHITE);
                        }
                    }
                    // Soft highlight on every word; selected words in blue.
                    let fade = ((now - self.ocr_done_at) / 0.3).clamp(0.0, 1.0) as f32;
                    let base = if self.text_mode { 70.0 } else { 42.0 };
                    let soft_dark = Color32::from_white_alpha((base * 1.3 * fade) as u8);
                    let soft_light = Color32::from_black_alpha((base * 1.1 * fade) as u8);
                    for (i, wr) in words.iter().enumerate() {
                        let light = self.word_light.get(i).copied().unwrap_or(false);
                        let soft = if light { soft_light } else { soft_dark };
                        let fill = if picked(i) { blue } else { soft };
                        painter.rect_filled(wr.expand(2.0), 3.0, fill);
                    }
                }
                if full {
                    brackets(&painter, r.shrink(14.0));
                } else {
                    brackets(&painter, r.expand(4.0));
                }
            }
        }
        glow(&painter, screen, now, self.glow);

        if self.full_default {
            let age = now - self.t0.unwrap_or(now);
            let alpha = (age / 0.4).min(1.0).min((4.5 - age) / 0.5).clamp(0.0, 1.0) as f32;
            pill(
                &painter,
                screen.center_top() + vec2(0.0, 48.0),
                "Drag to select a smaller area  ·  Esc to close",
                alpha,
            );
        }
        if self.sel.is_none() {
            let alpha = ((now - self.t0.unwrap_or(now)) / 0.4).clamp(0.0, 1.0) as f32;
            pill(
                &painter,
                screen.center_top() + vec2(0.0, 48.0),
                "Select anything to search  ·  Esc to close",
                alpha,
            );
        }
        if let Some((msg, t)) = &self.toast {
            let age = now - t;
            if age < 3.5 {
                let a = (1.0 - ((age - 3.0) / 0.5).clamp(0.0, 1.0)) as f32;
                pill(&painter, screen.center_top() + vec2(0.0, 104.0), msg, a);
            }
        }
    }

    fn handle_input(&mut self, ui: &mut egui::Ui, screen: Rect, words: &[Rect]) {
        let ctx = ui.ctx().clone();
        let fr = self.frame_rect(screen);
        let s = fr.width() / self.frame.width() as f32;
        let (fw, fh) = (self.frame.width() as f32, self.frame.height() as f32);
        let to_frame = |p: Pos2| {
            let v = (p - fr.min) / s;
            pos2(v.x.clamp(0.0, fw), v.y.clamp(0.0, fh))
        };
        let resp = ui.interact(screen, ui.scope_id().with("scene"), Sense::click_and_drag());
        let word_at = |p: Pos2| words.iter().position(|r| r.expand(3.0).contains(p));

        if let Some(h) = resp.hover_pos() {
            let icon = if word_at(h).is_some() {
                egui::CursorIcon::Text
            } else {
                egui::CursorIcon::Crosshair
            };
            ctx.set_cursor_icon(icon);
        }

        if resp.drag_started()
            && let Some(p) = resp.interact_pointer_pos()
        {
            self.drag = match word_at(p) {
                Some(i) => {
                    self.words_sel = Some((i, i));
                    Drag::Text(i)
                }
                None => Drag::Region(to_frame(p)),
            };
        }
        if resp.dragged()
            && let Some(p) = resp.interact_pointer_pos()
        {
            match self.drag {
                Drag::Region(start) => {
                    self.full_default = false;
                    let r = Rect::from_two_pos(start, to_frame(p));
                    if r.width() * s > 4.0 || r.height() * s > 4.0 {
                        if self.crop.is_some() || self.ocr_busy {
                            self.clear_results();
                        }
                        self.sel = Some(r);
                    }
                }
                Drag::Text(anchor) => {
                    if let Some(i) = word_at(p) {
                        self.words_sel = Some((anchor.min(i), anchor.max(i)));
                    }
                }
                Drag::None => {}
            }
        }
        if resp.drag_stopped() {
            if let Drag::Region(_) = self.drag {
                match self.sel {
                    Some(r) if r.width() * s >= MIN_SEL && r.height() * s >= MIN_SEL => {
                        self.analyze(&ctx)
                    }
                    _ => {
                        self.sel = None;
                        self.clear_results();
                    }
                }
            }
            self.drag = Drag::None;
        }
        if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            if let Some(i) = word_at(p) {
                self.words_sel = Some((i, i));
            } else if self.words_sel.is_some() {
                self.words_sel = None;
            } else if self.sel.is_some() && !self.full_default {
                self.sel = None;
                self.clear_results();
            }
        }
    }

    /// The action bar under the selection.
    fn chips(&mut self, ctx: &egui::Context, screen: Rect, avoid: Option<Rect>) {
        let Some(r) = self.sel_screen(screen) else {
            return;
        };
        if self.crop.is_none() {
            return;
        }
        let has_text = !self.text.trim().is_empty();
        let code = self.codes.first().cloned();

        // Estimate the bar width to place it before drawing.
        let mut width = 2.0 * 52.0 + 64.0;
        if has_text || self.ocr_busy {
            width += 3.0 * 52.0;
        }
        if code.is_some() {
            width += 52.0;
        }
        if self.cfg.ollama.enabled {
            width += 52.0;
        }
        let mut pos = pos2(r.center().x - width / 2.0, r.max.y + 18.0);
        if pos.y + 60.0 > screen.max.y {
            pos.y = r.min.y - 72.0;
            if pos.y < screen.min.y + 8.0 {
                // No room above or below: inside the selection, near the bottom.
                pos.y = (r.max.y - 76.0).min(screen.max.y - 76.0);
            }
        }
        pos.x = pos.x.clamp(screen.min.x + 8.0, screen.max.x - width - 8.0);
        if let Some(avoid) = avoid
            && avoid.intersects(Rect::from_min_size(pos, vec2(width, 56.0)))
        {
            pos.x = if avoid.min.x > screen.center().x {
                (avoid.min.x - width - 12.0).max(screen.min.x + 8.0)
            } else {
                avoid.max.x + 12.0
            };
        }

        let mut action = None;
        egui::Area::new(egui::Id::unique("rlens-chips"))
            .fixed_pos(pos)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(PANEL_BG)
                    .corner_radius(CornerRadius::same(28))
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .shadow(shadow())
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.horizontal(|ui| {
                            if has_text || self.ocr_busy {
                                let on = has_text;
                                let tip = if self.words_sel.is_some() {
                                    "Copy selected text"
                                } else {
                                    "Copy text"
                                };
                                if icon_button(ui, Icon::Copy, tip, false, on).clicked() {
                                    action = Some(Icon::Copy);
                                }
                                if icon_button(ui, Icon::Text, "Select text", self.text_mode, on)
                                    .clicked()
                                {
                                    action = Some(Icon::Text);
                                }
                                let active = self.panel == PanelKind::Translate;
                                if icon_button(ui, Icon::Translate, "Translate", active, on)
                                    .clicked()
                                {
                                    action = Some(Icon::Translate);
                                }
                            }
                            if let Some(c) = &code {
                                let (icon, tip) = if c.is_url() {
                                    (Icon::Link, format!("Open {}", c.text.trim()))
                                } else {
                                    (Icon::Qr, format!("Copy {}: {}", c.format, c.text))
                                };
                                if icon_button(ui, icon, &tip, false, true).clicked() {
                                    action = Some(Icon::Link);
                                }
                            }
                            if self.cfg.ollama.enabled
                                && icon_button(
                                    ui,
                                    Icon::Sparkle,
                                    "Ask local AI",
                                    self.panel == PanelKind::Ai,
                                    true,
                                )
                                .clicked()
                            {
                                action = Some(Icon::Sparkle);
                            }
                            if icon_button(ui, Icon::Image, "Copy image", false, true).clicked() {
                                action = Some(Icon::Image);
                            }
                            // Thin divider before the Search pill.
                            let (div, _) = ui.allocate_exact_size(vec2(9.0, 26.0), Sense::hover());
                            ui.painter().line_segment(
                                [div.center_top(), div.center_bottom()],
                                Stroke::new(1.0, Color32::from_gray(70)),
                            );
                            if self.cfg.lens.enabled
                                && search_button(ui)
                                    .on_hover_text("Search with Google Lens")
                                    .clicked()
                            {
                                action = Some(Icon::Search);
                            }
                        });
                    });
            });

        match action {
            Some(Icon::Copy) => self.copy_text(ctx),
            Some(Icon::Text) => {
                self.text_mode = !self.text_mode;
                if self.text_mode {
                    self.toast(ctx, "Drag across the words to select text");
                } else {
                    self.words_sel = None;
                }
            }
            Some(Icon::Translate) => {
                if self.panel == PanelKind::Translate {
                    self.panel = PanelKind::None;
                    self.show_translation = false;
                } else {
                    self.panel = PanelKind::Translate;
                    let key = format!("{}\u{0}{}", self.text, self.target);
                    if self.translated_from != key {
                        self.start_translate(ctx);
                    } else if self.paras.iter().any(|p| p.translated.is_some()) {
                        // Same text and language: show the overlay again.
                        self.show_translation = true;
                    }
                }
            }
            Some(Icon::Link) => {
                if let Some(c) = code {
                    if c.is_url() {
                        match net::open_url(c.text.trim()) {
                            Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                            Err(e) => self.toast(ctx, format!("Open link: {e:#}")),
                        }
                    } else {
                        let msg = match clipboard::set_text(&c.text) {
                            Ok(()) => format!("{} copied", c.format),
                            Err(e) => format!("Copy failed: {e:#}"),
                        };
                        self.toast(ctx, msg);
                    }
                }
            }
            Some(Icon::Sparkle) => {
                self.panel = if self.panel == PanelKind::Ai {
                    PanelKind::None
                } else {
                    PanelKind::Ai
                };
            }
            Some(Icon::Search) => self.start_lens(ctx, screen),
            Some(Icon::Image) => self.copy_image(ctx),
            Some(Icon::Qr) | None => {}
        }
    }

    /// The side panel goes on the side away from the selection.
    fn panel_rect(&self, screen: Rect) -> Rect {
        let h = (screen.height() - 72.0).min(560.0);
        let x = match self.free_side(screen) {
            lens_browser::Side::Right => screen.max.x - PANEL_W - 24.0,
            lens_browser::Side::Left => screen.min.x + 24.0,
        };
        // 48 px from the top keeps the panel clear of a desktop bar.
        Rect::from_min_size(pos2(x, screen.min.y + 48.0), vec2(PANEL_W, h))
    }

    fn panel(&mut self, ctx: &egui::Context, rect: Rect) {
        egui::Area::new(egui::Id::unique("rlens-panel"))
            .fixed_pos(rect.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(PANEL_BG)
                    .corner_radius(CornerRadius::same(20))
                    .inner_margin(18.0)
                    .shadow(shadow())
                    .show(ui, |ui| {
                        ui.set_width(rect.width() - 36.0);
                        ui.set_max_height(rect.height() - 36.0);
                        self.panel_contents(ui);
                    });
            });
    }

    fn panel_contents(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let title = match self.panel {
            PanelKind::Translate => "Translate",
            PanelKind::Ai => "Ask local AI",
            PanelKind::None => "",
        };
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(title)
                    .size(19.0)
                    .strong()
                    .color(Color32::WHITE),
            );
            if self.translate_busy || self.ai_busy {
                ui.spinner();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(icons::X).font(icons::font(18.0, false)),
                        )
                        .frame(false),
                    )
                    .on_hover_text("Close panel")
                    .clicked()
                {
                    self.panel = PanelKind::None;
                    self.show_translation = false;
                }
            });
        });
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .show(ui, |ui| match self.panel {
                PanelKind::Translate => self.tab_translate(ui),
                PanelKind::Ai => self.tab_ai(ui, &ctx),
                PanelKind::None => {}
            });
    }

    fn tab_ai(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add(
            egui::TextEdit::multiline(&mut self.ai_prompt)
                .desired_width(f32::INFINITY)
                .desired_rows(2)
                .hint_text("Ask about this image"),
        );
        if ui.add_enabled(!self.ai_busy, big_button("Ask")).clicked() {
            self.start_ai(ctx);
        }
        if !self.ai_out.is_empty() {
            ui.add_space(8.0);
            ui.add(egui::Label::new(egui::RichText::new(&self.ai_out).size(15.0)).wrap());
        }
    }

    fn tab_translate(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.label("To");
            let name = TARGET_LANGS
                .iter()
                .find(|(c, _)| *c == self.target)
                .map(|(_, n)| *n)
                .unwrap_or(self.target.as_str())
                .to_string();
            let before = self.target.clone();
            egui::ComboBox::from_id_salt("target")
                .selected_text(name)
                .width(200.0)
                .show_ui(ui, |ui| {
                    for (code, label) in TARGET_LANGS {
                        ui.selectable_value(&mut self.target, code.to_string(), *label);
                    }
                });
            if before != self.target {
                self.start_translate(&ctx);
            }
            if self.translate_busy {
                ui.spinner();
            }
        });
        ui.add_space(8.0);
        if self.chosen_text().trim().is_empty() {
            ui.weak(if self.ocr_busy {
                "Reading text…"
            } else {
                "No text to translate"
            });
            return;
        }
        if self.translated.is_empty() && !self.translate_busy {
            if ui.add(big_button("Translate")).clicked() {
                self.start_translate(&ctx);
            }
            return;
        }
        if !self.translated.is_empty() {
            egui::Frame::new()
                .fill(Color32::from_gray(44))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        egui::Label::new(egui::RichText::new(&self.translated).size(16.0)).wrap(),
                    );
                });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.add(big_button("Copy translation")).clicked() {
                    let msg = match clipboard::set_text(&self.translated) {
                        Ok(()) => "Translation copied".to_string(),
                        Err(e) => format!("Copy failed: {e:#}"),
                    };
                    self.toast(&ctx, msg);
                }
                let on_image = self.paras.iter().any(|p| p.translated.is_some());
                if on_image {
                    let label = if self.show_translation {
                        "Show original"
                    } else {
                        "Show translation"
                    };
                    if ui.button(label).clicked() {
                        self.show_translation = !self.show_translation;
                    }
                }
            });
            if self.show_translation {
                ui.label(
                    egui::RichText::new("Hold Space to see the original text.")
                        .small()
                        .weak(),
                );
            }
        }
    }
}

impl eframe::App for LensApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        let first = self.t0.is_none();
        let t0 = *self.t0.get_or_insert(now);
        if first && self.sel.is_some() {
            self.analyze(&ctx);
            // Debug builds: RLENS_DEBUG_ACTION=search|translate|copyimage presses a chip.
            #[cfg(debug_assertions)]
            match std::env::var("RLENS_DEBUG_ACTION").as_deref() {
                Ok("search") => self.start_lens(&ctx, ui.max_rect()),
                Ok("translate") => {
                    self.panel = PanelKind::Translate;
                    if let Ok(t) = std::env::var("RLENS_DEBUG_TARGET") {
                        self.target = t;
                    }
                }
                Ok("copyimage") => self.copy_image(&ctx),
                Ok("translate-select") => {
                    self.panel = PanelKind::Translate;
                    self.debug_select = true;
                }
                _ => {}
            }
        }
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        self.poll(&ctx);

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if !ctx.egui_wants_keyboard_input()
            && self.crop.is_some()
            && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::C))
        {
            self.copy_text(&ctx);
        }

        // Glow: fade in at start, fade out once an area is selected.
        let scanning = self.full_default && (self.ocr_busy || self.crop.is_none());
        let target = if self.sel.is_none() || scanning {
            1.0
        } else {
            0.0
        };
        let speed = if target > self.glow { 2.5 } else { 4.0 };
        self.glow += (target - self.glow).clamp(-speed * dt, speed * dt);
        if now - t0 < 0.05 {
            self.glow = self.glow.min(0.2);
        }

        let screen = ui.max_rect();
        let show_ui = self.sel.is_some() && self.crop.is_some() && self.quick.is_none();
        let panel_rect = self.panel_rect(screen);

        // The active text on the image: the translation while it shows
        // (hold Space for the original), else the OCR words.
        let fr = self.frame_rect(screen);
        let hold_original =
            !ctx.egui_wants_keyboard_input() && ctx.input(|i| i.key_down(egui::Key::Space));
        if self.show_translation {
            self.ensure_clean(&ctx);
        }
        let tlines = if self.show_translation && !hold_original {
            self.translation_layout(ui.painter(), fr)
        } else {
            Vec::new()
        };
        let on_translation = !tlines.is_empty();
        if on_translation != self.sel_on_translation {
            self.words_sel = None;
            self.sel_on_translation = on_translation;
        }
        self.tword_texts = tlines
            .iter()
            .flat_map(|l| l.words.iter().map(|w| w.text.clone()))
            .collect();
        let words: Vec<Rect> = if on_translation {
            tlines
                .iter()
                .flat_map(|l| l.words.iter().map(|w| w.rect))
                .collect()
        } else {
            self.word_rects(fr)
        };
        if self.debug_select
            && on_translation
            && !self.translate_busy
            && self.tword_texts.len() >= 3
        {
            self.debug_select = false;
            self.words_sel = Some((0, 2));
            self.copy_text(&ctx);
        }

        self.handle_input(ui, screen, &words);
        self.paint_scene(ui, screen, now, &tlines, &words);

        if show_ui {
            let open = self.panel != PanelKind::None;
            if open {
                self.panel(&ctx, panel_rect);
            }
            self.chips(&ctx, screen, open.then_some(panel_rect));
        }

        let animating = self.glow > 0.001
            || self.ocr_busy
            || now - self.ocr_done_at < 0.35
            || (self.full_default && now - t0 < 4.6)
            || self.toast.as_ref().is_some_and(|(_, t)| now - t < 3.6);
        if animating {
            ctx.request_repaint();
        }
    }
}

// ------------------------------------------------------------ painters

fn shadow() -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset: [0, 6],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(140),
    }
}

/// Colour at position `u` (0..1, wraps) of the glow gradient.
fn glow_color(u: f32) -> Color32 {
    let u = u.rem_euclid(1.0) * GLOW.len() as f32;
    let i = u.floor() as usize % GLOW.len();
    let j = (i + 1) % GLOW.len();
    let f = u.fract();
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * f) as u8;
    let (a, b) = (GLOW[i], GLOW[j]);
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

fn with_alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}

/// A moving multi-colour glow along the inside of the screen edges.
fn glow(painter: &egui::Painter, screen: Rect, now: f64, level: f32) {
    if level <= 0.001 {
        return;
    }
    let phase = (now * 0.12) as f32;
    let pulse = 0.8 + 0.2 * ((now * 2.2).sin() as f32);
    let perimeter = 2.0 * (screen.width() + screen.height());
    let edges = [
        (screen.left_top(), screen.right_top(), vec2(0.0, 1.0)),
        (screen.right_top(), screen.right_bottom(), vec2(-1.0, 0.0)),
        (screen.right_bottom(), screen.left_bottom(), vec2(0.0, -1.0)),
        (screen.left_bottom(), screen.left_top(), vec2(1.0, 0.0)),
    ];
    for (width, alpha) in [(90.0, 0.45), (26.0, 0.65), (5.0, 0.95)] {
        let mut mesh = Mesh::default();
        let mut dist = 0.0;
        for (a, b, normal) in edges {
            let len = a.distance(b);
            let steps = (len / 20.0).ceil().max(1.0) as usize;
            for k in 0..=steps {
                let t = k as f32 / steps as f32;
                let p = a + (b - a) * t;
                let u = (dist + len * t) / perimeter + phase;
                let c = with_alpha(glow_color(u), alpha * level * pulse);
                let idx = mesh.vertices.len() as u32;
                mesh.colored_vertex(p, c);
                mesh.colored_vertex(p + normal * width, Color32::TRANSPARENT);
                if k > 0 {
                    mesh.add_triangle(idx - 2, idx - 1, idx);
                    mesh.add_triangle(idx - 1, idx, idx + 1);
                }
            }
            dist += len;
        }
        painter.add(Shape::mesh(mesh));
    }
}

/// A light band that sweeps across the selection while work is running.
fn shimmer(painter: &egui::Painter, r: Rect, now: f64) {
    let band = (r.width() * 0.35).clamp(60.0, 240.0);
    let travel = r.width() + band * 2.0;
    let x = r.min.x - band + ((now * 420.0) as f32 % travel);
    let p = painter.with_clip_rect(r);
    let mut mesh = Mesh::default();
    let clear = Color32::TRANSPARENT;
    let light = Color32::from_white_alpha(46);
    for (i, (xx, c)) in [(x - band, clear), (x, light), (x + band, clear)]
        .into_iter()
        .enumerate()
    {
        mesh.colored_vertex(pos2(xx - 40.0, r.max.y), c);
        mesh.colored_vertex(pos2(xx + 40.0, r.min.y), c);
        if i > 0 {
            let n = mesh.vertices.len() as u32;
            mesh.add_triangle(n - 4, n - 3, n - 2);
            mesh.add_triangle(n - 3, n - 2, n - 1);
        }
    }
    p.add(Shape::mesh(mesh));
}

/// Rounded white corner brackets around the selection.
fn brackets(painter: &egui::Painter, r: Rect) {
    let len = (r.width().min(r.height()) * 0.28).clamp(14.0, 40.0);
    let rad = (len * 0.55).min(18.0);
    let corners = [
        (r.left_top(), vec2(1.0, 0.0), vec2(0.0, 1.0)),
        (r.right_top(), vec2(-1.0, 0.0), vec2(0.0, 1.0)),
        (r.right_bottom(), vec2(-1.0, 0.0), vec2(0.0, -1.0)),
        (r.left_bottom(), vec2(1.0, 0.0), vec2(0.0, -1.0)),
    ];
    for (c, dx, dy) in corners {
        // Arm along x, quarter circle at the corner, arm along y.
        let mut pts = vec![c + dx * len];
        let centre = c + dx * rad + dy * rad;
        for k in 0..=8 {
            let a = std::f32::consts::FRAC_PI_2 * k as f32 / 8.0;
            pts.push(centre - dy * rad * a.cos() - dx * rad * a.sin());
        }
        pts.push(c + dy * len);
        painter.add(Shape::line(
            pts.clone(),
            Stroke::new(8.0, Color32::from_black_alpha(60)),
        ));
        painter.add(Shape::line(pts, Stroke::new(4.5, Color32::WHITE)));
    }
    painter.rect_stroke(
        r.shrink(4.0),
        6.0,
        Stroke::new(1.0, Color32::from_white_alpha(40)),
        StrokeKind::Middle,
    );
}

fn pill(painter: &egui::Painter, center: Pos2, text: &str, alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let galley = painter.layout_no_wrap(
        text.to_string(),
        egui::FontId::proportional(15.0),
        with_alpha(Color32::WHITE, alpha),
    );
    let rect = Rect::from_center_size(center, galley.size() + vec2(36.0, 20.0));
    painter.rect_filled(rect, 22.0, with_alpha(PANEL_BG, 0.92 * alpha));
    painter.galley(rect.min + vec2(18.0, 10.0), galley, Color32::WHITE);
}

/// The four rectangles of `outer` that lie outside `inner`.
fn outside(outer: Rect, inner: Rect) -> [Rect; 4] {
    [
        Rect::from_min_max(outer.min, pos2(outer.max.x, inner.min.y)),
        Rect::from_min_max(pos2(outer.min.x, inner.max.y), outer.max),
        Rect::from_min_max(
            pos2(outer.min.x, inner.min.y),
            pos2(inner.min.x, inner.max.y),
        ),
        Rect::from_min_max(
            pos2(inner.max.x, inner.min.y),
            pos2(outer.max.x, inner.max.y),
        ),
    ]
}

// ------------------------------------------------------------ widgets

fn big_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(
        egui::RichText::new(text.to_string())
            .color(ACCENT_TEXT)
            .strong(),
    )
    .fill(ACCENT)
    .corner_radius(CornerRadius::same(22))
    .min_size(vec2(0.0, 38.0))
}

/// Dark theme with larger text and roomier controls.
pub fn apply_style(ctx: &egui::Context) {
    use egui::{FontId, TextStyle};
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Heading, FontId::proportional(20.0)),
            (TextStyle::Body, FontId::proportional(15.0)),
            (TextStyle::Button, FontId::proportional(15.0)),
            (TextStyle::Monospace, FontId::monospace(14.0)),
            (TextStyle::Small, FontId::proportional(12.5)),
        ]
        .into();
        s.spacing.button_padding = vec2(14.0, 6.0);
        s.spacing.item_spacing = vec2(8.0, 8.0);
        s.spacing.interact_size.y = 32.0;
        s.visuals.selection.bg_fill = Color32::from_rgb(66, 133, 244);
        s.visuals.extreme_bg_color = Color32::from_gray(24);
        s.visuals.widgets.inactive.corner_radius = CornerRadius::same(10);
        s.visuals.widgets.hovered.corner_radius = CornerRadius::same(10);
        s.visuals.widgets.active.corner_radius = CornerRadius::same(10);
        s.visuals.window_fill = PANEL_BG;
    });
}

/// Phosphor glyph for each action.
fn glyph(icon: Icon) -> &'static str {
    match icon {
        Icon::Copy => icons::COPY,
        Icon::Text => icons::TEXT_T,
        Icon::Translate => icons::TRANSLATE,
        Icon::Search => icons::MAGNIFYING_GLASS,
        Icon::Link => icons::LINK,
        Icon::Sparkle => icons::SPARKLE,
        Icon::Image => icons::IMAGE,
        Icon::Qr => icons::QR_CODE,
    }
}

/// Draw an icon glyph centred on `c` (fill weight when `filled`).
fn draw_icon(p: &egui::Painter, icon: Icon, c: Pos2, color: Color32, filled: bool) {
    p.text(
        c,
        egui::Align2::CENTER_CENTER,
        glyph(icon),
        icons::font(21.0, filled),
        color,
    );
}

/// Round icon button for the action bar.
fn icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    tip: &str,
    active: bool,
    enabled: bool,
) -> egui::Response {
    let size = vec2(42.0, 42.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let resp = resp.on_hover_text(tip);
    let hovered = resp.hovered() && enabled;
    let t = ui.ctx().animate_bool(resp.id.with("hover"), hovered);
    let p = ui.painter();
    if active {
        p.circle_filled(rect.center(), 20.0, ACCENT);
    } else if t > 0.0 {
        p.circle_filled(
            rect.center(),
            20.0,
            Color32::from_white_alpha((22.0 * t) as u8),
        );
    }
    let fg = if active {
        ACCENT_TEXT
    } else if enabled {
        Color32::from_gray((215.0 + 40.0 * t) as u8)
    } else {
        Color32::from_gray(105)
    };
    draw_icon(p, icon, rect.center(), fg, active);
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// The "Search" pill with a magnifier icon.
fn search_button(ui: &mut egui::Ui) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        "Search".into(),
        egui::FontId::proportional(15.0),
        ACCENT_TEXT,
    );
    let size = vec2(galley.size().x + 56.0, 40.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let t = ui.ctx().animate_bool(resp.id.with("hover"), resp.hovered());
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
    let fill = Color32::from_rgb(
        lerp(ACCENT.r(), 174),
        lerp(ACCENT.g(), 203),
        lerp(ACCENT.b(), 250),
    );
    let p = ui.painter();
    p.rect_filled(rect, 20.0, fill);
    draw_icon(
        p,
        Icon::Search,
        pos2(rect.min.x + 23.0, rect.center().y),
        ACCENT_TEXT,
        true,
    );
    p.galley(
        pos2(rect.min.x + 40.0, rect.center().y - galley.size().y / 2.0),
        galley,
        ACCENT_TEXT,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Median colour of the crop pixels on a thin ring just outside `rect`
/// (x, y, w, h). This is the background the translation is painted on.
fn ring_median(img: &RgbaImage, rect: [f32; 4]) -> Color32 {
    let (iw, ih) = (img.width() as i64, img.height() as i64);
    let [x, y, w, h] = rect;
    let (x0, y0) = (x as i64 - 3, y as i64 - 3);
    let (x1, y1) = ((x + w) as i64 + 3, (y + h) as i64 + 3);
    let mut px: Vec<[u8; 3]> = Vec::new();
    let mut take = |xx: i64, yy: i64| {
        if xx >= 0 && yy >= 0 && xx < iw && yy < ih {
            let p = img.get_pixel(xx as u32, yy as u32);
            px.push([p[0], p[1], p[2]]);
        }
    };
    for xx in (x0..=x1).step_by(2) {
        take(xx, y0);
        take(xx, y1);
    }
    for yy in (y0..=y1).step_by(2) {
        take(x0, yy);
        take(x1, yy);
    }
    if px.is_empty() {
        return Color32::from_gray(240);
    }
    let ch = |i: usize| {
        let mut v: Vec<u8> = px.iter().map(|p| p[i]).collect();
        v.sort_unstable();
        v[v.len() / 2]
    };
    Color32::from_rgb(ch(0), ch(1), ch(2))
}
