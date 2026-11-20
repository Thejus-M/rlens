//! Clipboard access for X11 and Wayland.
//!
//! On Wayland the clipboard content belongs to the process that set it.
//! `wl-copy` and `xclip` fork a helper that keeps the content alive after
//! rlens exits, so rlens prefers them. arboard is the fallback.

use anyhow::{Context, Result, bail};
use image::RgbaImage;
use std::io::Write;
use std::process::{Command, Stdio};

fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn pipe_to(cmd: &str, args: &[&str], data: &[u8]) -> Result<()> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("run {cmd}"))?;
    child.stdin.take().context("stdin")?.write_all(data)?;
    // wl-copy forks and the parent exits quickly; xclip stays in background.
    if cmd == "wl-copy" {
        child.wait()?;
    }
    Ok(())
}

fn external(mime: &str) -> Option<(&'static str, Vec<String>)> {
    if wayland() && which::which("wl-copy").is_ok() {
        return Some(("wl-copy", vec!["--type".into(), mime.into()]));
    }
    if std::env::var_os("DISPLAY").is_some() && which::which("xclip").is_ok() {
        return Some((
            "xclip",
            vec![
                "-selection".into(),
                "clipboard".into(),
                "-t".into(),
                mime.into(),
            ],
        ));
    }
    None
}

pub fn set_text(text: &str) -> Result<()> {
    if let Some((cmd, args)) = external("text/plain;charset=utf-8") {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        return pipe_to(cmd, &args, text.as_bytes());
    }
    arboard::Clipboard::new()?.set_text(text.to_string())?;
    Ok(())
}

/// Copy an image as PNG.
pub fn set_image(img: &RgbaImage) -> Result<()> {
    if let Some((cmd, args)) = external("image/png") {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        return pipe_to(cmd, &args, &crate::net::encode_png(img)?);
    }
    arboard::Clipboard::new()?.set_image(arboard::ImageData {
        width: img.width() as usize,
        height: img.height() as usize,
        bytes: img.as_raw().into(),
    })?;
    Ok(())
}

pub fn get_image() -> Result<RgbaImage> {
    if wayland() && which::which("wl-paste").is_ok() {
        let out = Command::new("wl-paste")
            .args(["--type", "image/png"])
            .output()?;
        if out.status.success() && !out.stdout.is_empty() {
            return Ok(image::load_from_memory(&out.stdout)?.to_rgba8());
        }
    }
    let data = arboard::Clipboard::new()?
        .get_image()
        .context("no image in the clipboard")?;
    match RgbaImage::from_raw(
        data.width as u32,
        data.height as u32,
        data.bytes.into_owned(),
    ) {
        Some(img) => Ok(img),
        None => bail!("bad clipboard image"),
    }
}
