//! Compiles the gettext catalogs (`po/<language>.po`) into Rust tables embedded in the binary,
//! so translations work without installing `.mo` files or the gettext tools.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

#[derive(Default)]
struct Entry {
    msgid: String,
    msgid_plural: Option<String>,
    msgstr: Vec<String>,
    fuzzy: bool,
}

#[derive(Clone, Copy)]
enum Field {
    None,
    Id,
    IdPlural,
    Str(usize),
}

fn unescape(quoted: &str, file: &Path, line: usize) -> String {
    let inner = quoted
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or_else(|| panic!("{}:{line}: malformed string {quoted}", file.display()));
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            other => panic!("{}:{line}: unsupported escape \\{other:?}", file.display()),
        }
    }
    out
}

fn parse(file: &Path) -> Vec<Entry> {
    let text = fs::read_to_string(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    let mut entries = Vec::new();
    let mut entry = Entry::default();
    let mut field = Field::None;
    let mut started = false;

    let mut flush = |entry: &mut Entry, started: &mut bool| {
        if *started {
            entries.push(std::mem::take(entry));
        }
        *started = false;
    };

    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let number = index + 1;
        if line.is_empty() {
            flush(&mut entry, &mut started);
            field = Field::None;
            continue;
        }
        if let Some(flags) = line.strip_prefix("#,") {
            entry.fuzzy |= flags.split(',').any(|f| f.trim() == "fuzzy");
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with('"') {
            let value = unescape(line, file, number);
            match field {
                Field::Id => entry.msgid.push_str(&value),
                Field::IdPlural => entry.msgid_plural.get_or_insert_with(String::new).push_str(&value),
                Field::Str(i) => entry.msgstr[i].push_str(&value),
                Field::None => panic!("{}:{number}: string outside of an entry", file.display()),
            }
            continue;
        }
        let (keyword, value) = line
            .split_once(char::is_whitespace)
            .unwrap_or_else(|| panic!("{}:{number}: unexpected line {line}", file.display()));
        let value = unescape(value.trim(), file, number);
        match keyword {
            "msgctxt" => panic!("{}:{number}: msgctxt is not supported", file.display()),
            "msgid" => {
                // A new msgid without a blank line starts a new entry.
                if started && !entry.msgstr.is_empty() {
                    flush(&mut entry, &mut started);
                }
                started = true;
                entry.msgid = value;
                field = Field::Id;
            }
            "msgid_plural" => {
                entry.msgid_plural = Some(value);
                field = Field::IdPlural;
            }
            "msgstr" => {
                entry.msgstr = vec![value];
                field = Field::Str(0);
            }
            other => {
                let index: usize = other
                    .strip_prefix("msgstr[")
                    .and_then(|s| s.strip_suffix(']'))
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| panic!("{}:{number}: unknown keyword {other}", file.display()));
                if entry.msgstr.len() <= index {
                    entry.msgstr.resize(index + 1, String::new());
                }
                entry.msgstr[index] = value;
                field = Field::Str(index);
            }
        }
    }
    flush(&mut entry, &mut started);
    entries
}

/// Converts a simple `Plural-Forms` expression (no `?:`) into a Rust expression.
fn plural_expression(header: &str, file: &Path) -> String {
    let rule = header
        .lines()
        .find_map(|l| l.strip_prefix("Plural-Forms:"))
        .and_then(|l| l.split(';').find_map(|part| part.trim().strip_prefix("plural=")))
        .unwrap_or_else(|| panic!("{}: missing Plural-Forms header", file.display()));
    if rule.contains('?') {
        panic!("{}: conditional plural rules are not supported: {rule}", file.display());
    }
    format!("({}) as usize", rule.trim())
}

fn main() {
    let po_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../po");
    println!("cargo:rerun-if-changed={}", po_dir.display());

    let mut files: Vec<_> =
        fs::read_dir(&po_dir).map(|dir| dir.filter_map(Result::ok).map(|e| e.path()).collect()).unwrap_or_default();
    files.retain(|p| p.extension().is_some_and(|e| e == "po"));
    files.sort();

    let mut code = String::from("pub(crate) static CATALOGS: &[Catalog] = &[\n");
    let mut rules = String::new();
    for file in &files {
        println!("cargo:rerun-if-changed={}", file.display());
        let language = file.file_stem().unwrap().to_string_lossy().into_owned();
        let entries = parse(file);
        let header = entries.iter().find(|e| e.msgid.is_empty()).map(|e| e.msgstr[0].clone()).unwrap_or_default();

        let mut messages: Vec<(&str, &str)> = Vec::new();
        let mut plurals: Vec<(&str, Vec<&str>)> = Vec::new();
        for entry in entries.iter().filter(|e| !e.msgid.is_empty() && !e.fuzzy) {
            if entry.msgstr.iter().any(String::is_empty) {
                continue;
            }
            match entry.msgid_plural {
                Some(_) => plurals.push((&entry.msgid, entry.msgstr.iter().map(String::as_str).collect())),
                None => messages.push((&entry.msgid, &entry.msgstr[0])),
            }
        }
        messages.sort();
        plurals.sort();

        let rule_fn = format!("plural_{}", language.replace(|c: char| !c.is_ascii_alphanumeric(), "_"));
        let _ = writeln!(
            rules,
            "#[allow(unused_parens, clippy::double_parens)]\nfn {rule_fn}(n: u64) -> usize {{ {} }}",
            plural_expression(&header, file)
        );
        let _ = writeln!(code, "    Catalog {{\n        language: {language:?},\n        messages: &[");
        for (id, text) in &messages {
            let _ = writeln!(code, "            ({id:?}, {text:?}),");
        }
        let _ = writeln!(code, "        ],\n        plurals: &[");
        for (id, forms) in &plurals {
            let _ = writeln!(code, "            ({id:?}, &{forms:?}),");
        }
        let _ = writeln!(code, "        ],\n        plural_index: {rule_fn},\n    }},");
    }
    code.push_str("];\n");
    code.push_str(&rules);

    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("catalogs.rs");
    fs::write(out, code).unwrap();
}
