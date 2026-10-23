//! Capabilities advertised by the scanner (`/eSCL/ScannerCapabilities`).

use serde::{Deserialize, Serialize};

use super::xml::Element;
use crate::Result;
use crate::i18n::i18n;

/// eSCL unit for scan regions: 1/300 inch.
pub const UNITS_PER_INCH: u32 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ColorMode {
    #[default]
    Rgb24,
    Grayscale8,
}

impl ColorMode {
    pub fn as_escl(self) -> &'static str {
        match self {
            ColorMode::Rgb24 => "RGB24",
            ColorMode::Grayscale8 => "Grayscale8",
        }
    }

    pub fn from_escl(value: &str) -> Option<ColorMode> {
        match value {
            "RGB24" => Some(ColorMode::Rgb24),
            "Grayscale8" => Some(ColorMode::Grayscale8),
            // BlackAndWhite1 cannot be carried as JPEG: ignore it.
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ColorMode::Rgb24 => i18n("Color"),
            ColorMode::Grayscale8 => i18n("Grayscale"),
        }
    }
}

/// Range of a numeric setting (sharpness, compression…).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub min: i32,
    pub max: i32,
    pub normal: i32,
    pub step: i32,
}

impl Range {
    fn parse(el: &Element) -> Option<Range> {
        Some(Range {
            min: el.child_i32("Min")?,
            max: el.child_i32("Max")?,
            normal: el.child_i32("Normal").or(el.child_i32("Min"))?,
            step: el.child_i32("Step").unwrap_or(1).max(1),
        })
    }
}

/// Capabilities of an input source (platen or feeder).
#[derive(Debug, Clone, PartialEq)]
pub struct InputCaps {
    pub min_width: u32,
    pub max_width: u32,
    pub min_height: u32,
    pub max_height: u32,
    pub color_modes: Vec<ColorMode>,
    pub default_color_mode: Option<ColorMode>,
    pub content_types: Vec<String>,
    pub document_formats: Vec<String>,
    pub resolutions: Vec<u32>,
    pub default_resolution: Option<u32>,
    pub intents: Vec<String>,
    pub max_optical_resolution: Option<u32>,
}

impl InputCaps {
    fn parse(el: &Element) -> InputCaps {
        let mut color_modes = Vec::new();
        let mut default_color_mode = None;
        let mut content_types = Vec::new();
        let mut document_formats = Vec::new();
        let mut resolutions = Vec::new();
        let mut default_resolution = None;

        let mut profiles = Vec::new();
        el.descendants("SettingProfile", &mut profiles);
        for profile in profiles {
            let mut modes = Vec::new();
            profile.descendants("ColorMode", &mut modes);
            for mode in modes {
                if let Some(m) = ColorMode::from_escl(mode.text()) {
                    if !color_modes.contains(&m) {
                        color_modes.push(m);
                    }
                    if mode.attr("default") == Some("true") {
                        default_color_mode = Some(m);
                    }
                }
            }

            let mut types = Vec::new();
            profile.descendants("ContentType", &mut types);
            for t in types {
                push_unique(&mut content_types, t.text());
            }

            let mut formats = Vec::new();
            profile.descendants("DocumentFormatExt", &mut formats);
            if formats.is_empty() {
                profile.descendants("DocumentFormat", &mut formats);
            }
            for f in formats {
                push_unique(&mut document_formats, f.text());
            }

            let mut discrete = Vec::new();
            profile.descendants("DiscreteResolution", &mut discrete);
            for res in discrete {
                if let (Some(x), Some(y)) = (res.child("XResolution"), res.child("YResolution")) {
                    // Only square resolutions are of interest.
                    if let (Ok(xv), Ok(yv)) = (x.text().parse::<u32>(), y.text().parse::<u32>())
                        && xv == yv
                    {
                        if !resolutions.contains(&xv) {
                            resolutions.push(xv);
                        }
                        if x.attr("default") == Some("true") {
                            default_resolution = Some(xv);
                        }
                    }
                }
            }

            if let Some(range) = profile.descendant("XResolutionRange").and_then(Range::parse) {
                for standard in [75, 100, 150, 200, 300, 400, 600, 1200] {
                    if (range.min..=range.max).contains(&(standard as i32)) && !resolutions.contains(&standard) {
                        resolutions.push(standard);
                    }
                }
            }
        }
        resolutions.sort_unstable();

        let mut intents = Vec::new();
        if let Some(supported) = el.descendant("SupportedIntents") {
            for intent in supported.children_named("Intent") {
                push_unique(&mut intents, intent.text());
            }
        }

        InputCaps {
            min_width: el.child_u32("MinWidth").unwrap_or(1),
            max_width: el.child_u32("MaxWidth").unwrap_or(2550),
            min_height: el.child_u32("MinHeight").unwrap_or(1),
            max_height: el.child_u32("MaxHeight").unwrap_or(3508),
            color_modes,
            default_color_mode,
            content_types,
            document_formats,
            resolutions,
            default_resolution,
            intents,
            max_optical_resolution: el.child_u32("MaxOpticalXResolution"),
        }
    }

    pub fn supports_jpeg(&self) -> bool {
        self.document_formats.iter().any(|f| f == "image/jpeg")
    }
}

fn push_unique(list: &mut Vec<String>, value: &str) {
    if !value.is_empty() && !list.iter().any(|v| v == value) {
        list.push(value.to_owned());
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Capabilities {
    pub version: String,
    pub make_and_model: String,
    pub platen: Option<InputCaps>,
    pub adf: Option<InputCaps>,
    pub sharpen: Option<Range>,
    pub compression: Option<Range>,
    pub background_reduction: bool,
}

impl Capabilities {
    pub fn parse(xml: &str) -> Result<Capabilities> {
        let root = Element::parse(xml)?;
        let platen = root.path(&["Platen", "PlatenInputCaps"]).map(InputCaps::parse);
        let adf = root.child("Adf").and_then(|adf| adf.child("AdfSimplexInputCaps")).map(InputCaps::parse);
        let background_reduction = root
            .child("SupportedMediaTypes")
            .is_some_and(|m| m.children_named("MediaType").any(|t| t.text() == "BackgroundReduction"));

        Ok(Capabilities {
            version: root.child_text("Version").unwrap_or("2.0").to_owned(),
            make_and_model: root.child_text("MakeAndModel").unwrap_or("Scanner eSCL").to_owned(),
            platen,
            adf,
            sharpen: root.child("SharpenSupport").and_then(Range::parse),
            compression: root.child("CompressionFactorSupport").and_then(Range::parse),
            background_reduction,
        })
    }

    /// Source used by the application: the platen, otherwise the feeder.
    pub fn input(&self) -> Option<&InputCaps> {
        self.platen.as_ref().or(self.adf.as_ref())
    }
}

/// Display label of an eSCL intent.
pub fn intent_label(intent: &str) -> String {
    match intent {
        "Document" => i18n("Document").into(),
        "TextAndGraphic" => i18n("Text and graphics").into(),
        "Photo" => i18n("Photo").into(),
        "Preview" => i18n("Preview").into(),
        "Auto" => i18n("Automatic").into(),
        "Object" => i18n("3D object").into(),
        "BusinessCard" => i18n("Business card").into(),
        other => other.into(),
    }
}

/// Display label of an eSCL content type.
pub fn content_type_label(content_type: &str) -> String {
    match content_type {
        "Photo" => i18n("Photo").into(),
        "Text" => i18n("Text").into(),
        "TextAndPhoto" => i18n("Text and photo").into(),
        "LineArt" => i18n("Line art").into(),
        "Magazine" => i18n("Magazine").into(),
        "Halftone" => i18n("Halftone (printed matter)").into(),
        "Thru" => i18n("Raw (no processing)").into(),
        "Auto" => i18n("Automatic").into(),
        other => other.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TS3300: &str = include_str!("../../tests/fixtures/ts3300_caps.xml");

    #[test]
    fn parses_ts3300_capabilities() {
        let caps = Capabilities::parse(TS3300).unwrap();
        assert_eq!(caps.make_and_model, "Canon TS3300 series");
        assert_eq!(caps.version, "2.63");
        assert!(caps.adf.is_none());
        let platen = caps.platen.as_ref().unwrap();
        assert_eq!(platen.max_width, 2550);
        assert_eq!(platen.max_height, 3508);
        assert_eq!(platen.color_modes, vec![ColorMode::Grayscale8, ColorMode::Rgb24]);
        assert_eq!(platen.default_color_mode, Some(ColorMode::Rgb24));
        assert_eq!(platen.resolutions, vec![75, 100, 150, 200, 300, 600]);
        assert_eq!(platen.default_resolution, Some(300));
        assert!(platen.intents.iter().any(|i| i == "Preview"));
        assert!(platen.content_types.iter().any(|c| c == "TextAndPhoto"));
        assert!(platen.supports_jpeg());
        assert_eq!(caps.sharpen, Some(Range { min: 0, max: 1, normal: 0, step: 1 }));
        assert_eq!(caps.compression, Some(Range { min: 0, max: 1, normal: 1, step: 1 }));
        assert!(caps.background_reduction);
    }
}
