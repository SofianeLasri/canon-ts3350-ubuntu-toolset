//! Logic shared by the GTK application and the CUPS backend.
//!
//! Nothing here depends on GTK: everything can be tested from the command line.

pub mod config;
pub mod cups;
pub mod duplex;
pub mod escl;
pub mod export;
pub mod i18n;
pub mod spool;

use std::fmt;

use i18n::i18n_f;

/// Application identifier (D-Bus, `.desktop` file, icon).
pub const APP_ID: &str = "io.github.sofianelasri.TS3350Toolset";
/// D-Bus path of the `GApplication` object, derived from [`APP_ID`].
pub const APP_DBUS_PATH: &str = "/io/github/sofianelasri/TS3350Toolset";
/// Name of the virtual CUPS queue created by `install.sh`.
pub const VIRTUAL_QUEUE: &str = "Canon_TS3350";
/// URI scheme of the CUPS backend (`ts3350-duplex:/<real queue>`).
pub const BACKEND_SCHEME: &str = "ts3350-duplex";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    Io(#[from] std::io::Error),
    Http(#[from] ureq::Error),
    Xml(String),
    Scanner { status: u16, message: String },
    Pdf(#[from] lopdf::Error),
    Image(#[from] image::ImageError),
    Cups(String),
    Config(String),
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Error::Io(e) => i18n_f("input/output error: {error}", &[("error", e)]),
            Error::Http(e) => i18n_f("network error: {error}", &[("error", e)]),
            Error::Xml(e) => i18n_f("invalid XML response: {error}", &[("error", e)]),
            Error::Scanner { status, message } => {
                i18n_f("the scanner answered {status}: {message}", &[("status", status), ("message", message)])
            }
            Error::Pdf(e) => i18n_f("PDF: {error}", &[("error", e)]),
            Error::Image(e) => i18n_f("image: {error}", &[("error", e)]),
            Error::Cups(e) => i18n_f("CUPS: {error}", &[("error", e)]),
            Error::Config(e) => i18n_f("configuration: {error}", &[("error", e)]),
            Error::Other(e) => e.clone(),
        };
        f.write_str(&text)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
