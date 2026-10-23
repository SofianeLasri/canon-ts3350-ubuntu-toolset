//! Builds the "fronts" and "backs" PDFs from the source document.

use std::collections::HashSet;
use std::path::Path;

use lopdf::{Dictionary, Document, Object, ObjectId, dictionary};

use super::plan::Slot;
use crate::i18n::{i18n, i18n_f};
use crate::{Error, Result};

/// Page attributes inherited from the `/Pages` tree (PDF 32000, § 7.7.3.4).
const INHERITABLE: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];
/// A4 in points, for a blank page in a document without `/MediaBox`.
const A4: [f32; 4] = [0.0, 0.0, 595.276, 841.89];

pub struct SourceDocument {
    doc: Document,
    pages: Vec<ObjectId>,
}

impl SourceDocument {
    pub fn load(path: &Path) -> Result<SourceDocument> {
        SourceDocument::from_document(Document::load(path)?)
    }

    pub fn load_mem(bytes: &[u8]) -> Result<SourceDocument> {
        SourceDocument::from_document(Document::load_mem(bytes)?)
    }

    fn from_document(mut doc: Document) -> Result<SourceDocument> {
        if doc.is_encrypted() {
            doc.decrypt("").map_err(|_| Error::Other(i18n("this PDF is password protected").into()))?;
        }
        let pages: Vec<ObjectId> = doc.get_pages().into_values().collect();
        if pages.is_empty() {
            return Err(Error::Other(i18n("the PDF contains no page").into()));
        }
        Ok(SourceDocument { doc, pages })
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Looks a key up on the page, then on its `/Pages` ancestors.
    fn inherited(&self, page: ObjectId, key: &[u8]) -> Option<Object> {
        let mut seen = HashSet::new();
        let mut current = page;
        while seen.insert(current) {
            let dict = self.doc.get_dictionary(current).ok()?;
            if let Ok(value) = dict.get(key) {
                return Some(value.clone());
            }
            current = dict.get(b"Parent").and_then(Object::as_reference).ok()?;
        }
        None
    }

    fn media_box(&self, page: ObjectId) -> Object {
        self.inherited(page, b"MediaBox").unwrap_or_else(|| Object::Array(A4.iter().map(|&v| v.into()).collect()))
    }

    /// PDF containing only the requested sides, in order, rotated by 180° if `rotate`.
    pub fn build_pass(&self, slots: &[Slot], rotate: bool) -> Result<Vec<u8>> {
        let mut doc = self.doc.clone();
        let root_pages = doc
            .catalog()?
            .get(b"Pages")
            .and_then(Object::as_reference)
            .map_err(|_| Error::Other(i18n("page tree not found").into()))?;
        let blank_box = self.media_box(self.pages[0]);

        let mut used = HashSet::new();
        let mut kids = Vec::with_capacity(slots.len());
        for slot in slots {
            let mut page = match *slot {
                Slot::Page(index) => {
                    let id = *self.pages.get(index).ok_or_else(|| {
                        Error::Other(i18n_f("page {page} is missing from the PDF", &[("page", &(index + 1))]))
                    })?;
                    let mut dict: Dictionary = self.doc.get_dictionary(id)?.clone();
                    // The page is attached directly to the root: materialise inherited attributes.
                    for key in INHERITABLE {
                        if !dict.has(key)
                            && let Some(value) = self.inherited(id, key)
                        {
                            dict.set(key, value);
                        }
                    }
                    if rotate {
                        let current = dict.get(b"Rotate").and_then(Object::as_i64).unwrap_or(0);
                        dict.set("Rotate", (current + 180).rem_euclid(360));
                    }
                    // A page already placed is duplicated rather than shared.
                    if used.insert(id) {
                        dict.set("Parent", root_pages);
                        doc.objects.insert(id, Object::Dictionary(dict));
                        kids.push(Object::Reference(id));
                        continue;
                    }
                    dict
                }
                Slot::Blank => dictionary! {
                    "Type" => "Page",
                    "MediaBox" => blank_box.clone(),
                    "Resources" => dictionary! {},
                },
            };
            page.set("Parent", root_pages);
            kids.push(Object::Reference(doc.add_object(page)));
        }

        let count = kids.len() as i64;
        let root = doc.get_dictionary_mut(root_pages)?;
        root.set("Kids", kids);
        root.set("Count", count);

        // These structures point to pages missing from the pass.
        if let Ok(catalog) = doc.catalog_mut() {
            for key in [&b"Outlines"[..], b"StructTreeRoot", b"OpenAction", b"PageLabels", b"MarkInfo"] {
                catalog.remove(key);
            }
        }
        doc.prune_objects();

        let mut out = Vec::new();
        doc.save_to(&mut out)?;
        Ok(out)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use lopdf::Stream;
    use lopdf::content::{Content, Operation};

    /// Test PDF: each page shows its number, half of the pages inherit the MediaBox.
    pub fn numbered_pdf(pages: usize) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let root_pages = doc.new_object_id();
        let inner_pages = doc.new_object_id();
        let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let mut inner_kids = Vec::new();
        for i in 0..pages {
            let content = Content {
                operations: vec![
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec!["F1".into(), 48.into()]),
                    Operation::new("Td", vec![100.into(), 600.into()]),
                    Operation::new("Tj", vec![Object::string_literal(format!("PAGE {}", i + 1))]),
                    Operation::new("ET", vec![]),
                ],
            };
            let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
            let mut page = dictionary! {
                "Type" => "Page",
                "Parent" => inner_pages,
                "Contents" => content_id,
                // Marker used by the tests to find the original page.
                "PieceInfo" => (i as i64),
            };
            if i % 2 == 0 {
                page.set("MediaBox", vec![0.into(), 0.into(), 420.into(), 595.into()]);
            }
            inner_kids.push(Object::Reference(doc.add_object(page)));
        }
        doc.objects.insert(
            inner_pages,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Parent" => root_pages,
                "Kids" => inner_kids,
                "Count" => pages as i64,
                "Rotate" => 90,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            }),
        );
        doc.objects.insert(
            root_pages,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![Object::Reference(inner_pages)],
                "Count" => pages as i64,
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => root_pages });
        doc.trailer.set("Root", catalog);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    /// (original index or None for a blank page, rotation, MediaBox width)
    fn describe(pdf: &[u8]) -> Vec<(Option<i64>, i64, f32)> {
        let doc = Document::load_mem(pdf).unwrap();
        doc.get_pages()
            .into_values()
            .map(|id| {
                let dict = doc.get_dictionary(id).unwrap();
                let marker = dict.get(b"PieceInfo").and_then(Object::as_i64).ok();
                let rotate = dict.get(b"Rotate").and_then(Object::as_i64).unwrap_or(0);
                let width = dict.get(b"MediaBox").unwrap().as_array().unwrap()[2].as_float().unwrap();
                assert!(dict.get(b"Resources").is_ok(), "inherited resources lost");
                (marker, rotate, width)
            })
            .collect()
    }

    #[test]
    fn builds_front_and_back_passes() {
        let source = SourceDocument::load_mem(&numbered_pdf(5)).unwrap();
        assert_eq!(source.page_count(), 5);

        let fronts = source.build_pass(&[Slot::Page(0), Slot::Page(2), Slot::Page(4)], false).unwrap();
        assert_eq!(describe(&fronts), vec![(Some(0), 90, 420.0), (Some(2), 90, 420.0), (Some(4), 90, 420.0)]);

        let backs = source.build_pass(&[Slot::Blank, Slot::Page(3), Slot::Page(1)], true).unwrap();
        assert_eq!(describe(&backs), vec![(None, 0, 420.0), (Some(3), 270, 595.0), (Some(1), 270, 595.0)]);
    }

    #[test]
    fn duplicated_slots_are_copied() {
        let source = SourceDocument::load_mem(&numbered_pdf(2)).unwrap();
        let pass = source.build_pass(&[Slot::Page(1), Slot::Page(1)], false).unwrap();
        assert_eq!(describe(&pass), vec![(Some(1), 90, 595.0), (Some(1), 90, 595.0)]);
    }

    #[test]
    fn rejects_out_of_range_pages() {
        let source = SourceDocument::load_mem(&numbered_pdf(2)).unwrap();
        assert!(source.build_pass(&[Slot::Page(7)], false).is_err());
    }
}
