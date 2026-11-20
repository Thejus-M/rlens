//! X11: grab the monitor under the pointer with xcap.

use anyhow::{Context, Result};
use image::RgbaImage;
use std::process::Command;

fn pointer_position() -> Option<(i32, i32)> {
    let out = Command::new("xdotool")
        .args(["getmouselocation", "--shell"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let get = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|v| v.trim().parse::<i32>().ok())
    };
    Some((get("X=")?, get("Y=")?))
}

fn grab_monitor() -> Result<RgbaImage> {
    let monitor = match pointer_position() {
        Some((x, y)) => xcap::Monitor::from_point(x, y).ok(),
        None => None,
    };
    let monitor = match monitor {
        Some(m) => m,
        None => {
            let all = xcap::Monitor::all().context("list monitors")?;
            let primary = all
                .iter()
                .position(|m| m.is_primary().unwrap_or(false))
                .unwrap_or(0);
            all.into_iter().nth(primary).context("no monitor found")?
        }
    };
    monitor.capture_image().context("capture monitor")
}

pub fn capture() -> Result<RgbaImage> {
    grab_monitor()
}
