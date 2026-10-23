//! Minimal gettext-style localisation.
//!
//! Source strings are written in English. Translations live in `po/<language>.po` (standard
//! gettext format) and are compiled into the binary by `build.rs`. The language follows the
//! usual locale variables (`LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG`).
//!
//! Placeholders use named braces: `i18n_f("{count} pages", &[("count", &3)])`.

use std::fmt::Display;
use std::sync::OnceLock;

pub(crate) struct Catalog {
    language: &'static str,
    /// Sorted by msgid.
    messages: &'static [(&'static str, &'static str)],
    /// Sorted by singular msgid.
    plurals: &'static [(&'static str, &'static [&'static str])],
    plural_index: fn(u64) -> usize,
}

include!(concat!(env!("OUT_DIR"), "/catalogs.rs"));

/// Languages requested by the environment, most preferred first (same precedence as gettext).
fn requested_languages() -> Vec<String> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let locale = var("LC_ALL").or_else(|| var("LC_MESSAGES")).or_else(|| var("LANG"));
    let mut languages = Vec::new();
    // gettext ignores LANGUAGE when the locale is "C".
    if !matches!(locale.as_deref(), Some("C" | "POSIX"))
        && let Some(list) = var("LANGUAGE")
    {
        languages.extend(list.split(':').filter(|l| !l.is_empty()).map(str::to_owned));
    }
    languages.extend(locale);
    languages
}

fn select(languages: &[String]) -> Option<&'static Catalog> {
    for requested in languages {
        let code = requested.split(['_', '.', '@']).next().unwrap_or_default();
        if code == "en" || code == "C" || code == "POSIX" {
            return None;
        }
        if let Some(catalog) = CATALOGS.iter().find(|c| c.language == code) {
            return Some(catalog);
        }
    }
    None
}

fn active() -> Option<&'static Catalog> {
    static ACTIVE: OnceLock<Option<&'static Catalog>> = OnceLock::new();
    *ACTIVE.get_or_init(|| select(&requested_languages()))
}

/// Code of the active UI language (`"en"` when untranslated).
pub fn language() -> &'static str {
    active().map_or("en", |c| c.language)
}

fn lookup(catalog: &Catalog, msgid: &str) -> Option<&'static str> {
    catalog.messages.binary_search_by_key(&msgid, |(id, _)| id).ok().map(|i| catalog.messages[i].1)
}

fn lookup_plural(catalog: &Catalog, singular: &str, n: u64) -> Option<&'static str> {
    let index = catalog.plurals.binary_search_by_key(&singular, |(id, _)| id).ok()?;
    let forms = catalog.plurals[index].1;
    forms.get((catalog.plural_index)(n)).or(forms.last()).copied()
}

/// Marks a string for translation without translating it, for constant tables.
/// Translate it later with [`i18n`].
pub const fn n_(msgid: &'static str) -> &'static str {
    msgid
}

/// Translates a message.
pub fn i18n(msgid: &'static str) -> &'static str {
    active().and_then(|c| lookup(c, msgid)).unwrap_or(msgid)
}

/// Translates a message whose form depends on `n`.
pub fn ni18n(singular: &'static str, plural: &'static str, n: u64) -> &'static str {
    match active() {
        Some(catalog) => lookup_plural(catalog, singular, n).unwrap_or(if n == 1 { singular } else { plural }),
        None if n == 1 => singular,
        None => plural,
    }
}

/// Replaces `{name}` placeholders.
pub fn fill(template: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = template.to_owned();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), &value.to_string());
    }
    out
}

/// Translates a message, then fills its placeholders.
pub fn i18n_f(msgid: &'static str, args: &[(&str, &dyn Display)]) -> String {
    fill(i18n(msgid), args)
}

/// Translates a plural message, then fills its placeholders.
pub fn ni18n_f(singular: &'static str, plural: &'static str, n: u64, args: &[(&str, &dyn Display)]) -> String {
    fill(ni18n(singular, plural, n), args)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashSet};
    use std::path::Path;

    use super::*;

    fn french() -> &'static Catalog {
        CATALOGS.iter().find(|c| c.language == "fr").expect("French catalog")
    }

    #[test]
    fn selects_language_from_locale() {
        let langs = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(select(&langs(&["fr_FR.UTF-8"])).map(|c| c.language), Some("fr"));
        assert_eq!(select(&langs(&["de", "fr_BE"])).map(|c| c.language), Some("fr"));
        assert!(select(&langs(&["en_US.UTF-8", "fr"])).is_none());
        assert!(select(&langs(&["C"])).is_none());
        assert!(select(&[]).is_none());
    }

    #[test]
    fn french_plurals() {
        let fr = french();
        assert_eq!(lookup_plural(fr, "{count} page", 1), Some("{count} page"));
        assert_eq!(lookup_plural(fr, "{count} page", 0), Some("{count} page"));
        assert_eq!(lookup_plural(fr, "{count} page", 2), Some("{count} pages"));
    }

    #[test]
    fn fills_placeholders() {
        assert_eq!(fill("{a} and {b} and {a}", &[("a", &1), ("b", &"x")]), "1 and x and 1");
    }

    /// Reads a Rust string literal starting at `s` (which begins with `"`).
    fn read_literal(s: &str) -> Option<(String, &str)> {
        let mut chars = s.strip_prefix('"')?.char_indices();
        let mut out = String::new();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => return Some((out, &s[i + 2..])),
                '\\' => match chars.next()?.1 {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    other => out.push(other),
                },
                c => out.push(c),
            }
        }
        None
    }

    /// Every message passed to the i18n helpers in the workspace sources.
    fn source_messages() -> (BTreeSet<String>, BTreeSet<String>) {
        let mut singular = BTreeSet::new();
        let mut plural = BTreeSet::new();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut stack = vec![root.join("crates")];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") && !path.ends_with("i18n.rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for (pattern, is_plural) in
                        [("ni18n_f(", true), ("ni18n(", true), ("i18n_f(", false), ("i18n(", false), ("n_(", false)]
                    {
                        let mut rest = text.as_str();
                        while let Some(pos) = rest.find(pattern) {
                            let before = rest[..pos].chars().last();
                            rest = &rest[pos + pattern.len()..];
                            // Skip `ni18n(` when looking for `i18n(`, and identifiers ending in i18n.
                            if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                                continue;
                            }
                            let Some((msgid, after)) = read_literal(rest.trim_start()) else { continue };
                            if is_plural {
                                plural.insert(msgid);
                                let _ = after;
                            } else {
                                singular.insert(msgid);
                            }
                        }
                    }
                }
            }
        }
        (singular, plural)
    }

    fn placeholders(text: &str) -> BTreeSet<&str> {
        text.split('{').skip(1).filter_map(|part| part.split_once('}').map(|(name, _)| name)).collect()
    }

    #[test]
    fn french_catalog_covers_every_source_message() {
        let fr = french();
        let (singular, plural) = source_messages();
        assert!(singular.len() > 50, "message extraction looks broken");

        let missing: Vec<_> = singular.iter().filter(|m| lookup(fr, m).is_none()).collect();
        assert!(missing.is_empty(), "untranslated messages in po/fr.po: {missing:#?}");
        let missing: Vec<_> = plural.iter().filter(|m| lookup_plural(fr, m, 2).is_none()).collect();
        assert!(missing.is_empty(), "untranslated plural messages in po/fr.po: {missing:#?}");

        for (id, text) in fr.messages {
            assert_eq!(placeholders(id), placeholders(text), "placeholders differ for {id:?}");
        }
        for (id, forms) in fr.plurals {
            // The singular form may omit `{count}`.
            let mut allowed = placeholders(id);
            allowed.insert("count");
            for form in forms.iter() {
                assert!(placeholders(form).is_subset(&allowed), "unknown placeholder in {form:?}");
            }
        }

        // Entries no longer used by the code.
        let used: HashSet<&str> = singular.iter().chain(&plural).map(String::as_str).collect();
        let stale: Vec<_> = fr
            .messages
            .iter()
            .map(|(id, _)| *id)
            .chain(fr.plurals.iter().map(|(id, _)| *id))
            .filter(|id| !used.contains(id))
            .collect();
        assert!(stale.is_empty(), "obsolete entries in po/fr.po: {stale:#?}");
    }
}
