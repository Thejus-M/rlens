use super::Cancelled;
use anyhow::{Context, Result, bail};
use ashpd::desktop::screenshot::Screenshot;
use image::RgbaImage;

/// Ask xdg-desktop-portal for a full-screen screenshot (GNOME, KDE, ...).
/// The rlens overlay does the area selection.
pub fn capture() -> Result<RgbaImage> {
    let uri = pollster::block_on(async {
        let request = Screenshot::request()
            .interactive(false)
            .modal(true)
            .send()
            .await?;
        request.response()
    });
    let uri = match uri {
        Ok(shot) => shot.uri().to_string(),
        Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)) => {
            return Err(Cancelled.into());
        }
        Err(e) => bail!("portal screenshot: {e}"),
    };
    let url = url::Url::parse(&uri).context("parse portal URI")?;
    let path = url
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("portal URI is not a file: {uri}"))?;
    let img = image::open(&path).with_context(|| format!("open {}", path.display()))?;
    // The portal saves into the user's Pictures folder; rlens does not keep it.
    let _ = std::fs::remove_file(&path);
    Ok(img.to_rgba8())
}
