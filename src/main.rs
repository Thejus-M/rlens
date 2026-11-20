//! rlens: a Google Lens style tool for every Linux desktop.

mod barcode;
mod capture;
mod clipboard;
mod config;
mod inpaint;
mod lens_browser;
mod net;
mod ocr;
mod ui;

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Select a screen area, then read text, decode codes, translate, or search it."
)]
struct Cli {
    /// Use an image file instead of a screen capture.
    #[arg(long, value_name = "IMAGE")]
    file: Option<PathBuf>,

    /// Use the image in the clipboard instead of a screen capture.
    #[arg(long, conflicts_with = "file")]
    clipboard: bool,

    /// Screen capture backend.
    #[arg(long, value_enum, default_value_t = capture::Backend::Auto)]
    backend: capture::Backend,

    /// No window: copy the OCR text to the clipboard and show a notification.
    #[arg(long, group = "quick")]
    ocr: bool,

    /// No window: copy the first QR/barcode to the clipboard (open it if it is a link).
    #[arg(long, group = "quick")]
    qr: bool,

    /// No window: send the selection to Google Lens and open the result.
    #[arg(long, group = "quick")]
    lens: bool,

    /// Preselect an area (frame pixels) instead of dragging, e.g. `100,200,640,360`.
    #[arg(long, value_name = "X,Y,W,H", value_parser = parse_area)]
    area: Option<[f32; 4]>,

    /// Print the OCR text to stdout instead of copying it (with --ocr).
    #[arg(long, requires = "ocr")]
    print: bool,
}

fn parse_area(s: &str) -> Result<[f32; 4], String> {
    let v: Vec<f32> = s
        .split(',')
        .map(|p| p.trim().parse::<f32>().map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    match v[..] {
        [x, y, w, h] if w > 0.0 && h > 0.0 => Ok([x, y, w, h]),
        _ => Err("expected X,Y,W,H with W and H above 0".into()),
    }
}

fn notify(summary: &str, body: &str) {
    let _ = notify_rust::Notification::new()
        .appname("rlens")
        .summary(summary)
        .body(body)
        .icon("edit-find")
        .show();
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("RLENS_LOG")
                .unwrap_or_else(|_| "rlens=warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.is::<capture::Cancelled>() => ExitCode::from(1),
        Err(e) => {
            eprintln!("rlens: {e:#}");
            notify("rlens error", &format!("{e:#}"));
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let cfg = config::Config::load();

    let frame = if let Some(path) = &cli.file {
        image::open(path)
            .with_context(|| format!("open {}", path.display()))?
            .to_rgba8()
    } else if cli.clipboard {
        clipboard::get_image()?
    } else {
        capture::capture(cli.backend)?
    };

    let quick = cli.ocr || cli.qr || cli.lens;
    if !quick {
        ui::run(cfg, frame, false, cli.area)?;
        return Ok(());
    }
    let Some(img) = ui::run(cfg.clone(), frame, true, cli.area)? else {
        return Err(capture::Cancelled.into());
    };

    if cli.ocr {
        let res = ocr::recognize(&img, &cfg.ocr.langs)?;
        if cli.print {
            println!("{}", res.text);
        } else if res.text.trim().is_empty() {
            notify("rlens", "No text found");
        } else {
            clipboard::set_text(&res.text)?;
            let preview: String = res.text.chars().take(160).collect();
            notify("Text copied", &preview);
        }
    } else if cli.qr {
        let codes = barcode::detect(&img);
        let Some(code) = codes.first() else {
            notify("rlens", "No QR code or barcode found");
            return Ok(());
        };
        clipboard::set_text(&code.text)?;
        if code.is_url() {
            net::open_url(code.text.trim())?;
        }
        notify(&format!("{} copied", code.format), &code.text);
    } else if cli.lens {
        lens_browser::open(
            &img,
            lens_browser::Side::Right,
            &cfg.lens.hl,
            &cfg.lens.browser,
        )?;
    }
    Ok(())
}
