//! “Scan” tab: settings, platen preview and document pages.

mod export_dialog;
mod preview;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, glib};
use ts3350_core::escl::{
    self, Capabilities, Client, ColorMode, DiscoveredScanner, Paper, Region, ScanSettings, content_type_label,
    intent_label,
};
use ts3350_core::export::ScannedPage;
use ts3350_core::i18n::{i18n, i18n_f, ni18n_f};

use crate::context::AppContext;
use crate::util::{background, string_list, texture_from_rgba};
use preview::Preview;

/// Thumbnail size (longest side, in pixels).
const THUMBNAIL: u32 = 320;

struct PageEntry {
    page: ScannedPage,
    thumbnail: image::RgbaImage,
    texture: gdk::Texture,
}

impl PageEntry {
    fn new(page: ScannedPage, thumbnail: image::RgbaImage) -> PageEntry {
        let texture = texture_from_rgba(&thumbnail);
        PageEntry { page, thumbnail, texture }
    }

    fn rotate(&mut self) {
        self.page.rotation = self.page.rotation.clockwise();
        self.thumbnail = image::imageops::rotate90(&self.thumbnail);
        self.texture = texture_from_rgba(&self.thumbnail);
    }
}

pub struct ScanPage {
    pub root: adw::OverlaySplitView,
    ctx: Rc<AppContext>,

    scanner_row: adw::ComboRow,
    status_row: adw::ActionRow,
    status_spinner: adw::Spinner,
    refresh_button: gtk::Button,
    scanners: RefCell<Vec<DiscoveredScanner>>,
    client: RefCell<Option<Client>>,
    caps: RefCell<Option<Capabilities>>,

    mode_row: adw::ComboRow,
    modes: RefCell<Vec<ColorMode>>,
    resolution_row: adw::ComboRow,
    resolutions: RefCell<Vec<u32>>,
    intent_row: adw::ComboRow,
    intents: RefCell<Vec<String>>,
    paper_row: adw::ComboRow,
    content_row: adw::ComboRow,
    contents: RefCell<Vec<Option<String>>>,
    sharpen_row: adw::SwitchRow,
    background_row: adw::SwitchRow,
    quality_row: adw::SwitchRow,
    settings_groups: Vec<adw::PreferencesGroup>,

    preview_button: gtk::Button,
    scan_button: gtk::Button,
    scan_label: gtk::Label,
    scan_spinner: adw::Spinner,
    preview: Rc<Preview>,

    pages: RefCell<Vec<PageEntry>>,
    pages_box: gtk::Box,
    pages_title: gtk::Label,
    export_button: gtk::Button,
    clear_button: gtk::Button,

    busy: Cell<bool>,
    /// True while the lists are being filled: “selected” signals are ignored.
    updating: Cell<bool>,
}

impl ScanPage {
    pub fn new(ctx: Rc<AppContext>) -> Rc<ScanPage> {
        // --- Scanner ---
        let scanner_row =
            adw::ComboRow::builder().title(i18n("Scanner")).model(&gtk::StringList::new(&[i18n("Searching…")])).build();
        scanner_row.set_sensitive(false);
        let refresh_button = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text(i18n("Search for scanners again"))
            .valign(gtk::Align::Center)
            .build();
        refresh_button.add_css_class("flat");
        scanner_row.add_suffix(&refresh_button);
        let status_spinner = adw::Spinner::new();
        let status_row =
            adw::ActionRow::builder().title(i18n("Status")).subtitle(i18n("Looking for the scanner…")).build();
        status_row.add_css_class("property");
        status_row.add_suffix(&status_spinner);
        let scanner_group = adw::PreferencesGroup::builder().title(i18n("Scanner")).build();
        scanner_group.add(&scanner_row);
        scanner_group.add(&status_row);

        // --- Main settings ---
        let mode_row = adw::ComboRow::builder().title(i18n("Color")).build();
        let resolution_row = adw::ComboRow::builder().title(i18n("Resolution")).subtitle(i18n("Dots per inch")).build();
        let intent_row = adw::ComboRow::builder().title(i18n("Document type")).build();
        let paper_labels: Vec<String> = Paper::ALL.iter().map(|p| p.label().to_owned()).collect();
        let paper_row = adw::ComboRow::builder().title(i18n("Paper size")).model(&string_list(&paper_labels)).build();
        let main_group = adw::PreferencesGroup::builder().title(i18n("Scanning")).build();
        for row in [&mode_row, &resolution_row, &intent_row, &paper_row] {
            main_group.add(row);
        }

        // --- Advanced settings ---
        let content_row = adw::ComboRow::builder().title(i18n("Image processing")).build();
        let sharpen_row = adw::SwitchRow::builder().title(i18n("Sharpen")).build();
        let background_row = adw::SwitchRow::builder()
            .title(i18n("Background reduction"))
            .subtitle(i18n("Whitens the paper background (text documents)"))
            .build();
        let quality_row = adw::SwitchRow::builder()
            .title(i18n("Maximum quality"))
            .subtitle(i18n("Less JPEG compression on the scanner side, bigger files"))
            .build();
        let advanced_group = adw::PreferencesGroup::builder().title(i18n("Advanced settings")).build();
        for row in [
            content_row.upcast_ref::<gtk::Widget>(),
            sharpen_row.upcast_ref(),
            background_row.upcast_ref(),
            quality_row.upcast_ref(),
        ] {
            advanced_group.add(row);
        }

        // --- Buttons ---
        let preview_button = gtk::Button::builder()
            .label(i18n("Preview"))
            .tooltip_text(i18n("Quick low-resolution scan to choose the area"))
            .build();
        preview_button.add_css_class("pill");
        let scan_spinner = adw::Spinner::builder().visible(false).build();
        let scan_label = gtk::Label::new(Some(i18n("Scan a page")));
        let scan_content = gtk::Box::builder().spacing(8).halign(gtk::Align::Center).build();
        scan_content.append(&scan_spinner);
        scan_content.append(&scan_label);
        let scan_button = gtk::Button::builder()
            .child(&scan_content)
            .tooltip_text(i18n("Scans the glass and adds the page to the document"))
            .build();
        scan_button.add_css_class("pill");
        scan_button.add_css_class("suggested-action");
        let buttons = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).build();
        buttons.append(&scan_button);
        buttons.append(&preview_button);

        let sidebar_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(24)
            .margin_top(18)
            .margin_bottom(18)
            .margin_start(18)
            .margin_end(18)
            .build();
        sidebar_box.append(&scanner_group);
        sidebar_box.append(&main_group);
        sidebar_box.append(&advanced_group);
        sidebar_box.append(&buttons);
        let sidebar =
            gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&sidebar_box).build();

        // --- Main area: preview + pages ---
        let preview = Preview::new();
        let pages_title = gtk::Label::builder().label(i18n("Document")).xalign(0.0).hexpand(true).build();
        pages_title.add_css_class("heading");
        let clear_button =
            gtk::Button::builder().icon_name("user-trash-symbolic").tooltip_text(i18n("Clear all pages")).build();
        clear_button.add_css_class("flat");
        let export_button = gtk::Button::builder().label(i18n("Export…")).build();
        export_button.add_css_class("suggested-action");
        let pages_header = gtk::Box::builder().spacing(6).margin_start(18).margin_end(18).margin_top(12).build();
        pages_header.append(&pages_title);
        pages_header.append(&clear_button);
        pages_header.append(&export_button);
        let pages_box =
            gtk::Box::builder().spacing(12).margin_start(18).margin_end(18).margin_top(6).margin_bottom(12).build();
        let pages_scroll = gtk::ScrolledWindow::builder()
            .vscrollbar_policy(gtk::PolicyType::Never)
            .min_content_height(270)
            .vexpand(false)
            .child(&pages_box)
            .build();
        let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        content.append(&preview.root);
        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        content.append(&pages_header);
        content.append(&pages_scroll);

        let root = adw::OverlaySplitView::builder()
            .sidebar(&sidebar)
            .content(&content)
            .min_sidebar_width(340.0)
            .max_sidebar_width(400.0)
            .sidebar_width_fraction(0.32)
            .build();

        let page = Rc::new(ScanPage {
            root,
            ctx,
            scanner_row,
            status_row,
            status_spinner,
            refresh_button,
            scanners: RefCell::new(Vec::new()),
            client: RefCell::new(None),
            caps: RefCell::new(None),
            mode_row,
            modes: RefCell::new(Vec::new()),
            resolution_row,
            resolutions: RefCell::new(Vec::new()),
            intent_row,
            intents: RefCell::new(Vec::new()),
            paper_row,
            content_row,
            contents: RefCell::new(Vec::new()),
            sharpen_row,
            background_row,
            quality_row,
            settings_groups: vec![main_group, advanced_group],
            preview_button,
            scan_button,
            scan_label,
            scan_spinner,
            preview,
            pages: RefCell::new(Vec::new()),
            pages_box,
            pages_title,
            export_button,
            clear_button,
            busy: Cell::new(false),
            updating: Cell::new(false),
        });
        page.connect_signals();
        page.set_controls_sensitive(false);
        page.rebuild_pages();
        page.start();
        page
    }

    fn connect_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.refresh_button.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.discover();
            }
        });

        let weak = Rc::downgrade(self);
        self.scanner_row.connect_selected_notify(move |row| {
            let Some(page) = weak.upgrade() else { return };
            if page.updating.get() {
                return;
            }
            let url = page.scanners.borrow().get(row.selected() as usize).map(|s| s.url.clone());
            if let Some(url) = url {
                page.connect(url);
            }
        });

        for row in [&self.mode_row, &self.resolution_row, &self.intent_row, &self.content_row] {
            let weak = Rc::downgrade(self);
            row.connect_selected_notify(move |_| {
                if let Some(page) = weak.upgrade() {
                    page.save_prefs();
                }
            });
        }
        let weak = Rc::downgrade(self);
        self.paper_row.connect_selected_notify(move |_| {
            if let Some(page) = weak.upgrade()
                && !page.updating.get()
            {
                page.save_prefs();
                page.update_selection();
            }
        });
        for row in [&self.sharpen_row, &self.background_row, &self.quality_row] {
            let weak = Rc::downgrade(self);
            row.connect_active_notify(move |_| {
                if let Some(page) = weak.upgrade() {
                    page.save_prefs();
                }
            });
        }

        let weak = Rc::downgrade(self);
        self.preview.connect_select(move |region| {
            let Some(page) = weak.upgrade() else { return };
            page.ctx.update_config(|c| {
                c.scan.paper = Paper::Custom;
                c.scan.custom_region = Some(region);
            });
            page.updating.set(true);
            page.paper_row.set_selected(Paper::ALL.iter().position(|p| *p == Paper::Custom).unwrap_or(0) as u32);
            page.updating.set(false);
        });

        let weak = Rc::downgrade(self);
        self.preview_button.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.run_preview();
            }
        });
        let weak = Rc::downgrade(self);
        self.scan_button.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.run_scan();
            }
        });
        let weak = Rc::downgrade(self);
        self.export_button.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                let pages: Vec<ScannedPage> = page.pages.borrow().iter().map(|e| e.page.clone()).collect();
                export_dialog::show(page.ctx.clone(), pages);
            }
        });
        let weak = Rc::downgrade(self);
        self.clear_button.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.confirm_clear();
            }
        });
    }

    // ------------------------------------------------------------------
    // Scanner connection
    // ------------------------------------------------------------------

    fn start(self: &Rc<Self>) {
        let config = self.ctx.config.borrow().clone();
        let known = config.scanner_url.clone().filter(|u| !u.is_empty()).or(config.last_scanner_url.clone());
        if let Some(url) = known {
            // Connect straight to the last scanner: no need to wait for mDNS.
            self.connect(url);
        }
        self.discover();
    }

    /// Reconnects (after the address changed in the preferences).
    pub fn reconnect(self: &Rc<Self>) {
        *self.client.borrow_mut() = None;
        *self.caps.borrow_mut() = None;
        self.start();
    }

    fn discover(self: &Rc<Self>) {
        if self.client.borrow().is_none() {
            self.set_status(i18n("Looking for the scanner on the network…"), true);
        }
        self.refresh_button.set_sensitive(false);
        let page = self.clone();
        glib::spawn_future_local(async move {
            let found = background(|| escl::discover(Duration::from_secs(6))).await;
            page.refresh_button.set_sensitive(true);
            page.set_scanners(found);
        });
    }

    fn set_scanners(self: &Rc<Self>, found: Vec<DiscoveredScanner>) {
        let config = self.ctx.config.borrow().clone();
        let current = self.client.borrow().as_ref().map(|c| c.base_url().to_owned());

        let mut list = Vec::new();
        if let Some(url) = config.scanner_url.clone().filter(|u| !u.is_empty()) {
            list.push(DiscoveredScanner { name: i18n("Manual address").into(), url });
        }
        for scanner in found {
            if !list.iter().any(|s: &DiscoveredScanner| same_url(&s.url, &scanner.url)) {
                list.push(scanner);
            }
        }
        if let Some(url) = &current
            && !list.iter().any(|s| same_url(&s.url, url))
        {
            let name =
                self.caps.borrow().as_ref().map(|c| c.make_and_model.clone()).unwrap_or_else(|| i18n("Scanner").into());
            list.push(DiscoveredScanner { name, url: url.clone() });
        }

        if list.is_empty() {
            self.updating.set(true);
            self.scanner_row.set_model(Some(&gtk::StringList::new(&[i18n("No scanner")])));
            self.updating.set(false);
            self.scanner_row.set_sensitive(false);
            self.set_status(
                i18n("No scanner found. Check that the printer is on and connected to the same network."),
                false,
            );
            return;
        }

        let preferred = current.clone().or(config.scanner_url).or(config.last_scanner_url);
        let index = preferred.and_then(|u| list.iter().position(|s| same_url(&s.url, &u))).unwrap_or(0);
        let names: Vec<String> = list.iter().map(|s| s.name.clone()).collect();
        let url = list[index].url.clone();
        *self.scanners.borrow_mut() = list;

        self.updating.set(true);
        self.scanner_row.set_model(Some(&string_list(&names)));
        self.scanner_row.set_selected(index as u32);
        self.updating.set(false);
        self.scanner_row.set_sensitive(!self.busy.get());

        if !current.is_some_and(|c| same_url(&c, &url)) {
            self.connect(url);
        }
    }

    fn connect(self: &Rc<Self>, url: String) {
        self.set_status(i18n("Connecting to the scanner…"), true);
        self.set_controls_sensitive(false);
        let page = self.clone();
        glib::spawn_future_local(async move {
            let client = Client::new(&url);
            let probe = client.clone();
            match background(move || probe.capabilities()).await {
                Ok(caps) => {
                    page.ctx.update_config(|c| c.last_scanner_url = Some(url));
                    *page.client.borrow_mut() = Some(client);
                    page.apply_caps(caps);
                }
                Err(error) => {
                    if page.client.borrow().is_none() {
                        page.set_status(&i18n_f("Scanner unreachable: {error}", &[("error", &error)]), false);
                    }
                }
            }
        });
    }

    fn apply_caps(self: &Rc<Self>, caps: Capabilities) {
        let Some(input) = caps.input().cloned() else {
            self.set_status(i18n("This scanner has no usable platen or feeder."), false);
            return;
        };
        let prefs = self.ctx.config.borrow().scan.clone();
        self.updating.set(true);

        let mut modes = input.color_modes.clone();
        if modes.is_empty() {
            modes.push(ColorMode::Rgb24);
        }
        // Colour first: it is the most common choice.
        modes.sort_by_key(|m| *m != ColorMode::Rgb24);
        let labels: Vec<String> = modes.iter().map(|m| m.label().to_owned()).collect();
        self.mode_row.set_model(Some(&string_list(&labels)));
        self.mode_row.set_selected(modes.iter().position(|m| *m == prefs.color_mode).unwrap_or(0) as u32);
        *self.modes.borrow_mut() = modes;

        let mut resolutions = input.resolutions.clone();
        if resolutions.is_empty() {
            resolutions.push(300);
        }
        let labels: Vec<String> = resolutions.iter().map(|r| i18n_f("{dpi} dpi", &[("dpi", r)])).collect();
        self.resolution_row.set_model(Some(&string_list(&labels)));
        let wanted = if resolutions.contains(&prefs.resolution) {
            prefs.resolution
        } else {
            input.default_resolution.unwrap_or(300)
        };
        let index = resolutions.iter().enumerate().min_by_key(|(_, r)| r.abs_diff(wanted)).map(|(i, _)| i).unwrap_or(0);
        self.resolution_row.set_selected(index as u32);
        *self.resolutions.borrow_mut() = resolutions;

        let intents: Vec<String> = input.intents.iter().filter(|i| *i != "Preview").cloned().collect();
        let labels: Vec<String> = intents.iter().map(|i| intent_label(i)).collect();
        self.intent_row.set_model(Some(&string_list(&labels)));
        self.intent_row.set_visible(!intents.is_empty());
        let index = prefs.intent.as_ref().and_then(|p| intents.iter().position(|i| i == p)).unwrap_or(0);
        self.intent_row.set_selected(index as u32);
        *self.intents.borrow_mut() = intents;

        let mut contents: Vec<Option<String>> = vec![None];
        contents.extend(input.content_types.iter().filter(|c| *c != "Auto").cloned().map(Some));
        let labels: Vec<String> = contents
            .iter()
            .map(|c| c.as_deref().map(content_type_label).unwrap_or_else(|| i18n("Automatic").into()))
            .collect();
        self.content_row.set_model(Some(&string_list(&labels)));
        self.content_row.set_visible(contents.len() > 1);
        self.content_row.set_selected(contents.iter().position(|c| *c == prefs.content_type).unwrap_or(0) as u32);
        *self.contents.borrow_mut() = contents;

        self.paper_row.set_selected(Paper::ALL.iter().position(|p| *p == prefs.paper).unwrap_or(0) as u32);
        self.sharpen_row.set_visible(caps.sharpen.is_some_and(|r| r.max > r.min));
        self.sharpen_row.set_active(prefs.sharpen);
        self.background_row.set_visible(caps.background_reduction);
        self.background_row.set_active(prefs.background_reduction);
        self.quality_row.set_visible(caps.compression.is_some_and(|r| r.max > r.min));
        self.quality_row.set_active(prefs.high_quality);

        self.preview.set_platen(input.max_width, input.max_height);
        let name = caps.make_and_model.clone();
        *self.caps.borrow_mut() = Some(caps);
        self.updating.set(false);

        self.save_prefs();
        self.update_selection();
        self.set_status(&i18n_f("{name} — ready", &[("name", &name)]), false);
        self.set_controls_sensitive(true);
    }

    // ------------------------------------------------------------------
    // Settings
    // ------------------------------------------------------------------

    fn save_prefs(&self) {
        if self.updating.get() || self.caps.borrow().is_none() {
            return;
        }
        let pick = |row: &adw::ComboRow| row.selected() as usize;
        let color_mode = self.modes.borrow().get(pick(&self.mode_row)).copied();
        let resolution = self.resolutions.borrow().get(pick(&self.resolution_row)).copied();
        let intent = self.intents.borrow().get(pick(&self.intent_row)).cloned();
        let content_type = self.contents.borrow().get(pick(&self.content_row)).cloned();
        let paper = Paper::ALL.get(pick(&self.paper_row)).copied();
        self.ctx.update_config(|c| {
            let scan = &mut c.scan;
            if let Some(mode) = color_mode {
                scan.color_mode = mode;
            }
            if let Some(resolution) = resolution {
                scan.resolution = resolution;
            }
            if intent.is_some() {
                scan.intent = intent;
            }
            if let Some(content_type) = content_type {
                scan.content_type = content_type;
            }
            if let Some(paper) = paper {
                scan.paper = paper;
            }
            scan.sharpen = self.sharpen_row.is_active();
            scan.background_reduction = self.background_row.is_active();
            scan.high_quality = self.quality_row.is_active();
        });
    }

    /// Area to scan for the chosen paper size.
    fn region(&self) -> Option<Region> {
        let caps = self.caps.borrow();
        let input = caps.as_ref()?.input()?;
        let prefs = &self.ctx.config.borrow().scan;
        let region = match prefs.paper {
            Paper::Custom => prefs.custom_region.unwrap_or_else(|| Region::full(input)),
            paper => paper.region(input)?,
        };
        Some(region.clamp_to(input))
    }

    fn update_selection(&self) {
        self.preview.set_selection(self.region());
    }

    fn current_settings(&self) -> Option<(Client, ScanSettings, String)> {
        let client = self.client.borrow().clone()?;
        let caps = self.caps.borrow();
        let caps = caps.as_ref()?;
        let input = caps.input()?;
        let prefs = self.ctx.config.borrow().scan.clone();
        let settings = ScanSettings {
            color_mode: prefs.color_mode,
            resolution: prefs.resolution,
            intent: prefs.intent.filter(|i| input.intents.contains(i)),
            content_type: prefs.content_type.filter(|c| input.content_types.contains(c)),
            region: self.region()?,
            sharpen: caps.sharpen.map(|r| if prefs.sharpen { r.max } else { r.min }),
            compression: caps.compression.map(|r| if prefs.high_quality { r.min } else { r.normal }),
            background_reduction: prefs.background_reduction && caps.background_reduction,
        };
        Some((client, settings, caps.version.clone()))
    }

    // ------------------------------------------------------------------
    // Scanning
    // ------------------------------------------------------------------

    fn run_preview(self: &Rc<Self>) {
        let Some((client, mut settings, version)) = self.current_settings() else { return };
        let caps = self.caps.borrow().clone();
        let Some(input) = caps.as_ref().and_then(|c| c.input()).cloned() else { return };
        settings.resolution = input.resolutions.first().copied().unwrap_or(75);
        settings.region = Region::full(&input);
        settings.color_mode =
            if input.color_modes.contains(&ColorMode::Rgb24) { ColorMode::Rgb24 } else { settings.color_mode };
        if input.intents.iter().any(|i| i == "Preview") {
            settings.intent = Some("Preview".into());
        }
        settings.background_reduction = false;

        self.set_busy(Some(i18n("Previewing…")));
        let page = self.clone();
        glib::spawn_future_local(async move {
            let result = background(move || -> ts3350_core::Result<image::RgbaImage> {
                let jpeg = client.scan(&settings, &version)?;
                Ok(image::load_from_memory(&jpeg)?.to_rgba8())
            })
            .await;
            page.set_busy(None);
            match result {
                Ok(image) => page.preview.set_image(&image),
                Err(error) => page.ctx.error(i18n("Preview failed"), error),
            }
        });
    }

    fn run_scan(self: &Rc<Self>) {
        let Some((client, settings, version)) = self.current_settings() else { return };
        let dpi = settings.resolution;
        self.set_busy(Some(i18n("Scanning…")));
        let page = self.clone();
        glib::spawn_future_local(async move {
            let result = background(move || -> ts3350_core::Result<(ScannedPage, image::RgbaImage)> {
                let jpeg = client.scan(&settings, &version)?;
                let scanned = ScannedPage::new(jpeg, dpi);
                let thumbnail = scanned.thumbnail(THUMBNAIL)?;
                Ok((scanned, thumbnail))
            })
            .await;
            page.set_busy(None);
            match result {
                Ok((scanned, thumbnail)) => {
                    page.pages.borrow_mut().push(PageEntry::new(scanned, thumbnail));
                    page.rebuild_pages();
                    let count = page.pages.borrow().len();
                    page.ctx.toast(&i18n_f("Page {number} added to the document", &[("number", &count)]));
                }
                Err(error) => page.ctx.error(i18n("Scan failed"), error),
            }
        });
    }

    fn set_status(&self, text: &str, working: bool) {
        self.status_row.set_subtitle(text);
        self.status_spinner.set_visible(working);
    }

    fn set_controls_sensitive(&self, sensitive: bool) {
        let ready = sensitive && self.caps.borrow().is_some() && !self.busy.get();
        for group in &self.settings_groups {
            group.set_sensitive(ready);
        }
        self.preview_button.set_sensitive(ready);
        self.scan_button.set_sensitive(ready);
    }

    fn set_busy(&self, label: Option<&str>) {
        self.busy.set(label.is_some());
        self.scan_spinner.set_visible(label.is_some());
        self.scan_label.set_label(label.unwrap_or(i18n("Scan a page")));
        self.scanner_row.set_sensitive(label.is_none() && !self.scanners.borrow().is_empty());
        self.refresh_button.set_sensitive(label.is_none());
        self.set_controls_sensitive(label.is_none());
    }

    // ------------------------------------------------------------------
    // Document pages
    // ------------------------------------------------------------------

    fn rebuild_pages(self: &Rc<Self>) {
        while let Some(child) = self.pages_box.first_child() {
            self.pages_box.remove(&child);
        }
        let pages = self.pages.borrow();
        self.pages_title.set_label(&if pages.is_empty() {
            i18n("Document").to_owned()
        } else {
            let count = pages.len();
            ni18n_f("Document — {count} page", "Document — {count} pages", count as u64, &[("count", &count)])
        });
        self.export_button.set_sensitive(!pages.is_empty());
        self.clear_button.set_sensitive(!pages.is_empty());

        if pages.is_empty() {
            let empty = gtk::Label::builder()
                .label(i18n("No page yet: put the original face down on the glass,\naligned with the corner marked in orange, then click “Scan a page”."))
                .justify(gtk::Justification::Center)
                .hexpand(true)
                .build();
            empty.add_css_class("dim-label");
            self.pages_box.append(&empty);
            return;
        }

        let count = pages.len();
        for (index, entry) in pages.iter().enumerate() {
            // Fixed square: a landscape page must not widen its card.
            let picture = gtk::Image::builder().paintable(&entry.texture).pixel_size(180).build();
            let label = gtk::Label::new(Some(&i18n_f(
                "Page {number} · {dpi} dpi",
                &[("number", &(index + 1)), ("dpi", &entry.page.dpi)],
            )));
            label.add_css_class("caption");

            let actions = gtk::Box::builder().spacing(2).halign(gtk::Align::Center).build();
            let buttons = [
                ("object-rotate-right-symbolic", i18n("Rotate by 90°"), true, PageAction::Rotate),
                ("go-previous-symbolic", i18n("Move left"), index > 0, PageAction::MoveLeft),
                ("go-next-symbolic", i18n("Move right"), index + 1 < count, PageAction::MoveRight),
                ("user-trash-symbolic", i18n("Delete the page"), true, PageAction::Delete),
            ];
            for (icon, tooltip, sensitive, action) in buttons {
                let button = gtk::Button::builder().icon_name(icon).tooltip_text(tooltip).sensitive(sensitive).build();
                button.add_css_class("flat");
                button.add_css_class("circular");
                let weak = Rc::downgrade(self);
                button.connect_clicked(move |_| {
                    if let Some(page) = weak.upgrade() {
                        page.page_action(index, action);
                    }
                });
                actions.append(&button);
            }

            let card = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(6)
                .valign(gtk::Align::Start)
                .build();
            card.add_css_class("card");
            let inner = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(6)
                .margin_top(10)
                .margin_bottom(6)
                .margin_start(10)
                .margin_end(10)
                .build();
            inner.append(&picture);
            inner.append(&label);
            inner.append(&actions);
            card.append(&inner);
            self.pages_box.append(&card);
        }
    }

    fn page_action(self: &Rc<Self>, index: usize, action: PageAction) {
        {
            let mut pages = self.pages.borrow_mut();
            if index >= pages.len() {
                return;
            }
            match action {
                PageAction::Rotate => pages[index].rotate(),
                PageAction::MoveLeft if index > 0 => pages.swap(index, index - 1),
                PageAction::MoveRight if index + 1 < pages.len() => pages.swap(index, index + 1),
                PageAction::Delete => {
                    pages.remove(index);
                }
                _ => {}
            }
        }
        self.rebuild_pages();
    }

    fn confirm_clear(self: &Rc<Self>) {
        let count = self.pages.borrow().len();
        let dialog = adw::AlertDialog::new(
            Some(i18n("Clear the document?")),
            Some(&ni18n_f(
                "{count} page will be deleted. Remember to export the document first.",
                "{count} pages will be deleted. Remember to export the document first.",
                count as u64,
                &[("count", &count)],
            )),
        );
        dialog.add_responses(&[("cancel", i18n("Cancel")), ("clear", i18n("Clear"))]);
        dialog.set_response_appearance("clear", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        let page = self.clone();
        glib::spawn_future_local(async move {
            if dialog.choose_future(Some(&page.ctx.window)).await == "clear" {
                page.pages.borrow_mut().clear();
                page.rebuild_pages();
            }
        });
    }
}

#[derive(Clone, Copy)]
enum PageAction {
    Rotate,
    MoveLeft,
    MoveRight,
    Delete,
}

fn same_url(a: &str, b: &str) -> bool {
    let normalize = |u: &str| u.trim_end_matches('/').replace(":80/", "/").to_ascii_lowercase();
    normalize(a) == normalize(b)
}
