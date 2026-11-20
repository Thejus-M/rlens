//! wlroots compositors (Hyprland, Sway, river, Wayfire, niri, ...): `grim`.

use anyhow::{Context, Result, bail};
use image::RgbaImage;
use std::process::Command;

pub fn available() -> bool {
    which::which("grim").is_ok()
}

fn json(cmd: &str, args: &[&str]) -> Option<serde_json::Value> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status.success().then_some(())?;
    serde_json::from_slice(&out.stdout).ok()
}

/// Name of the output that has focus, so the freeze frame matches the
/// monitor where the overlay window opens.
fn focused_output() -> Option<String> {
    let pick = |list: serde_json::Value, key: &str| {
        list.as_array()?
            .iter()
            .find(|m| m.get(key).and_then(|f| f.as_bool()) == Some(true))
            .and_then(|m| m.get("name")?.as_str().map(str::to_string))
    };
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
        && let Some(v) = json("hyprctl", &["monitors", "-j"])
    {
        return pick(v, "focused");
    }
    if std::env::var_os("SWAYSOCK").is_some()
        && let Some(v) = json("swaymsg", &["-t", "get_outputs", "-r"])
    {
        return pick(v, "focused");
    }
    if std::env::var_os("NIRI_SOCKET").is_some()
        && let Some(v) = json("niri", &["msg", "-j", "focused-output"])
    {
        return v.get("name")?.as_str().map(str::to_string);
    }
    None
}

pub fn capture() -> Result<RgbaImage> {
    if !available() {
        bail!("needs `grim`");
    }
    let mut cmd = Command::new("grim");
    if let Some(output) = focused_output() {
        tracing::debug!("grim output: {output}");
        cmd.args(["-o", &output]);
    }
    let out = cmd.args(["-t", "png", "-"]).output().context("run grim")?;
    if !out.status.success() {
        bail!(
            "grim failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let img = image::load_from_memory(&out.stdout).context("decode grim output")?;
    Ok(img.to_rgba8())
}
