use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use image::DynamicImage;
use image::codecs::jpeg::JpegEncoder;
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};

use super::{ExportOptions, ScannedPage, jpeg_info, prepare};
use crate::Result;

/// JPEG to embed in the PDF, with its dimensions.
struct EmbeddedJpeg {
    data: Vec<u8>,
    width: u32,
    height: u32,
    gray: bool,
    dpi: u32,
}

fn embedded_jpeg(page: &ScannedPage, options: &ExportOptions) -> Result<EmbeddedJpeg> {
    let downscale = options.max_dpi.is_some_and(|max| max < page.dpi);
    if options.keep_original && !downscale {
        let (width, height, gray) = jpeg_info(&page.jpeg)?;
        return Ok(EmbeddedJpeg { data: page.jpeg.as_ref().clone(), width, height, gray, dpi: page.dpi });
    }

    let (image, dpi) = prepare(page, options.max_dpi)?;
    let gray = matches!(image, DynamicImage::ImageLuma8(_) | DynamicImage::ImageLuma16(_));
    let image =
        if gray { DynamicImage::ImageLuma8(image.to_luma8()) } else { DynamicImage::ImageRgb8(image.to_rgb8()) };
    let mut data = Vec::new();
    image.write_with_encoder(JpegEncoder::new_with_quality(&mut data, options.quality.clamp(1, 100)))?;
    Ok(EmbeddedJpeg { data, width: image.width(), height: image.height(), gray, dpi })
}

/// Writes a PDF where each page is a JPEG image at its real size.
///
/// Rotation uses the `/Rotate` attribute, so no re-encoding is needed.
pub fn write_pdf<W: Write>(pages: &[ScannedPage], options: &ExportOptions, title: &str, out: &mut W) -> Result<()> {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let mut kids = Vec::with_capacity(pages.len());

    for page in pages {
        let jpeg = embedded_jpeg(page, options)?;
        let image = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => jpeg.width as i64,
                "Height" => jpeg.height as i64,
                "ColorSpace" => if jpeg.gray { "DeviceGray" } else { "DeviceRGB" },
                "BitsPerComponent" => 8,
                "Filter" => "DCTDecode",
            },
            jpeg.data,
        )
        .with_compression(false);
        let image_id = doc.add_object(image);

        let width_pt = jpeg.width as f32 * 72.0 / jpeg.dpi as f32;
        let height_pt = jpeg.height as f32 * 72.0 / jpeg.dpi as f32;
        let content = Content {
            operations: vec![
                Operation::new("q", vec![]),
                Operation::new("cm", vec![width_pt.into(), 0.into(), 0.into(), height_pt.into(), 0.into(), 0.into()]),
                Operation::new("Do", vec!["Im0".into()]),
                Operation::new("Q", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode()?));

        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), width_pt.into(), height_pt.into()],
            "Contents" => content_id,
            "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => image_id } },
            "Rotate" => page.rotation.degrees() as i64,
        });
        kids.push(Object::Reference(page_id));
    }

    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => pages.len() as i64,
        }),
    );
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    let info_id = doc.add_object(dictionary! {
        "Title" => Object::string_literal(title.as_bytes().to_vec()),
        "Producer" => Object::string_literal("Canon TS3350 Toolset"),
        "CreationDate" => Object::string_literal(pdf_date_now()),
    });
    doc.trailer.set("Root", catalog_id);
    doc.trailer.set("Info", info_id);
    doc.compress();
    doc.save_to(out)?;
    Ok(())
}

/// PDF date (`D:YYYYMMDDHHmmSSZ`) in UTC.
pub(crate) fn pdf_date_now() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Howard Hinnant's "civil from days" algorithm.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("D:{year:04}{month:02}{day:02}{:02}{:02}{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::Rotation;
    use crate::export::tests::sample_jpeg;

    fn reload(pages: &[ScannedPage], options: &ExportOptions) -> Document {
        let mut bytes = Vec::new();
        write_pdf(pages, options, "Test", &mut bytes).unwrap();
        Document::load_mem(&bytes).unwrap()
    }

    #[test]
    fn writes_one_page_per_scan() {
        let mut second = ScannedPage::new(sample_jpeg(150, 300, true), 150);
        second.rotation = Rotation::R90;
        let pages = vec![ScannedPage::new(sample_jpeg(300, 600, false), 300), second];
        let doc = reload(&pages, &ExportOptions::default());
        let ids: Vec<_> = doc.get_pages().into_values().collect();
        assert_eq!(ids.len(), 2);

        let first = doc.get_dictionary(ids[0]).unwrap();
        let media: Vec<f32> =
            first.get(b"MediaBox").unwrap().as_array().unwrap().iter().map(|o| o.as_float().unwrap()).collect();
        assert_eq!(media, vec![0.0, 0.0, 72.0, 144.0]);

        let second = doc.get_dictionary(ids[1]).unwrap();
        assert_eq!(second.get(b"Rotate").unwrap().as_i64().unwrap(), 90);
        let xobjects = second
            .get(b"Resources")
            .and_then(Object::as_dict)
            .and_then(|r| r.get(b"XObject"))
            .and_then(Object::as_dict)
            .unwrap();
        let image_id = xobjects.get(b"Im0").unwrap().as_reference().unwrap();
        let image = doc.get_object(image_id).unwrap().as_stream().unwrap();
        assert_eq!(image.dict.get(b"Filter").unwrap().as_name().unwrap(), b"DCTDecode");
        assert_eq!(image.dict.get(b"ColorSpace").unwrap().as_name().unwrap(), b"DeviceGray");
    }

    #[test]
    fn keeps_original_jpeg_bytes() {
        let jpeg = sample_jpeg(30, 30, false);
        let pages = vec![ScannedPage::new(jpeg.clone(), 300)];
        let doc = reload(&pages, &ExportOptions::default());
        let found = doc.objects.values().any(|o| matches!(o, Object::Stream(s) if s.content == jpeg));
        assert!(found);
    }

    #[test]
    fn reencodes_and_downscales() {
        let pages = vec![ScannedPage::new(sample_jpeg(600, 600, false), 600)];
        let options = ExportOptions { keep_original: false, max_dpi: Some(150), quality: 50, ..Default::default() };
        let doc = reload(&pages, &options);
        let image = doc
            .objects
            .values()
            .find_map(|o| match o {
                Object::Stream(s) if s.dict.get(b"Subtype").ok().and_then(|v| v.as_name().ok()) == Some(b"Image") => {
                    Some(s)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(image.dict.get(b"Width").unwrap().as_i64().unwrap(), 150);
    }

    #[test]
    fn pdf_date_format() {
        let date = pdf_date_now();
        assert!(date.starts_with("D:20"));
        assert_eq!(date.len(), 17);
    }
}
