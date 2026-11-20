pub mod app;
mod fonts;
mod icons;

use crate::{config::Config, lens_browser};
use anyhow::Result;
use eframe::egui;
use image::RgbaImage;
use std::sync::{Arc, Mutex};

/// Show the full-screen overlay on `frame`.
/// In quick mode the overlay closes after the selection and returns the crop.
pub fn run(
    cfg: Config,
    frame: RgbaImage,
    quick: bool,
    area: Option<[f32; 4]>,
) -> Result<Option<RgbaImage>> {
    let out = Arc::new(Mutex::new(None));
    let quick_out = quick.then(|| out.clone());
    // On Hyprland and Sway a fullscreen window hides the floating Lens
    // panel, so the overlay is a borderless floating window that covers the
    // monitor. Other desktops use real fullscreen.
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("rlens")
        .with_app_id("rlens")
        .with_decorations(false);
    match lens_browser::monitor_geometry().filter(|_| lens_browser::can_place()) {
        Some(mon) => {
            viewport = viewport.with_inner_size([mon.2 as f32, mon.3 as f32]);
            let pid = std::process::id();
            std::thread::spawn(move || {
                lens_browser::place(lens_browser::Target::Pid(pid), mon, false)
            });
        }
        None => viewport = viewport.with_fullscreen(true),
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "rlens",
        options,
        Box::new(move |cc| Ok(Box::new(app::LensApp::new(cc, cfg, frame, quick_out, area)))),
    )
    .map_err(|e| anyhow::anyhow!("window: {e}"))?;
    lens_browser::release_panel();
    let crop = out.lock().unwrap().take();
    Ok(crop)
}
