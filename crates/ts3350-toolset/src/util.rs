use std::path::PathBuf;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use ts3350_core::i18n::{i18n, i18n_f};

/// Runs `work` on a background thread and resumes on the GTK thread with its result.
pub async fn background<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    match gio::spawn_blocking(work).await {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

pub fn texture_from_rgba(image: &image::RgbaImage) -> gdk::Texture {
    let (width, height) = image.dimensions();
    let bytes = glib::Bytes::from(image.as_raw().as_slice());
    gdk::MemoryTexture::new(width as i32, height as i32, gdk::MemoryFormat::R8g8b8a8, &bytes, width as usize * 4)
        .upcast()
}

/// Temporary folder of the application in `$XDG_RUNTIME_DIR`.
pub fn runtime_dir() -> PathBuf {
    let dir = glib::user_runtime_dir().join("ts3350-toolset");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Default file name of an export, e.g. "Scan 2026-10-23 20.15".
pub fn scan_file_name() -> String {
    // Translators: strftime format used in file names; avoid “/” and “:”.
    let date = glib::DateTime::now_local().and_then(|now| now.format(i18n("%Y-%m-%d %H.%M")));
    match date {
        Ok(date) => i18n_f("Scan {date}", &[("date", &date)]),
        Err(_) => i18n("Scanned document").to_owned(),
    }
}

/// Readable local time of a Unix timestamp.
pub fn format_time(unix: u64) -> String {
    glib::DateTime::from_unix_local(unix as i64)
        // Translators: strftime format of a reception date, e.g. “Oct 23, 20:15”.
        .and_then(|d| d.format(i18n("%b %d, %H:%M")))
        .map(|s| s.to_string())
        .unwrap_or_default()
}

pub fn string_list(items: &[String]) -> gtk::StringList {
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    gtk::StringList::new(&refs)
}

/// Renders a widget into a PNG (diagnostics, development builds only).
#[cfg(debug_assertions)]
pub fn save_snapshot(widget: &gtk::Widget, path: &str) {
    use gtk::gsk;
    use gtk::gsk::prelude::*;

    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, widget.width() as f64, widget.height() as f64);
    let Some(node) = snapshot.to_node() else { return };
    let renderer = gsk::CairoRenderer::new();
    if renderer.realize(None).is_ok() {
        if let Err(error) = renderer.render_texture(&node, None).save_to_png(path) {
            eprintln!("Snapshot failed: {error}");
        }
        renderer.unrealize();
    }
}
