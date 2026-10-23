use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use ts3350_core::i18n::i18n;
use ts3350_core::{BACKEND_SCHEME, VIRTUAL_QUEUE, cups};

use crate::util::{background, string_list};
use crate::window::MainWindow;

pub fn show(window: &Rc<MainWindow>) {
    let ctx = window.ctx.clone();
    let config = ctx.config.borrow().clone();

    // --- Scanner ---
    let url_row = adw::EntryRow::builder()
        .title(i18n("Scanner eSCL address"))
        .text(config.scanner_url.clone().unwrap_or_default())
        .show_apply_button(true)
        .build();
    let scanner_group = adw::PreferencesGroup::builder()
        .title(i18n("Scanner"))
        .description(i18n("Leave empty to discover the scanner automatically. Example: http://192.168.1.38/eSCL"))
        .build();
    scanner_group.add(&url_row);
    let w = window.clone();
    url_row.connect_apply(move |row| {
        let text = row.text().trim().to_owned();
        w.ctx.update_config(|c| c.scanner_url = (!text.is_empty()).then_some(text));
        w.scan.reconnect();
        w.ctx.toast(i18n("Reconnecting to the scanner"));
    });

    // --- Physical printer ---
    let queue_row = adw::ComboRow::builder()
        .title(i18n("Physical printer"))
        .subtitle(i18n("CUPS queue that receives the fronts and backs"))
        .model(&gtk::StringList::new(&[i18n("Automatic")]))
        .build();
    let printer_group = adw::PreferencesGroup::builder().title(i18n("Printing")).build();
    printer_group.add(&queue_row);
    let queues: Rc<std::cell::RefCell<Vec<String>>> = Rc::default();
    let loading = Rc::new(Cell::new(true));
    {
        let (row, queues, loading, preferred) =
            (queue_row.clone(), queues.clone(), loading.clone(), config.target_queue.clone());
        glib::spawn_future_local(async move {
            let found = background(cups::queues).await.unwrap_or_default();
            let names: Vec<String> = found
                .into_iter()
                .filter(|q| q.name != VIRTUAL_QUEUE && !q.uri.starts_with(BACKEND_SCHEME))
                .map(|q| q.name)
                .collect();
            let mut labels = vec![i18n("Automatic").to_owned()];
            labels.extend(names.iter().cloned());
            row.set_model(Some(&string_list(&labels)));
            let index = preferred.and_then(|p| names.iter().position(|n| *n == p)).map(|i| i + 1).unwrap_or(0);
            row.set_selected(index as u32);
            *queues.borrow_mut() = names;
            loading.set(false);
        });
    }
    let w = window.clone();
    let (q, l) = (queues.clone(), loading.clone());
    queue_row.connect_selected_notify(move |row| {
        if l.get() {
            return;
        }
        let selected = row.selected() as usize;
        let queue = if selected == 0 { None } else { q.borrow().get(selected - 1).cloned() };
        w.ctx.update_config(|c| c.target_queue = queue);
        w.duplex.refresh();
    });

    // --- Duplex ---
    let reverse_row = adw::SwitchRow::builder()
        .title(i18n("Reverse the order of the backs"))
        .subtitle(i18n("The last sheet gets its back printed first"))
        .active(config.duplex.reverse_backs)
        .build();
    let rotate_row =
        adw::SwitchRow::builder().title(i18n("Rotate the backs by 180°")).active(config.duplex.rotate_backs).build();
    let calibrate_row = adw::ButtonRow::builder()
        .title(i18n("Print a test page…"))
        .start_icon_name("document-print-symbolic")
        .action_name("app.calibrate")
        .build();
    let duplex_group = adw::PreferencesGroup::builder()
        .title(i18n("Manual duplex"))
        .description(i18n("These settings come from the test page: leave them alone if the backs come out right."))
        .build();
    duplex_group.add(&reverse_row);
    duplex_group.add(&rotate_row);
    duplex_group.add(&calibrate_row);
    let w = window.clone();
    reverse_row.connect_active_notify(move |row| {
        let active = row.is_active();
        w.ctx.update_config(|c| c.duplex.reverse_backs = active);
        w.duplex.refresh();
    });
    let w = window.clone();
    rotate_row.connect_active_notify(move |row| {
        let active = row.is_active();
        w.ctx.update_config(|c| c.duplex.rotate_backs = active);
        w.duplex.refresh();
    });

    let page = adw::PreferencesPage::builder().title(i18n("General")).icon_name("preferences-system-symbolic").build();
    page.add(&scanner_group);
    page.add(&printer_group);
    page.add(&duplex_group);
    let dialog = adw::PreferencesDialog::builder().title(i18n("Preferences")).build();
    dialog.add(&page);

    let d = dialog.clone();
    calibrate_row.connect_activated(move |_| {
        d.close();
    });
    dialog.present(Some(&ctx.window));
}
