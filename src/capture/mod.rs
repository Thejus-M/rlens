//! Freeze-frame capture of the screen. The rlens overlay shows this frame
//! and the user selects an area on it. Backends:
//!
//! * `wlroots`: `grim` (Hyprland, Sway, river, Wayfire, niri, ...).
//! * `portal`: xdg-desktop-portal Screenshot (GNOME, KDE Plasma, and others).
//! * `x11`: xcap (any X11 desktop).

mod portal;
mod wlroots;
mod x11;

use anyhow::{Result, bail};
use image::RgbaImage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Backend {
    Auto,
    Wlroots,
    Portal,
    X11,
}

/// Returned when the user cancels.
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("selection cancelled")
    }
}

impl std::error::Error for Cancelled {}

fn env_has(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| !v.is_empty())
}

fn desktop() -> String {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_lowercase()
}

/// GNOME and KDE do not implement the wlroots screencopy protocol.
fn is_gnome_or_kde() -> bool {
    let d = desktop();
    d.contains("gnome") || d.contains("kde") || d.contains("plasma") || d.contains("unity")
}

pub fn capture(backend: Backend) -> Result<RgbaImage> {
    match backend {
        Backend::Wlroots => wlroots::capture(),
        Backend::Portal => portal::capture(),
        Backend::X11 => x11::capture(),
        Backend::Auto => capture_auto(),
    }
}

fn capture_auto() -> Result<RgbaImage> {
    let wayland = env_has("WAYLAND_DISPLAY");
    let mut errors = Vec::new();

    if wayland && !is_gnome_or_kde() && wlroots::available() {
        match wlroots::capture() {
            Ok(img) => return Ok(img),
            Err(e) if e.is::<Cancelled>() => return Err(e),
            Err(e) => {
                tracing::warn!("wlroots backend: {e:#}");
                errors.push(format!("wlroots: {e:#}"));
            }
        }
    }

    if wayland || env_has("DBUS_SESSION_BUS_ADDRESS") {
        match portal::capture() {
            Ok(img) => return Ok(img),
            Err(e) if e.is::<Cancelled>() => return Err(e),
            Err(e) => {
                tracing::warn!("portal backend: {e:#}");
                errors.push(format!("portal: {e:#}"));
            }
        }
    }

    if env_has("DISPLAY") {
        match x11::capture() {
            Ok(img) => return Ok(img),
            Err(e) if e.is::<Cancelled>() => return Err(e),
            Err(e) => errors.push(format!("x11: {e:#}")),
        }
    }

    if errors.is_empty() {
        bail!("no capture backend available (no WAYLAND_DISPLAY or DISPLAY)");
    }
    bail!("all capture backends failed:\n  {}", errors.join("\n  "))
}
