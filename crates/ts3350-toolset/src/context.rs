//! State shared by the views of the application.

use std::cell::RefCell;
use std::fmt::Display;

use adw::prelude::*;
use gtk::gio;
use ts3350_core::config::Config;
use ts3350_core::i18n::i18n;

pub struct AppContext {
    pub app: adw::Application,
    pub window: adw::ApplicationWindow,
    pub toasts: adw::ToastOverlay,
    pub config: RefCell<Config>,
}

impl AppContext {
    pub fn toast(&self, message: &str) {
        self.toasts.add_toast(adw::Toast::builder().title(message).use_markup(false).build());
    }

    /// Toast with an action button.
    pub fn toast_with_button(&self, message: &str, button: &str, on_click: impl Fn() + 'static) {
        let toast = adw::Toast::builder().title(message).use_markup(false).button_label(button).build();
        toast.connect_button_clicked(move |_| on_click());
        self.toasts.add_toast(toast);
    }

    pub fn error(&self, heading: &str, error: impl Display) {
        let dialog = adw::AlertDialog::new(Some(heading), Some(&error.to_string()));
        dialog.add_response("close", i18n("Close"));
        dialog.present(Some(&self.window));
    }

    /// Changes the configuration, then saves it to disk.
    pub fn update_config(&self, change: impl FnOnce(&mut Config)) {
        let mut config = self.config.borrow_mut();
        change(&mut config);
        if let Err(error) = config.save() {
            eprintln!("Cannot save the configuration: {error}");
        }
    }

    /// Sends a GNOME notification when the window is not focused.
    pub fn notify_if_inactive(&self, id: &str, title: &str, body: &str) {
        if self.window.is_active() {
            return;
        }
        let notification = gio::Notification::new(title);
        notification.set_body(Some(body));
        notification.set_icon(&gio::ThemedIcon::new(ts3350_core::APP_ID));
        self.app.send_notification(Some(id), &notification);
    }
}
