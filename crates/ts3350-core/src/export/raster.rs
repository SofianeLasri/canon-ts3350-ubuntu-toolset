use image::codecs::avif::AvifEncoder;
use image::codecs::jpeg::{JpegEncoder, PixelDensity};
use image::codecs::png::{CompressionType, FilterType, PngEncoder};

use super::{ExportFormat, ExportOptions, PngCompression, ScannedPage, prepare};
use crate::i18n::i18n;
use crate::{Error, Result};

/// Encodes a page as PNG, JPEG or AVIF, with its rotation applied.
pub fn encode_raster(page: &ScannedPage, options: &ExportOptions) -> Result<Vec<u8>> {
    let (image, dpi) = prepare(page, options.max_dpi)?;
    let image = page.rotation.apply(image);
    let quality = options.quality.clamp(1, 100);
    let mut out = Vec::new();
    match options.format {
        ExportFormat::Png => {
            let compression = match options.png_compression {
                PngCompression::Fast => CompressionType::Fast,
                PngCompression::Default => CompressionType::Default,
                PngCompression::Best => CompressionType::Best,
            };
            image.write_with_encoder(PngEncoder::new_with_quality(&mut out, compression, FilterType::Adaptive))?;
        }
        ExportFormat::Jpeg => {
            let mut encoder = JpegEncoder::new_with_quality(&mut out, quality);
            encoder.set_pixel_density(PixelDensity::dpi(dpi.min(u16::MAX as u32) as u16));
            image.write_with_encoder(encoder)?;
        }
        ExportFormat::Avif => {
            image.write_with_encoder(AvifEncoder::new_with_speed_quality(
                &mut out,
                options.avif_speed.clamp(1, 10),
                quality,
            ))?;
        }
        ExportFormat::Pdf => return Err(Error::Other(i18n("PDF is not an image format").into())),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::Rotation;
    use crate::export::tests::sample_jpeg;
    use image::ImageFormat;

    fn options(format: ExportFormat) -> ExportOptions {
        ExportOptions { format, quality: 60, avif_speed: 10, ..Default::default() }
    }

    #[test]
    fn encodes_every_raster_format() {
        let mut page = ScannedPage::new(sample_jpeg(64, 32, false), 300);
        page.rotation = Rotation::R90;
        for (format, expected) in [
            (ExportFormat::Png, ImageFormat::Png),
            (ExportFormat::Jpeg, ImageFormat::Jpeg),
            (ExportFormat::Avif, ImageFormat::Avif),
        ] {
            let bytes = encode_raster(&page, &options(format)).unwrap();
            assert_eq!(image::guess_format(&bytes).unwrap(), expected, "{format:?}");
            if format != ExportFormat::Avif {
                // A 90° rotation swaps width and height.
                let img = image::load_from_memory(&bytes).unwrap();
                assert_eq!((img.width(), img.height()), (32, 64));
            }
        }
    }

    #[test]
    fn grayscale_pages_encode() {
        let page = ScannedPage::new(sample_jpeg(16, 16, true), 150);
        for format in [ExportFormat::Png, ExportFormat::Jpeg, ExportFormat::Avif] {
            assert!(!encode_raster(&page, &options(format)).unwrap().is_empty());
        }
    }
}
