//! Export of scanned pages: multi-page PDF, PNG, JPEG or AVIF.

mod pdf;
mod raster;

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::codecs::jpeg::JpegDecoder;
use image::imageops::FilterType;
use image::{ColorType, DynamicImage, ImageDecoder, ImageFormat};
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::i18n::i18n;

pub use pdf::write_pdf;
pub use raster::encode_raster;

/// Clockwise rotation applied to a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Rotation {
    #[default]
    R0,
    R90,
    R180,
    R270,
}

impl Rotation {
    pub fn degrees(self) -> u16 {
        match self {
            Rotation::R0 => 0,
            Rotation::R90 => 90,
            Rotation::R180 => 180,
            Rotation::R270 => 270,
        }
    }

    pub fn clockwise(self) -> Rotation {
        match self {
            Rotation::R0 => Rotation::R90,
            Rotation::R90 => Rotation::R180,
            Rotation::R180 => Rotation::R270,
            Rotation::R270 => Rotation::R0,
        }
    }

    pub fn apply(self, image: DynamicImage) -> DynamicImage {
        match self {
            Rotation::R0 => image,
            Rotation::R90 => image.rotate90(),
            Rotation::R180 => image.rotate180(),
            Rotation::R270 => image.rotate270(),
        }
    }
}

/// A page as returned by the scanner: the original JPEG is never modified.
#[derive(Debug, Clone)]
pub struct ScannedPage {
    pub jpeg: Arc<Vec<u8>>,
    pub dpi: u32,
    pub rotation: Rotation,
}

impl ScannedPage {
    pub fn new(jpeg: Vec<u8>, dpi: u32) -> ScannedPage {
        ScannedPage { jpeg: Arc::new(jpeg), dpi, rotation: Rotation::R0 }
    }

    pub fn decode(&self) -> Result<DynamicImage> {
        Ok(image::load_from_memory_with_format(&self.jpeg, ImageFormat::Jpeg)?)
    }

    /// RGBA thumbnail, already rotated, fitting in a `max` pixel square.
    pub fn thumbnail(&self, max: u32) -> Result<image::RgbaImage> {
        Ok(self.rotation.apply(self.decode()?.thumbnail(max, max)).to_rgba8())
    }
}

/// Width, height and grayscale flag of a JPEG, without decoding it.
pub fn jpeg_info(jpeg: &[u8]) -> Result<(u32, u32, bool)> {
    let decoder = JpegDecoder::new(Cursor::new(jpeg))?;
    let (width, height) = decoder.dimensions();
    let gray = matches!(decoder.color_type(), ColorType::L8 | ColorType::L16);
    Ok((width, height, gray))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ExportFormat {
    #[default]
    Pdf,
    Png,
    Jpeg,
    Avif,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 4] = [ExportFormat::Pdf, ExportFormat::Png, ExportFormat::Jpeg, ExportFormat::Avif];

    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Pdf => "pdf",
            ExportFormat::Png => "png",
            ExportFormat::Jpeg => "jpg",
            ExportFormat::Avif => "avif",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Pdf => "PDF",
            ExportFormat::Png => i18n("PNG (lossless)"),
            ExportFormat::Jpeg => "JPEG",
            ExportFormat::Avif => "AVIF",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            ExportFormat::Pdf => "application/pdf",
            ExportFormat::Png => "image/png",
            ExportFormat::Jpeg => "image/jpeg",
            ExportFormat::Avif => "image/avif",
        }
    }

    /// Image formats produce one file per page.
    pub fn is_multipage(self) -> bool {
        self == ExportFormat::Pdf
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PngCompression {
    Fast,
    #[default]
    Default,
    Best,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportOptions {
    pub format: ExportFormat,
    /// Quality from 1 to 100 (JPEG, AVIF, and PDF when re-encoding).
    pub quality: u8,
    /// PDF: embed the scanner JPEG as is, without re-encoding.
    pub keep_original: bool,
    /// Downscales pages whose resolution exceeds this value.
    pub max_dpi: Option<u32>,
    pub png_compression: PngCompression,
    /// AVIF encoder speed (1 = slow and compact, 10 = fast).
    pub avif_speed: u8,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            format: ExportFormat::Pdf,
            quality: 85,
            keep_original: true,
            max_dpi: None,
            png_compression: PngCompression::Default,
            avif_speed: 6,
        }
    }
}

/// Decodes, downscales if needed, and returns the image with its effective resolution.
fn prepare(page: &ScannedPage, max_dpi: Option<u32>) -> Result<(DynamicImage, u32)> {
    let image = page.decode()?;
    match max_dpi {
        Some(max) if max < page.dpi => {
            let width = (image.width() as u64 * max as u64 / page.dpi as u64).max(1) as u32;
            let height = (image.height() as u64 * max as u64 / page.dpi as u64).max(1) as u32;
            Ok((image.resize_exact(width, height, FilterType::Lanczos3), max))
        }
        _ => Ok((image, page.dpi)),
    }
}

/// Output paths: `doc.png` for one page, otherwise `doc-01.png`, `doc-02.png`…
pub fn output_paths(path: &Path, format: ExportFormat, pages: usize) -> Vec<PathBuf> {
    let path = path.with_extension(format.extension());
    if format.is_multipage() || pages <= 1 {
        return vec![path];
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let width = pages.to_string().len().max(2);
    (1..=pages).map(|i| path.with_file_name(format!("{stem}-{i:0width$}.{}", format.extension()))).collect()
}

/// Writes the document and returns the files created.
pub fn export(pages: &[ScannedPage], options: &ExportOptions, path: &Path, title: &str) -> Result<Vec<PathBuf>> {
    let paths = output_paths(path, options.format, pages.len());
    if options.format == ExportFormat::Pdf {
        let mut file = std::io::BufWriter::new(std::fs::File::create(&paths[0])?);
        write_pdf(pages, options, title, &mut file)?;
        std::io::Write::flush(&mut file)?;
    } else {
        for (page, out) in pages.iter().zip(&paths) {
            std::fs::write(out, encode_raster(page, options)?)?;
        }
    }
    Ok(paths)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use image::codecs::jpeg::JpegEncoder;
    use image::{GrayImage, RgbImage};

    /// Synthetic JPEG of `width`×`height` pixels.
    pub fn sample_jpeg(width: u32, height: u32, gray: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let encoder = JpegEncoder::new_with_quality(&mut out, 90);
        if gray {
            let img = GrayImage::from_fn(width, height, |x, y| image::Luma([((x + y) % 256) as u8]));
            DynamicImage::ImageLuma8(img).write_with_encoder(encoder).unwrap();
        } else {
            let img = RgbImage::from_fn(width, height, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 128]));
            DynamicImage::ImageRgb8(img).write_with_encoder(encoder).unwrap();
        }
        out
    }

    #[test]
    fn jpeg_info_reads_header() {
        assert_eq!(jpeg_info(&sample_jpeg(40, 20, false)).unwrap(), (40, 20, false));
        assert_eq!(jpeg_info(&sample_jpeg(8, 8, true)).unwrap(), (8, 8, true));
    }

    #[test]
    fn output_paths_are_numbered() {
        let base = Path::new("/tmp/scan.pdf");
        assert_eq!(output_paths(base, ExportFormat::Pdf, 3), vec![PathBuf::from("/tmp/scan.pdf")]);
        assert_eq!(output_paths(base, ExportFormat::Png, 1), vec![PathBuf::from("/tmp/scan.png")]);
        assert_eq!(
            output_paths(base, ExportFormat::Avif, 2),
            vec![PathBuf::from("/tmp/scan-01.avif"), PathBuf::from("/tmp/scan-02.avif")]
        );
    }

    #[test]
    fn rotation_cycles() {
        let mut r = Rotation::R0;
        for expected in [90, 180, 270, 0] {
            r = r.clockwise();
            assert_eq!(r.degrees(), expected);
        }
    }

    #[test]
    fn prepare_downscales() {
        let page = ScannedPage::new(sample_jpeg(600, 300, false), 600);
        let (img, dpi) = prepare(&page, Some(150)).unwrap();
        assert_eq!((img.width(), img.height(), dpi), (150, 75, 150));
        let (img, dpi) = prepare(&page, Some(1200)).unwrap();
        assert_eq!((img.width(), dpi), (600, 600));
    }
}
