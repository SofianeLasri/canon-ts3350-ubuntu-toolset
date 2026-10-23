use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use ts3350_core::APP_ID;
use ts3350_core::i18n::i18n;

use crate::preferences;
use crate::window::MainWindow;

type WindowSlot = Rc<RefCell<Option<Rc<MainWindow>>>>;

/// Single application window, created on first request.
fn main_window(app: &adw::Application, slot: &WindowSlot) -> Rc<MainWindow> {
    if let Some(window) = slot.borrow().as_ref() {
        return window.clone();
    }
    let window = MainWindow::new(app);
    *slot.borrow_mut() = Some(window.clone());
    window
}

pub fn run() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::HANDLES_OPEN).build();
    let slot: WindowSlot = Rc::default();

    app.connect_startup(|app| {
        gtk::Window::set_default_icon_name(APP_ID);
        // During development the icon is not installed in the theme yet.
        let dev_icons = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/icons"));
        if dev_icons.is_dir()
            && let Some(display) = gdk::Display::default()
        {
            gtk::IconTheme::for_display(&display).add_search_path(dev_icons);
        }
        app.set_accels_for_action("app.preferences", &["<Control>comma"]);
        app.set_accels_for_action("app.quit", &["<Control>q"]);
        app.set_accels_for_action("window.close", &["<Control>w"]);
    });

    let preferences_action = gio::SimpleAction::new("preferences", None);
    let (a, s) = (app.clone(), slot.clone());
    preferences_action.connect_activate(move |_, _| preferences::show(&main_window(&a, &s)));
    app.add_action(&preferences_action);

    let about_action = gio::SimpleAction::new("about", None);
    let (a, s) = (app.clone(), slot.clone());
    about_action.connect_activate(move |_, _| {
        let window = main_window(&a, &s);
        adw::AboutDialog::builder()
            .application_name("Canon TS3350")
            .application_icon(APP_ID)
            .developer_name("Sofiane Lasri")
            .version(env!("CARGO_PKG_VERSION"))
            .comments(i18n("Multi-page scanning and manual duplex printing for the Canon Pixma TS3350."))
            .license_type(gtk::License::MitX11)
            .build()
            .present(Some(&window.ctx.window));
    });
    app.add_action(&about_action);

    let calibrate_action = gio::SimpleAction::new("calibrate", None);
    let (a, s) = (app.clone(), slot.clone());
    calibrate_action.connect_activate(move |_, _| {
        let window = main_window(&a, &s);
        window.present();
        window.stack.set_visible_child_name("duplex");
        window.duplex.start_calibration();
    });
    app.add_action(&calibrate_action);

    let quit_action = gio::SimpleAction::new("quit", None);
    let a = app.clone();
    quit_action.connect_activate(move |_, _| a.quit());
    app.add_action(&quit_action);

    // `gdbus call … --method org.gtk.Actions.Activate debug-snapshot "[<'/path.png'>]" {}`
    #[cfg(debug_assertions)]
    {
        let action = gio::SimpleAction::new("debug-snapshot", Some(glib::VariantTy::STRING));
        let (a, s) = (app.clone(), slot.clone());
        action.connect_activate(move |_, parameter| {
            if let Some(path) = parameter.and_then(|p| p.get::<String>()) {
                crate::util::save_snapshot(main_window(&a, &s).ctx.window.upcast_ref(), &path);
            }
        });
        app.add_action(&action);
    }

    let s = slot.clone();
    app.connect_activate(move |app| main_window(app, &s).present());

    let s = slot.clone();
    app.connect_open(move |app, files, _hint| {
        let window = main_window(app, &s);
        window.present();
        for path in files.iter().filter_map(|f| f.path()) {
            window.open_path(path);
        }
    });

    app.run()
}
