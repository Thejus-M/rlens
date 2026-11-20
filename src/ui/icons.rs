//! Phosphor icons (MIT), embedded as two icon fonts: regular and fill.

use eframe::egui;

pub const COPY: &str = "\u{E1CA}";
pub const TEXT_T: &str = "\u{E48A}";
pub const TRANSLATE: &str = "\u{E4A2}";
pub const IMAGE: &str = "\u{E2CA}";
pub const MAGNIFYING_GLASS: &str = "\u{E30C}";
pub const LINK: &str = "\u{E2E2}";
pub const SPARKLE: &str = "\u{E6A2}";
pub const X: &str = "\u{E4F6}";
pub const QR_CODE: &str = "\u{E3E6}";

const REGULAR: &[u8] = include_bytes!("../../assets/fonts/Phosphor.ttf");
const FILL: &[u8] = include_bytes!("../../assets/fonts/Phosphor-Fill.ttf");

/// Register the two icon fonts as their own font families.
pub fn install(ctx: &egui::Context) {
    for (name, bytes) in [("phosphor", REGULAR), ("phosphor-fill", FILL)] {
        let family = egui::FontFamily::Name(name.into());
        ctx.add_font(egui::epaint::text::FontInsert::new(
            name,
            egui::FontData::from_static(bytes),
            vec![egui::epaint::text::InsertFontFamily {
                family,
                priority: egui::epaint::text::FontPriority::Highest,
            }],
        ));
    }
}

pub fn font(size: f32, filled: bool) -> egui::FontId {
    let name = if filled { "phosphor-fill" } else { "phosphor" };
    egui::FontId::new(size, egui::FontFamily::Name(name.into()))
}
