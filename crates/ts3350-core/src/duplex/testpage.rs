//! Calibration document: 2 sheets, each side shows its number and a "TOP" arrow.

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};

use crate::Result;
use crate::i18n::{i18n, i18n_f};

const WIDTH: f32 = 595.276;
const HEIGHT: f32 = 841.89;

/// Encodes text for the standard Helvetica font (WinAnsiEncoding matches Latin-1 for letters).
fn win_ansi(value: &str) -> Vec<u8> {
    value.chars().map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?')).collect()
}

fn text(ops: &mut Vec<Operation>, size: i64, y: f32, value: &str) {
    // Rough centring: Helvetica-Bold capitals are about 0.6 em wide.
    let x = (WIDTH - value.chars().count() as f32 * size as f32 * 0.62) / 2.0;
    ops.push(Operation::new("BT", vec![]));
    ops.push(Operation::new("Tf", vec!["F1".into(), size.into()]));
    ops.push(Operation::new("Td", vec![x.max(36.0).into(), y.into()]));
    ops.push(Operation::new("Tj", vec![Object::string_literal(win_ansi(value))]));
    ops.push(Operation::new("ET", vec![]));
}

fn page_content(sheet: usize, front: bool) -> Result<Vec<u8>> {
    let mut ops = Vec::new();
    // Arrow pointing to the top of the page.
    let cx = WIDTH / 2.0;
    let points = [
        (cx, 790.0),
        (cx - 70.0, 690.0),
        (cx - 25.0, 690.0),
        (cx - 25.0, 600.0),
        (cx + 25.0, 600.0),
        (cx + 25.0, 690.0),
        (cx + 70.0, 690.0),
    ];
    ops.push(Operation::new("rg", vec![0.91.into(), 0.33.into(), 0.13.into()]));
    ops.push(Operation::new("m", vec![points[0].0.into(), points[0].1.into()]));
    for (x, y) in &points[1..] {
        ops.push(Operation::new("l", vec![(*x).into(), (*y).into()]));
    }
    ops.push(Operation::new("h", vec![]));
    ops.push(Operation::new("f", vec![]));
    ops.push(Operation::new("rg", vec![0.into(), 0.into(), 0.into()]));

    text(&mut ops, 28, 560.0, i18n("TOP"));
    text(&mut ops, 64, 420.0, &i18n_f("SHEET {sheet}", &[("sheet", &sheet)]));
    text(&mut ops, 48, 350.0, if front { i18n("FRONT") } else { i18n("BACK") });
    text(&mut ops, 14, 120.0, i18n("Manual duplex test - Canon TS3350 Toolset"));
    Ok(Content { operations: ops }.encode()?)
}

/// 4-page PDF: sheet 1 front, sheet 1 back, sheet 2 front, sheet 2 back.
pub fn calibration_pdf() -> Result<Vec<u8>> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica-Bold",
        "Encoding" => "WinAnsiEncoding",
    });
    let mut kids = Vec::new();
    for sheet in 1..=2 {
        for front in [true, false] {
            let content = doc.add_object(Stream::new(dictionary! {}, page_content(sheet, front)?));
            kids.push(Object::Reference(doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Contents" => content,
            })));
        }
    }
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => 4,
            "MediaBox" => vec![0.into(), 0.into(), WIDTH.into(), HEIGHT.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    doc.compress();
    let mut out = Vec::new();
    doc.save_to(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duplex::SourceDocument;

    #[test]
    fn calibration_document_has_four_pages() {
        let pdf = calibration_pdf().unwrap();
        assert_eq!(SourceDocument::load_mem(&pdf).unwrap().page_count(), 4);
    }
}
