//! Remove text from an image region so translated text can be drawn in its
//! place on the real background, as Google Lens does.
//!
//! The text pixels are found by their distance from the local background
//! colour. Each row's runs of text pixels are filled by linear
//! interpolation between the clean pixels on both sides, then lightly
//! blurred so no seams show.

use image::RgbaImage;

/// Colour difference (sum of |dR|+|dG|+|dB|) above which a pixel is text.
const THRESHOLD: i32 = 60;

fn dist(p: &image::Rgba<u8>, bg: [u8; 3]) -> i32 {
    (0..3).map(|i| (p[i] as i32 - bg[i] as i32).abs()).sum()
}

/// Pixel bounds (x0, y0, x1, y1) of `rect` grown by `pad`, clamped to the image.
fn bounds(img: &RgbaImage, rect: [f32; 4], pad: i64) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = (img.width() as i64, img.height() as i64);
    let x0 = (rect[0] as i64 - pad).clamp(0, w);
    let y0 = (rect[1] as i64 - pad).clamp(0, h);
    let x1 = ((rect[0] + rect[2]).ceil() as i64 + pad).clamp(0, w);
    let y1 = ((rect[1] + rect[3]).ceil() as i64 + pad).clamp(0, h);
    (x1 > x0 && y1 > y0).then_some((x0 as u32, y0 as u32, x1 as u32, y1 as u32))
}

/// Text mask over the bounds, dilated by one pixel.
fn mask(img: &RgbaImage, b: (u32, u32, u32, u32), bg: [u8; 3]) -> Vec<bool> {
    let (x0, y0, x1, y1) = b;
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut raw = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            raw[y * w + x] = dist(img.get_pixel(x0 + x as u32, y0 + y as u32), bg) > THRESHOLD;
        }
    }
    let mut out = raw.clone();
    for y in 0..h {
        for x in 0..w {
            if raw[y * w + x] {
                continue;
            }
            let near = (y.saturating_sub(1)..=(y + 1).min(h - 1))
                .any(|yy| (x.saturating_sub(1)..=(x + 1).min(w - 1)).any(|xx| raw[yy * w + xx]));
            out[y * w + x] = near;
        }
    }
    out
}

/// Erase the text inside `rect` (x, y, w, h) whose background is about `bg`.
pub fn erase_text(img: &mut RgbaImage, rect: [f32; 4], bg: [u8; 3]) {
    let Some(b) = bounds(img, rect, 3) else {
        return;
    };
    let (x0, y0, x1, y1) = b;
    let w = (x1 - x0) as usize;
    let h = (y1 - y0) as usize;
    let m = mask(img, b, bg);

    // Row-wise linear fill between the clean neighbours of each masked run.
    for y in 0..h {
        let mut x = 0;
        while x < w {
            if !m[y * w + x] {
                x += 1;
                continue;
            }
            let start = x;
            while x < w && m[y * w + x] {
                x += 1;
            }
            let py = y0 + y as u32;
            let left = (start > 0).then(|| *img.get_pixel(x0 + start as u32 - 1, py));
            let right = (x < w).then(|| *img.get_pixel(x0 + x as u32, py));
            let fallback = image::Rgba([bg[0], bg[1], bg[2], 255]);
            let (a, c) = match (left, right) {
                (Some(l), Some(r)) => (l, r),
                (Some(l), None) => (l, l),
                (None, Some(r)) => (r, r),
                (None, None) => (fallback, fallback),
            };
            let n = (x - start + 1) as f32;
            for (k, xx) in (start..x).enumerate() {
                let t = (k + 1) as f32 / n;
                let mix = |i: usize| (a[i] as f32 * (1.0 - t) + c[i] as f32 * t).round() as u8;
                img.put_pixel(
                    x0 + xx as u32,
                    py,
                    image::Rgba([mix(0), mix(1), mix(2), 255]),
                );
            }
        }
    }

    // Soften the filled pixels with a 3×3 box blur (masked pixels only).
    let snapshot = img.clone();
    for y in 0..h {
        for x in 0..w {
            if !m[y * w + x] {
                continue;
            }
            let (px, py) = (x0 + x as u32, y0 + y as u32);
            let mut acc = [0u32; 3];
            let mut n = 0;
            for yy in py.saturating_sub(1)..=(py + 1).min(img.height() - 1) {
                for xx in px.saturating_sub(1)..=(px + 1).min(img.width() - 1) {
                    let p = snapshot.get_pixel(xx, yy);
                    for i in 0..3 {
                        acc[i] += p[i] as u32;
                    }
                    n += 1;
                }
            }
            let v = |i: usize| (acc[i] / n) as u8;
            img.put_pixel(px, py, image::Rgba([v(0), v(1), v(2), 255]));
        }
    }
}

/// The colour of the text (ink) inside `rect`: the median of the pixels that
/// differ most from the background. `None` when no text pixels are found.
pub fn text_color(img: &RgbaImage, rect: [f32; 4], bg: [u8; 3]) -> Option<[u8; 3]> {
    let (x0, y0, x1, y1) = bounds(img, rect, 0)?;
    let mut px: Vec<(i32, [u8; 3])> = Vec::new();
    for y in y0..y1 {
        for x in x0..x1 {
            let p = img.get_pixel(x, y);
            let d = dist(p, bg);
            if d > THRESHOLD {
                px.push((d, [p[0], p[1], p[2]]));
            }
        }
    }
    if px.is_empty() {
        return None;
    }
    // The strongest half: the solid core of the glyphs, not their edges.
    px.sort_unstable_by_key(|(d, _)| -d);
    px.truncate((px.len() / 2).max(1));
    let med = |i: usize| {
        let mut v: Vec<u8> = px.iter().map(|(_, c)| c[i]).collect();
        v.sort_unstable();
        v[v.len() / 2]
    };
    Some([med(0), med(1), med(2)])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A horizontal gradient with a black bar ("text") in the middle.
    fn sample() -> RgbaImage {
        RgbaImage::from_fn(120, 40, |x, y| {
            if (40..80).contains(&x) && (15..25).contains(&y) {
                image::Rgba([0, 0, 0, 255])
            } else {
                let v = 150 + (x as u8 / 2);
                image::Rgba([v, v, 230, 255])
            }
        })
    }

    #[test]
    fn erases_text_and_keeps_gradient() {
        let mut img = sample();
        erase_text(&mut img, [38.0, 13.0, 44.0, 14.0], [180, 180, 230]);
        for x in [40u32, 60, 79] {
            let p = img.get_pixel(x, 20);
            let want = 150 + (x as i32 / 2);
            assert!(
                (p[0] as i32 - want).abs() <= 6,
                "x={x} got {:?} want {want}",
                p
            );
            assert!((p[2] as i32 - 230).abs() <= 6);
        }
    }

    #[test]
    fn finds_ink_colour() {
        let img = sample();
        let ink = text_color(&img, [38.0, 13.0, 44.0, 14.0], [180, 180, 230]).unwrap();
        assert!(ink.iter().all(|c| *c < 20), "{ink:?}");
    }
}
