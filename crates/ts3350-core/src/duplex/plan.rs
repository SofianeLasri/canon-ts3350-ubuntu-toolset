//! Distribution of the pages between the two passes of manual duplex printing.

use serde::{Deserialize, Serialize};

use crate::i18n::i18n;

/// Binding edge, as in the print dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Binding {
    /// "Book" flip (CUPS option `DuplexNoTumble`).
    #[default]
    LongEdge,
    /// "Notepad" flip (CUPS option `DuplexTumble`).
    ShortEdge,
}

impl Binding {
    pub fn label(self) -> &'static str {
        match self {
            Binding::LongEdge => i18n("Long edge (book)"),
            Binding::ShortEdge => i18n("Short edge (notepad)"),
        }
    }
}

/// Physical behaviour of the printer when the stack is reloaded.
///
/// The TS3350 outputs sheets face up. Once the stack is flipped and put back
/// top edge first in the rear tray, sheet 1 goes through first, the right way
/// up. Calibration corrects these values if practice shows otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DuplexProfile {
    /// Print the backs from the last sheet to the first.
    pub reverse_backs: bool,
    /// Rotate the backs by 180° (on top of the rotation required by the binding).
    pub rotate_backs: bool,
}

/// Content of one side of a sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// Zero-based index of a document page.
    Page(usize),
    /// Side left blank so the sheet is ejected and the stack stays aligned.
    Blank,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassPlan {
    pub sheets: usize,
    /// First pass: one side per sheet, from sheet 1 to the last.
    pub fronts: Vec<Slot>,
    /// Second pass, in printing order.
    pub backs: Vec<Slot>,
    /// Whether the backs must be rotated by 180°.
    pub rotate_backs: bool,
}

impl PassPlan {
    /// A single-page document needs no second pass.
    pub fn needs_second_pass(&self) -> bool {
        self.backs.iter().any(|s| matches!(s, Slot::Page(_)))
    }
}

pub fn plan(page_count: usize, binding: Binding, profile: &DuplexProfile) -> PassPlan {
    let sheets = page_count.div_ceil(2);
    let fronts = (0..sheets).map(|sheet| Slot::Page(sheet * 2)).collect();
    let mut backs: Vec<Slot> = (0..sheets)
        .map(|sheet| {
            let index = sheet * 2 + 1;
            if index < page_count { Slot::Page(index) } else { Slot::Blank }
        })
        .collect();
    if profile.reverse_backs {
        backs.reverse();
    }
    PassPlan { sheets, fronts, backs, rotate_backs: (binding == Binding::ShortEdge) != profile.rotate_backs }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Slot::{Blank, Page};

    const NATURAL: DuplexProfile = DuplexProfile { reverse_backs: false, rotate_backs: false };

    #[test]
    fn even_document() {
        let p = plan(4, Binding::LongEdge, &NATURAL);
        assert_eq!(p.sheets, 2);
        assert_eq!(p.fronts, vec![Page(0), Page(2)]);
        assert_eq!(p.backs, vec![Page(1), Page(3)]);
        assert!(!p.rotate_backs);
        assert!(p.needs_second_pass());
    }

    #[test]
    fn odd_document_gets_a_blank_back() {
        let p = plan(5, Binding::LongEdge, &NATURAL);
        assert_eq!(p.fronts, vec![Page(0), Page(2), Page(4)]);
        assert_eq!(p.backs, vec![Page(1), Page(3), Blank]);
    }

    #[test]
    fn reversed_profile() {
        let profile = DuplexProfile { reverse_backs: true, rotate_backs: false };
        let p = plan(5, Binding::LongEdge, &profile);
        assert_eq!(p.backs, vec![Blank, Page(3), Page(1)]);
    }

    #[test]
    fn rotation_combines_binding_and_profile() {
        for (binding, rotate_profile, expected) in [
            (Binding::LongEdge, false, false),
            (Binding::LongEdge, true, true),
            (Binding::ShortEdge, false, true),
            (Binding::ShortEdge, true, false),
        ] {
            let profile = DuplexProfile { reverse_backs: false, rotate_backs: rotate_profile };
            assert_eq!(plan(2, binding, &profile).rotate_backs, expected);
        }
    }

    #[test]
    fn every_page_is_printed_exactly_once() {
        for pages in 1..=7 {
            for reverse in [false, true] {
                let profile = DuplexProfile { reverse_backs: reverse, rotate_backs: false };
                let p = plan(pages, Binding::LongEdge, &profile);
                assert_eq!(p.fronts.len(), p.backs.len());
                let mut seen: Vec<usize> = p
                    .fronts
                    .iter()
                    .chain(&p.backs)
                    .filter_map(|s| if let Page(i) = s { Some(*i) } else { None })
                    .collect();
                seen.sort_unstable();
                assert_eq!(seen, (0..pages).collect::<Vec<_>>());
            }
        }
    }

    #[test]
    fn single_page_needs_one_pass() {
        let p = plan(1, Binding::LongEdge, &NATURAL);
        assert_eq!(p.backs, vec![Blank]);
        assert!(!p.needs_second_pass());
    }
}
