//! User configuration, stored at `$XDG_CONFIG_HOME/rlens/config.toml`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub ocr: OcrConfig,
    pub translate: TranslateConfig,
    pub ollama: OllamaConfig,
    pub lens: LensConfig,
    pub ui: UiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiConfig {
    /// Start with the whole screen selected: text is read at once and a
    /// drag selects a smaller area.
    pub select_all: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self { select_all: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OcrConfig {
    /// Tesseract language codes joined by `+`, for example `eng+hin`.
    pub langs: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TranslateConfig {
    /// `google`, `libretranslate` or `ollama`.
    pub engine: String,
    /// Target language code, for example `en` or `hi`.
    pub target: String,
    pub libretranslate_url: String,
    pub libretranslate_key: String,
    /// Ollama text model used when `engine = "ollama"`.
    pub ollama_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OllamaConfig {
    /// Show the "Ask local AI" button. Off until local models are set up.
    pub enabled: bool,
    pub url: String,
    pub vision_model: String,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LensConfig {
    pub enabled: bool,
    /// Interface language for the Google Lens result page.
    pub hl: String,
    /// Browser for the results side panel: `auto` picks the first installed
    /// Chromium-family browser (needed for the app-window panel).
    pub browser: String,
}

impl Default for OcrConfig {
    fn default() -> Self {
        Self {
            langs: "eng".into(),
        }
    }
}

impl Default for TranslateConfig {
    fn default() -> Self {
        Self {
            engine: "google".into(),
            target: "en".into(),
            libretranslate_url: "https://libretranslate.com".into(),
            libretranslate_key: String::new(),
            ollama_model: "gemma4:e2b".into(),
        }
    }
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: "http://localhost:11434".into(),
            vision_model: "granite3.2-vision:2b".into(),
            prompt: "Describe this image. Identify the main objects, any products, \
                     landmarks, plants or animals, and read any visible text."
                .into(),
        }
    }
}

impl Default for LensConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            hl: "en".into(),
            browser: "auto".into(),
        }
    }
}

pub fn path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "rlens").map(|d| d.config_dir().join("config.toml"))
}

impl Config {
    /// Load the config. Write the defaults on first run.
    pub fn load() -> Self {
        match Self::try_load() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("config: {e:#}; using defaults");
                Self::default()
            }
        }
    }

    fn try_load() -> Result<Self> {
        let Some(path) = path() else {
            return Ok(Self::default());
        };
        if !path.exists() {
            let cfg = Self::default();
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, toml::to_string_pretty(&cfg)?)
                .with_context(|| format!("write {}", path.display()))?;
            return Ok(cfg);
        }
        let text = std::fs::read_to_string(&path)?;
        toml::from_str(&text).with_context(|| format!("parse {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_round_trip() {
        let cfg = Config::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn partial_file_uses_defaults() {
        let cfg: Config = toml::from_str("[translate]\ntarget = \"hi\"\n").unwrap();
        assert_eq!(cfg.translate.target, "hi");
        assert_eq!(cfg.translate.engine, "google");
        assert_eq!(cfg.ocr.langs, "eng");
    }
}
