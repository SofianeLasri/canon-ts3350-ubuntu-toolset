//! Export dialog: format, quality, resolution, then file selection.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use ts3350_core::export::{self, ExportFormat, ExportOptions, PngCompression, ScannedPage};
use ts3350_core::i18n::{i18n, i18n_f, ni18n_f};

use crate::context::AppContext;
use crate::util::{background, scan_file_name, string_list};

const PNG_LEVELS: [PngCompression; 3] = [PngCompression::Fast, PngCompression::Default, PngCompression::Best];
const AVIF_SPEEDS: [u8; 3] = [8, 6, 3];
const MAX_DPI: [Option<u32>; 5] = [None, Some(300), Some(200), Some(150), Some(100)];

struct Rows {
    format: adw::ComboRow,
    original: adw::SwitchRow,
    quality: adw::ActionRow,
    scale: gtk::Scale,
    png: adw::ComboRow,
    avif: adw::ComboRow,
    dpi: adw::ComboRow,
    files_hint: adw::PreferencesGroup,
}

impl Rows {
    fn format(&self) -> ExportFormat {
        ExportFormat::ALL[(self.format.selected() as usize).min(ExportFormat::ALL.len() - 1)]
    }

    fn options(&self) -> ExportOptions {
        ExportOptions {
            format: self.format(),
            quality: self.scale.value().round() as u8,
            keep_original: self.original.is_active(),
            max_dpi: MAX_DPI[(self.dpi.selected() as usize).min(MAX_DPI.len() - 1)],
            png_compression: PNG_LEVELS[(self.png.selected() as usize).min(PNG_LEVELS.len() - 1)],
            avif_speed: AVIF_SPEEDS[(self.avif.selected() as usize).min(AVIF_SPEEDS.len() - 1)],
        }
    }

    fn update(&self, pages: usize) {
        let format = self.format();
        let pdf = format == ExportFormat::Pdf;
        self.original.set_visible(pdf);
        self.quality.set_visible(format != ExportFormat::Png);
        self.quality.set_sensitive(!pdf || !self.original.is_active());
        self.png.set_visible(format == ExportFormat::Png);
        self.avif.set_visible(format == ExportFormat::Avif);
        let several = !format.is_multipage() && pages > 1;
        self.files_hint.set_visible(several);
        if several {
            self.files_hint.set_description(Some(&ni18n_f(
                "A {format} file holds a single image: the {count} page will be saved as numbered files (-01, -02…).",
                "A {format} file holds a single image: the {count} pages will be saved as numbered files (-01, -02…).",
                pages as u64,
                &[("format", &format.extension().to_uppercase()), ("count", &pages)],
            )));
        }
    }
}

pub fn show(ctx: Rc<AppContext>, pages: Vec<ScannedPage>) {
    if pages.is_empty() {
        return;
    }
    let saved = ctx.config.borrow().export.clone();

    let labels: Vec<String> = ExportFormat::ALL.iter().map(|f| f.label().to_owned()).collect();
    let format = adw::ComboRow::builder().title(i18n("Format")).model(&string_list(&labels)).build();
    format.set_subtitle(i18n("A PDF holds all the pages in a single file"));
    format.set_selected(ExportFormat::ALL.iter().position(|f| *f == saved.format).unwrap_or(0) as u32);

    let original = adw::SwitchRow::builder()
        .title(i18n("Original quality"))
        .subtitle(i18n("Embeds the scanner images as is, without recompressing them"))
        .active(saved.keep_original)
        .build();
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 10.0, 100.0, 1.0);
    scale.set_value(saved.quality.clamp(10, 100) as f64);
    scale.set_digits(0);
    scale.set_draw_value(true);
    scale.set_value_pos(gtk::PositionType::Left);
    scale.set_hexpand(true);
    scale.set_width_request(180);
    for (mark, label) in [(50.0, None), (85.0, Some(i18n("Recommended")))] {
        scale.add_mark(mark, gtk::PositionType::Bottom, label);
    }
    let quality = adw::ActionRow::builder()
        .title(i18n("Quality"))
        .subtitle(i18n("The lower it is, the smaller the file"))
        .build();
    quality.add_suffix(&scale);

    let png = adw::ComboRow::builder()
        .title(i18n("Compression"))
        .subtitle(i18n("Lossless: only the saving time changes"))
        .model(&gtk::StringList::new(&[i18n("Fast"), i18n("Normal"), i18n("Maximum")]))
        .build();
    png.set_selected(PNG_LEVELS.iter().position(|l| *l == saved.png_compression).unwrap_or(1) as u32);
    let avif = adw::ComboRow::builder()
        .title(i18n("Encoding speed"))
        .model(&gtk::StringList::new(&[i18n("Fast"), i18n("Balanced"), i18n("Slow (smaller file)")]))
        .build();
    avif.set_selected(AVIF_SPEEDS.iter().position(|s| *s == saved.avif_speed).unwrap_or(1) as u32);
    let dpi = adw::ComboRow::builder()
        .title(i18n("Resolution"))
        .subtitle(i18n("Lowering the resolution makes the file much smaller"))
        .model(&string_list(
            &MAX_DPI
                .iter()
                .map(|dpi| match dpi {
                    Some(dpi) => i18n_f("{dpi} dpi", &[("dpi", dpi)]),
                    None => i18n("Original").to_owned(),
                })
                .collect::<Vec<_>>(),
        ))
        .build();
    dpi.set_selected(MAX_DPI.iter().position(|d| *d == saved.max_dpi).unwrap_or(0) as u32);

    let format_group = adw::PreferencesGroup::new();
    format_group.add(&format);
    let quality_group = adw::PreferencesGroup::builder().title(i18n("Quality")).build();
    for row in [
        original.upcast_ref::<gtk::Widget>(),
        quality.upcast_ref(),
        png.upcast_ref(),
        avif.upcast_ref(),
        dpi.upcast_ref(),
    ] {
        quality_group.add(row);
    }
    let files_hint = adw::PreferencesGroup::new();

    let save = gtk::Button::builder().label(i18n("Save…")).halign(gtk::Align::Center).build();
    save.add_css_class("pill");
    save.add_css_class("suggested-action");
    let save_group = adw::PreferencesGroup::new();
    save_group.add(&save);

    let page = adw::PreferencesPage::new();
    page.add(&format_group);
    page.add(&quality_group);
    page.add(&files_hint);
    page.add(&save_group);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&page));
    let dialog = adw::Dialog::builder()
        .title(ni18n_f("Export {count} page", "Export {count} pages", pages.len() as u64, &[("count", &pages.len())]))
        .content_width(520)
        .child(&toolbar)
        .build();

    let rows = Rc::new(Rows { format, original, quality, scale, png, avif, dpi, files_hint });
    rows.update(pages.len());
    let count = pages.len();
    let r = rows.clone();
    rows.format.connect_selected_notify(move |_| r.update(count));
    let r = rows.clone();
    rows.original.connect_active_notify(move |_| r.update(count));

    let pages = Rc::new(pages);
    let window = ctx.window.clone();
    let dialog_ref = dialog.clone();
    save.connect_clicked(move |_| {
        let options = rows.options();
        ctx.update_config(|c| c.export = options.clone());
        let ctx = ctx.clone();
        let pages = pages.clone();
        let dialog = dialog_ref.clone();
        glib::spawn_future_local(async move {
            let Some(path) = choose_file(&ctx, options.format).await else { return };
            dialog.close();
            save_document(ctx, pages, options, path).await;
        });
    });

    dialog.present(Some(&window));
}

async fn choose_file(ctx: &AppContext, format: ExportFormat) -> Option<std::path::PathBuf> {
    let filter = gtk::FileFilter::new();
    filter.add_mime_type(format.mime());
    filter.set_name(Some(&i18n_f("{format} files", &[("format", &format.extension().to_uppercase())])));
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);

    let folder = ctx
        .config
        .borrow()
        .last_export_dir
        .clone()
        .filter(|d| d.is_dir())
        .or_else(|| glib::user_special_dir(glib::UserDirectory::Documents));
    let chooser = gtk::FileDialog::builder()
        .title(i18n("Save the document"))
        .initial_name(format!("{}.{}", scan_file_name(), format.extension()))
        .filters(&filters)
        .modal(true)
        .build();
    if let Some(folder) = folder {
        chooser.set_initial_folder(Some(&gio::File::for_path(folder)));
    }
    chooser.save_future(Some(&ctx.window)).await.ok()?.path()
}

async fn save_document(
    ctx: Rc<AppContext>,
    pages: Rc<Vec<ScannedPage>>,
    options: ExportOptions,
    path: std::path::PathBuf,
) {
    let progress = adw::Toast::builder().title(i18n("Saving…")).timeout(0).build();
    ctx.toasts.add_toast(progress.clone());

    let title = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let pages = pages.as_ref().clone();
    let target = path.clone();
    let result = background(move || export::export(&pages, &options, &target, &title)).await;
    progress.dismiss();

    match result {
        Ok(files) => {
            if let Some(dir) = path.parent() {
                ctx.update_config(|c| c.last_export_dir = Some(dir.to_owned()));
            }
            let Some(first) = files.first().cloned() else { return };
            let window = ctx.window.clone();
            if files.len() == 1 {
                ctx.toast_with_button(i18n("Document saved"), i18n("Open"), move || {
                    gtk::FileLauncher::new(Some(&gio::File::for_path(&first))).launch(
                        Some(&window),
                        None::<&gio::Cancellable>,
                        |_| {},
                    );
                });
            } else {
                let message = ni18n_f(
                    "{count} file saved",
                    "{count} files saved",
                    files.len() as u64,
                    &[("count", &files.len())],
                );
                ctx.toast_with_button(&message, i18n("Show"), move || {
                    gtk::FileLauncher::new(Some(&gio::File::for_path(&first))).open_containing_folder(
                        Some(&window),
                        None::<&gio::Cancellable>,
                        |_| {},
                    );
                });
            }
        }
        Err(error) => ctx.error(i18n("Saving failed"), error),
    }
}
