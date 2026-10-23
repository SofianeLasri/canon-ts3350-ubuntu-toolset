use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;
use ts3350_core::i18n::i18n;

use crate::context::AppContext;
use crate::duplex::DuplexPage;
use crate::scan::ScanPage;

pub struct MainWindow {
    pub ctx: Rc<AppContext>,
    pub stack: adw::ViewStack,
    pub scan: Rc<ScanPage>,
    pub duplex: Rc<DuplexPage>,
}

impl MainWindow {
    pub fn new(app: &adw::Application) -> Rc<MainWindow> {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Canon TS3350")
            .default_width(1200)
            .default_height(820)
            .width_request(900)
            .height_request(600)
            .build();
        let toasts = adw::ToastOverlay::new();
        let ctx = Rc::new(AppContext {
            app: app.clone(),
            window: window.clone(),
            toasts: toasts.clone(),
            config: std::cell::RefCell::new(ts3350_core::config::Config::load()),
        });

        let scan = ScanPage::new(ctx.clone());
        let duplex = DuplexPage::new(ctx.clone());
        let stack = adw::ViewStack::new();
        stack.add_titled_with_icon(&scan.root, Some("scan"), i18n("Scan"), "scanner-symbolic");
        stack.add_titled_with_icon(&duplex.root, Some("duplex"), i18n("Duplex"), "printer-symbolic");

        let switcher = adw::ViewSwitcher::builder().stack(&stack).policy(adw::ViewSwitcherPolicy::Wide).build();
        let header = adw::HeaderBar::builder().title_widget(&switcher).build();
        let menu = gio::Menu::new();
        let main_section = gio::Menu::new();
        main_section.append(Some(i18n("Preferences")), Some("app.preferences"));
        main_section.append(Some(i18n("About Canon TS3350")), Some("app.about"));
        menu.append_section(None, &main_section);
        let quit_section = gio::Menu::new();
        quit_section.append(Some(i18n("Quit")), Some("app.quit"));
        menu.append_section(None, &quit_section);
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .primary(true)
            .tooltip_text(i18n("Main menu"))
            .build();
        header.pack_end(&menu_button);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&stack));
        toasts.set_child(Some(&toolbar));
        window.set_content(Some(&toasts));

        let duplex_page = duplex.clone();
        stack.connect_visible_child_name_notify(move |stack| {
            if stack.visible_child_name().as_deref() == Some("duplex") {
                duplex_page.refresh();
            }
        });

        Rc::new(MainWindow { ctx, stack, scan, duplex })
    }

    pub fn present(&self) {
        self.ctx.window.present();
    }

    /// PDF received through “Open with”, the command line or the CUPS backend.
    pub fn open_path(&self, path: PathBuf) {
        self.stack.set_visible_child_name("duplex");
        self.duplex.open_path(path);
    }
}
