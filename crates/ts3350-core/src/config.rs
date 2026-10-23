//! User preferences (`~/.config/ts3350-toolset/config.toml`).

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::duplex::DuplexProfile;
use crate::escl::{ColorMode, Paper, Region};
use crate::export::ExportOptions;
use crate::i18n::i18n;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanPrefs {
    pub color_mode: ColorMode,
    pub resolution: u32,
    pub intent: Option<String>,
    pub content_type: Option<String>,
    pub paper: Paper,
    pub custom_region: Option<Region>,
    pub sharpen: bool,
    pub background_reduction: bool,
    pub high_quality: bool,
}

impl Default for ScanPrefs {
    fn default() -> Self {
        ScanPrefs {
            color_mode: ColorMode::Rgb24,
            resolution: 300,
            intent: Some("Document".into()),
            content_type: None,
            paper: Paper::A4,
            custom_region: None,
            sharpen: false,
            background_reduction: false,
            high_quality: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// eSCL URL entered by hand; otherwise mDNS discovery.
    pub scanner_url: Option<String>,
    /// Last scanner used (preselected on startup).
    pub last_scanner_url: Option<String>,
    /// Real CUPS queue; otherwise automatic detection.
    pub target_queue: Option<String>,
    pub duplex: DuplexProfile,
    pub scan: ScanPrefs,
    pub export: ExportOptions,
    pub last_export_dir: Option<PathBuf>,
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        directories::BaseDirs::new().map(|d| d.config_dir().join("ts3350-toolset").join("config.toml"))
    }

    /// Loads the configuration; a missing or unreadable file yields the defaults.
    pub fn load() -> Config {
        Config::path()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = Config::path().ok_or_else(|| Error::Config(i18n("configuration folder not found").into()))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))?;
        fs::write(path, text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let mut config = Config::default();
        config.scan.paper = Paper::Custom;
        config.scan.custom_region = Some(Region { x: 10, y: 20, width: 300, height: 400 });
        config.duplex.reverse_backs = true;
        let text = toml::to_string_pretty(&config).unwrap();
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), config);
        // A partial file keeps the defaults for everything else.
        let partial: Config = toml::from_str("target_queue = \"X\"\n").unwrap();
        assert_eq!(partial.scan, ScanPrefs::default());
    }
}
