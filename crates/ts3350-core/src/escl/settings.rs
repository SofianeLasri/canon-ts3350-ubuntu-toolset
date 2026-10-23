//! Scan request (`ScanSettings`) and paper sizes.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use super::caps::{ColorMode, InputCaps, UNITS_PER_INCH};
use crate::i18n::i18n;

/// Scan region in 1/300 inch, from the alignment corner of the platen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    pub fn full(caps: &InputCaps) -> Region {
        Region { x: 0, y: 0, width: caps.max_width, height: caps.max_height }
    }

    /// Brings the region back within the platen.
    pub fn clamp_to(self, caps: &InputCaps) -> Region {
        let x = self.x.min(caps.max_width.saturating_sub(caps.min_width));
        let y = self.y.min(caps.max_height.saturating_sub(caps.min_height));
        Region {
            x,
            y,
            width: self.width.clamp(caps.min_width, caps.max_width - x),
            height: self.height.clamp(caps.min_height, caps.max_height - y),
        }
    }

    /// Size in pixels of the image produced at this resolution.
    pub fn pixels(&self, dpi: u32) -> (u32, u32) {
        (self.width * dpi / UNITS_PER_INCH, self.height * dpi / UNITS_PER_INCH)
    }
}

/// Paper sizes offered in the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Paper {
    #[default]
    A4,
    A5,
    A6,
    Letter,
    Photo10x15,
    Photo13x18,
    FullPlaten,
    Custom,
}

impl Paper {
    pub const ALL: [Paper; 8] = [
        Paper::A4,
        Paper::A5,
        Paper::A6,
        Paper::Letter,
        Paper::Photo10x15,
        Paper::Photo13x18,
        Paper::FullPlaten,
        Paper::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Paper::A4 => "A4 (210 × 297 mm)",
            Paper::A5 => "A5 (148 × 210 mm)",
            Paper::A6 => "A6 (105 × 148 mm)",
            Paper::Letter => i18n("US Letter (8.5 × 11 in)"),
            Paper::Photo10x15 => i18n("Photo 10 × 15 cm"),
            Paper::Photo13x18 => i18n("Photo 13 × 18 cm"),
            Paper::FullPlaten => i18n("Whole platen"),
            Paper::Custom => i18n("Custom area"),
        }
    }

    /// Matching region, or `None` for a custom area.
    pub fn region(self, caps: &InputCaps) -> Option<Region> {
        let mm = |w: f64, h: f64| Region {
            x: 0,
            y: 0,
            width: (w / 25.4 * UNITS_PER_INCH as f64).round() as u32,
            height: (h / 25.4 * UNITS_PER_INCH as f64).round() as u32,
        };
        let region = match self {
            Paper::A4 => mm(210.0, 297.0),
            Paper::A5 => mm(148.0, 210.0),
            Paper::A6 => mm(105.0, 148.0),
            Paper::Letter => Region { x: 0, y: 0, width: 2550, height: 3300 },
            Paper::Photo10x15 => mm(100.0, 150.0),
            Paper::Photo13x18 => mm(130.0, 180.0),
            Paper::FullPlaten => Region::full(caps),
            Paper::Custom => return None,
        };
        Some(region.clamp_to(caps))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScanSettings {
    pub color_mode: ColorMode,
    pub resolution: u32,
    pub intent: Option<String>,
    pub content_type: Option<String>,
    pub region: Region,
    pub sharpen: Option<i32>,
    pub compression: Option<i32>,
    pub background_reduction: bool,
}

impl ScanSettings {
    /// XML body of `POST /eSCL/ScanJobs`.
    pub fn to_xml(&self, version: &str) -> String {
        let mut xml = String::new();
        xml.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
        xml.push('\n');
        xml.push_str(
            r#"<scan:ScanSettings xmlns:scan="http://schemas.hp.com/imaging/escl/2011/05/03" xmlns:pwg="http://www.pwg.org/schemas/2010/12/sm">"#,
        );
        let _ = write!(xml, "<pwg:Version>{}</pwg:Version>", escape(version));
        if let Some(intent) = &self.intent {
            let _ = write!(xml, "<scan:Intent>{}</scan:Intent>", escape(intent));
        }
        let r = self.region;
        let _ = write!(
            xml,
            "<pwg:ScanRegions pwg:MustHonor=\"true\"><pwg:ScanRegion>\
             <pwg:ContentRegionUnits>escl:ThreeHundredthsOfInches</pwg:ContentRegionUnits>\
             <pwg:XOffset>{}</pwg:XOffset><pwg:YOffset>{}</pwg:YOffset>\
             <pwg:Width>{}</pwg:Width><pwg:Height>{}</pwg:Height>\
             </pwg:ScanRegion></pwg:ScanRegions>",
            r.x, r.y, r.width, r.height
        );
        xml.push_str("<pwg:InputSource>Platen</pwg:InputSource>");
        if let Some(content_type) = &self.content_type {
            let _ = write!(xml, "<pwg:ContentType>{}</pwg:ContentType>", escape(content_type));
        }
        let _ = write!(xml, "<scan:ColorMode>{}</scan:ColorMode>", self.color_mode.as_escl());
        xml.push_str("<pwg:DocumentFormat>image/jpeg</pwg:DocumentFormat>");
        xml.push_str("<scan:DocumentFormatExt>image/jpeg</scan:DocumentFormatExt>");
        let _ = write!(
            xml,
            "<scan:XResolution>{0}</scan:XResolution><scan:YResolution>{0}</scan:YResolution>",
            self.resolution
        );
        if let Some(sharpen) = self.sharpen {
            let _ = write!(xml, "<scan:Sharpen>{sharpen}</scan:Sharpen>");
        }
        if let Some(compression) = self.compression {
            let _ = write!(xml, "<scan:CompressionFactor>{compression}</scan:CompressionFactor>");
        }
        if self.background_reduction {
            xml.push_str("<scan:MediaType>BackgroundReduction</scan:MediaType>");
        }
        xml.push_str("</scan:ScanSettings>");
        xml
    }
}

fn escape(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::escl::caps::Capabilities;
    use crate::escl::xml::Element;

    fn platen() -> InputCaps {
        Capabilities::parse(include_str!("../../tests/fixtures/ts3300_caps.xml")).unwrap().platen.unwrap()
    }

    #[test]
    fn a4_region_fits_the_platen() {
        let caps = platen();
        let a4 = Paper::A4.region(&caps).unwrap();
        assert_eq!((a4.width, a4.height), (2480, 3508));
        assert_eq!(a4.pixels(300), (2480, 3508));
        assert_eq!(Paper::FullPlaten.region(&caps).unwrap(), Region::full(&caps));
        assert!(Paper::Custom.region(&caps).is_none());
    }

    #[test]
    fn region_is_clamped() {
        let caps = platen();
        let r = Region { x: 2500, y: 0, width: 500, height: 99999 }.clamp_to(&caps);
        assert!(r.x + r.width <= caps.max_width);
        assert_eq!(r.height, caps.max_height);
    }

    #[test]
    fn settings_xml_is_well_formed() {
        let settings = ScanSettings {
            color_mode: ColorMode::Grayscale8,
            resolution: 150,
            intent: Some("Document".into()),
            content_type: Some("Text".into()),
            region: Region { x: 0, y: 0, width: 2480, height: 3508 },
            sharpen: Some(1),
            compression: None,
            background_reduction: true,
        };
        let xml = settings.to_xml("2.63");
        let root = Element::parse(&xml).unwrap();
        assert_eq!(root.name, "ScanSettings");
        assert_eq!(root.child_text("ColorMode"), Some("Grayscale8"));
        assert_eq!(root.child_u32("XResolution"), Some(150));
        assert_eq!(root.child_text("Intent"), Some("Document"));
        assert_eq!(root.child_text("MediaType"), Some("BackgroundReduction"));
        assert!(root.child("CompressionFactor").is_none());
        let region = root.path(&["ScanRegions", "ScanRegion"]).unwrap();
        assert_eq!(region.child_u32("Width"), Some(2480));
    }
}
