//! QR code and barcode detection with rxing (pure Rust ZXing port).

use image::RgbaImage;

#[derive(Debug, Clone, PartialEq)]
pub struct Code {
    pub text: String,
    pub format: String,
}

impl Code {
    pub fn is_url(&self) -> bool {
        let t = self.text.trim();
        t.starts_with("http://") || t.starts_with("https://")
    }
}

fn pretty_format(raw: &str) -> String {
    match raw.to_lowercase().replace([' ', '_', '-'], "").as_str() {
        "qrcode" => "QR code".into(),
        "microqrcode" => "Micro QR code".into(),
        "ean13" => "EAN-13".into(),
        "ean8" => "EAN-8".into(),
        "upca" => "UPC-A".into(),
        "upce" => "UPC-E".into(),
        "code128" => "Code 128".into(),
        "code39" => "Code 39".into(),
        "code93" => "Code 93".into(),
        "datamatrix" => "Data Matrix".into(),
        "pdf417" => "PDF417".into(),
        "aztec" => "Aztec".into(),
        "itf" => "ITF".into(),
        "codabar" => "Codabar".into(),
        _ => raw.to_uppercase(),
    }
}

/// Find all codes in the image. An image with no codes gives an empty list.
pub fn detect(img: &RgbaImage) -> Vec<Code> {
    let luma = image::DynamicImage::ImageRgba8(img.clone()).to_luma8();
    let (w, h) = luma.dimensions();
    let mut hints = rxing::DecodeHints::default().with(rxing::DecodeHintValue::TryHarder(true));
    let results =
        rxing::helpers::detect_multiple_in_luma_with_hints(luma.into_raw(), w, h, &mut hints)
            .unwrap_or_default();
    let mut codes: Vec<Code> = Vec::new();
    for r in results {
        let code = Code {
            text: r.getText().to_string(),
            format: pretty_format(&r.getBarcodeFormat().to_string()),
        };
        if !codes.contains(&code) {
            codes.push(code);
        }
    }
    codes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> RgbaImage {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        image::open(path).unwrap().to_rgba8()
    }

    #[test]
    fn decodes_qr() {
        let codes = detect(&fixture("qr.png"));
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].text, "https://example.com/rlens-test");
        assert!(codes[0].is_url());
        assert_eq!(codes[0].format, "QR code");
    }

    #[test]
    fn decodes_ean13() {
        let codes = detect(&fixture("ean13.png"));
        assert!(
            codes
                .iter()
                .any(|c| c.text == "5901234123457" && c.format == "EAN-13"),
            "{codes:?}"
        );
    }

    #[test]
    fn decodes_qr_next_to_text() {
        let codes = detect(&fixture("mixed.png"));
        assert!(
            codes
                .iter()
                .any(|c| c.text == "https://example.com/rlens-test")
        );
    }

    #[test]
    fn no_codes_in_plain_text() {
        assert!(detect(&fixture("text.png")).is_empty());
    }
}
