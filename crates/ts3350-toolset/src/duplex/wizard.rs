//! Manual duplex wizard: fronts → stack flip → backs.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use ts3350_core::cups::{self, JobProgress, PrintOptions};
use ts3350_core::duplex::{self, Binding, PassPlan, SourceDocument};
use ts3350_core::i18n::{i18n, i18n_f, n_, ni18n_f};
use ts3350_core::spool::SpoolJob;

use super::illustration;
use crate::context::AppContext;
use crate::util::{background, runtime_dir};

const SIZES: [(&str, &str); 7] = [
    ("A4", "A4"),
    ("A5", "A5"),
    ("B5", "JIS B5"),
    ("Letter", n_("US Letter")),
    ("Legal", n_("US Legal")),
    ("4x6", n_("Photo 10 × 15 cm")),
    ("5x7", n_("Photo 13 × 18 cm")),
];
const QUALITIES: [(&str, &str); 2] = [("Normal", n_("Normal")), ("High", n_("High"))];
const COLORS: [(&str, &str); 2] = [("RGB", n_("Color")), ("Gray", n_("Grayscale"))];
const MEDIA: [(&str, &str); 2] = [("Stationery", n_("Plain paper")), ("Photographic", n_("Photo paper"))];
const BINDINGS: [Binding; 2] = [Binding::LongEdge, Binding::ShortEdge];

/// Document to print and initial settings.
pub struct WizardJob {
    pub pdf: PathBuf,
    pub title: String,
    pub binding: Binding,
    pub options: PrintOptions,
    pub target_queue: Option<String>,
    pub spool: Option<SpoolJob>,
    pub calibration: bool,
}

impl WizardJob {
    pub fn from_spool(job: SpoolJob) -> WizardJob {
        WizardJob {
            pdf: job.pdf.clone(),
            title: job.meta.title.clone(),
            binding: job.meta.binding,
            options: job.meta.options.clone(),
            target_queue: Some(job.meta.target_queue.clone()),
            spool: Some(job),
            calibration: false,
        }
    }

    pub fn from_file(path: PathBuf) -> WizardJob {
        let title =
            path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| i18n("Document").into());
        WizardJob {
            pdf: path,
            title,
            binding: Binding::LongEdge,
            options: PrintOptions::default(),
            target_queue: None,
            spool: None,
            calibration: false,
        }
    }

    pub fn calibration() -> ts3350_core::Result<WizardJob> {
        let pdf = runtime_dir().join("duplex-test.pdf");
        std::fs::write(&pdf, duplex::calibration_pdf()?)?;
        Ok(WizardJob {
            pdf,
            title: i18n("Duplex test").into(),
            binding: Binding::LongEdge,
            options: PrintOptions {
                page_size: Some("A4".into()),
                media_type: Some("Stationery".into()),
                ..Default::default()
            },
            target_queue: None,
            spool: None,
            calibration: true,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Summary,
    Fronts,
    Flip,
    Backs,
    Done,
}

struct Passes {
    plan: PassPlan,
    fronts: PathBuf,
    backs: PathBuf,
    queue: String,
    options: PrintOptions,
}

/// Drop-down list mapped to CUPS keywords.
struct Choice {
    row: adw::ComboRow,
    keys: Vec<String>,
}

impl Choice {
    fn new(title: &str, entries: &[(&str, &'static str)], current: Option<&str>) -> Choice {
        let mut keys: Vec<String> = entries.iter().map(|(k, _)| (*k).to_owned()).collect();
        let mut labels: Vec<&str> = entries.iter().map(|(_, l)| i18n(l)).collect();
        // Value coming from the print dialog but missing from our list: keep it.
        if let Some(current) = current
            && !keys.iter().any(|k| k == current)
        {
            keys.push(current.to_owned());
            labels.push(current);
        }
        let row = adw::ComboRow::builder().title(title).model(&gtk::StringList::new(&labels)).build();
        if let Some(index) = current.and_then(|c| keys.iter().position(|k| k == c)) {
            row.set_selected(index as u32);
        }
        Choice { row, keys }
    }

    fn value(&self) -> Option<String> {
        self.keys.get(self.row.selected() as usize).cloned()
    }
}

struct Wizard {
    ctx: Rc<AppContext>,
    job: WizardJob,
    dialog: adw::Dialog,
    nav: adw::NavigationView,
    on_finished: Rc<dyn Fn()>,
    stage: Cell<Stage>,
    closed: Cell<bool>,
    page_count: Cell<usize>,
    passes: RefCell<Option<Passes>>,
    cups_job: RefCell<Option<(String, String)>>,

    summary_icon: gtk::Image,
    summary_text: gtk::Label,
    binding: adw::ComboRow,
    size: Choice,
    quality: Choice,
    color: Choice,
    media: Choice,
    options_group: adw::PreferencesGroup,
    start_button: gtk::Button,
    root_page: RefCell<Option<adw::NavigationPage>>,
    progress: RefCell<Option<ProgressWidgets>>,
}

/// Labels of the current progress page.
struct ProgressWidgets {
    status: gtk::Label,
    problem: gtk::Label,
}

pub fn open(ctx: Rc<AppContext>, job: WizardJob, on_finished: Rc<dyn Fn()>) {
    let binding = adw::ComboRow::builder()
        .title(i18n("Binding"))
        .model(&gtk::StringList::new(&BINDINGS.map(Binding::label)))
        .build();
    binding.set_selected(BINDINGS.iter().position(|b| *b == job.binding).unwrap_or(0) as u32);
    let options = &job.options;
    let size = Choice::new(i18n("Paper size"), &SIZES, options.page_size.as_deref().or(Some("A4")));
    let quality = Choice::new(i18n("Quality"), &QUALITIES, options.print_quality.as_deref());
    let color = Choice::new(i18n("Color"), &COLORS, options.color_model.as_deref());
    let media = Choice::new(i18n("Paper type"), &MEDIA, options.media_type.as_deref());

    let dialog = adw::Dialog::builder()
        .title(if job.calibration { i18n("Duplex test") } else { i18n("Two-sided printing") })
        .content_width(560)
        .content_height(640)
        .build();
    let nav = adw::NavigationView::new();
    dialog.set_child(Some(&nav));

    let wizard = Rc::new(Wizard {
        ctx,
        job,
        dialog,
        nav,
        on_finished,
        stage: Cell::new(Stage::Summary),
        closed: Cell::new(false),
        page_count: Cell::new(0),
        passes: RefCell::new(None),
        cups_job: RefCell::new(None),
        summary_icon: gtk::Image::from_icon_name("printer-symbolic"),
        summary_text: gtk::Label::new(Some(i18n("Reading the document…"))),
        binding,
        size,
        quality,
        color,
        media,
        options_group: adw::PreferencesGroup::builder().title(i18n("Settings")).build(),
        start_button: gtk::Button::new(),
        root_page: RefCell::new(None),
        progress: RefCell::new(None),
    });
    wizard.build_summary();
    wizard.connect_close();
    // Signal handlers only hold weak references: the dialog keeps the wizard
    // alive until it is closed.
    let keep_alive = RefCell::new(Some(wizard.clone()));
    wizard.dialog.connect_closed(move |_| {
        keep_alive.borrow_mut().take();
    });
    wizard.dialog.present(Some(&wizard.ctx.window));
    wizard.load();
}

fn page(title: &str, content: &impl IsA<gtk::Widget>) -> adw::NavigationPage {
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(content));
    adw::NavigationPage::builder().title(title).child(&toolbar).can_pop(false).build()
}

/// “3 sheets to print”
fn sheets_to_print(sheets: usize) -> String {
    ni18n_f("{count} sheet to print", "{count} sheets to print", sheets as u64, &[("count", &sheets)])
}

fn pill(label: &str, suggested: bool) -> gtk::Button {
    let button = gtk::Button::builder().label(label).halign(gtk::Align::Center).build();
    button.add_css_class("pill");
    if suggested {
        button.add_css_class("suggested-action");
    }
    button
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let clamp = adw::Clamp::builder().maximum_size(480).child(child).build();
    gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&clamp).build()
}

impl Wizard {
    // ------------------------------------------------------------------
    // Step 1: summary
    // ------------------------------------------------------------------

    fn build_summary(self: &Rc<Self>) {
        // No AdwStatusPage here: its inner scrolled window would be clipped inside the box.
        self.summary_icon.set_pixel_size(64);
        self.summary_icon.add_css_class("dim-label");
        let title = gtk::Label::builder().label(&self.job.title).wrap(true).justify(gtk::Justification::Center).build();
        title.add_css_class("title-1");
        self.summary_text.set_wrap(true);
        self.summary_text.set_justify(gtk::Justification::Center);

        self.options_group.add(&self.binding);
        for choice in [&self.size, &self.quality, &self.color, &self.media] {
            self.options_group.add(&choice.row);
        }
        self.options_group.set_visible(!self.job.calibration);
        self.start_button.set_label(i18n("Print the fronts"));
        self.start_button.add_css_class("pill");
        self.start_button.add_css_class("suggested-action");
        self.start_button.set_halign(gtk::Align::Center);
        self.start_button.set_sensitive(false);

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(12)
            .margin_end(12)
            .build();
        content.append(&self.summary_icon);
        content.append(&title);
        content.append(&self.summary_text);
        content.append(&self.options_group);
        content.append(&self.start_button);

        let weak = Rc::downgrade(self);
        self.binding.connect_selected_notify(move |_| {
            if let Some(wizard) = weak.upgrade() {
                wizard.refresh_summary();
            }
        });
        let weak = Rc::downgrade(self);
        self.start_button.connect_clicked(move |_| {
            if let Some(wizard) = weak.upgrade() {
                wizard.print_fronts();
            }
        });
        let root = page(i18n("Summary"), &scrolled(&content));
        self.nav.add(&root);
        *self.root_page.borrow_mut() = Some(root);
    }

    /// Shows `page` instead of the current step (the summary stays at the root).
    fn show(&self, page: &adw::NavigationPage) {
        match self.root_page.borrow().as_ref() {
            Some(root) => self.nav.replace(&[root.clone(), page.clone()]),
            None => self.nav.push(page),
        }
    }

    fn load(self: &Rc<Self>) {
        let path = self.job.pdf.clone();
        let wizard = self.clone();
        glib::spawn_future_local(async move {
            match background(move || SourceDocument::load(&path).map(|doc| doc.page_count())).await {
                Ok(count) => {
                    wizard.page_count.set(count);
                    wizard.refresh_summary();
                    wizard.start_button.set_sensitive(true);
                }
                Err(error) => {
                    wizard.summary_icon.set_icon_name(Some("dialog-error-symbolic"));
                    wizard.summary_text.set_label(&i18n_f("Cannot read this PDF: {error}", &[("error", &error)]));
                    wizard.options_group.set_visible(false);
                    wizard.start_button.set_visible(false);
                    wizard.stage.set(Stage::Done);
                }
            }
        });
    }

    fn binding_value(&self) -> Binding {
        BINDINGS[(self.binding.selected() as usize).min(1)]
    }

    fn plan(&self) -> PassPlan {
        let profile = self.ctx.config.borrow().duplex;
        duplex::plan(self.page_count.get(), self.binding_value(), &profile)
    }

    fn refresh_summary(&self) {
        let pages = self.page_count.get();
        if pages == 0 {
            return;
        }
        let plan = self.plan();
        let pages_text = ni18n_f("{count} page", "{count} pages", pages as u64, &[("count", &pages)]);
        let sheets_text = ni18n_f("{count} sheet", "{count} sheets", plan.sheets as u64, &[("count", &plan.sheets)]);
        let mut text = i18n_f(
            "{pages} on {sheets}.\nPut at least {sheets} in the rear tray.",
            &[("pages", &pages_text), ("sheets", &sheets_text)],
        );
        if self.job.calibration {
            text.push_str("\n\n");
            text.push_str(i18n("Use plain A4 paper. This test checks the order and orientation of the backs."));
        }
        if !plan.needs_second_pass() {
            text.push_str("\n\n");
            text.push_str(i18n("A single page: printed directly, no flipping needed."));
            self.start_button.set_label(i18n("Print"));
        } else {
            self.start_button.set_label(i18n("Print the fronts"));
        }
        self.summary_text.set_label(&text);
    }

    fn print_options(&self) -> PrintOptions {
        if self.job.calibration {
            return self.job.options.clone();
        }
        PrintOptions {
            page_size: self.size.value(),
            color_model: self.color.value(),
            print_quality: self.quality.value(),
            media_type: self.media.value(),
        }
    }

    // ------------------------------------------------------------------
    // Printing the passes
    // ------------------------------------------------------------------

    fn print_fronts(self: &Rc<Self>) {
        self.start_button.set_sensitive(false);
        let plan = self.plan();
        let options = self.print_options();
        let pdf = self.job.pdf.clone();
        let title = self.job.title.clone();
        let wanted_queue = self.job.target_queue.clone().or_else(|| self.ctx.config.borrow().target_queue.clone());
        let stem = format!("pass-{}", glib::random_int());

        let wizard = self.clone();
        glib::spawn_future_local(async move {
            let job_plan = plan.clone();
            let job_options = options.clone();
            let result = background(move || -> ts3350_core::Result<(Passes, String)> {
                let queue = cups::find_target_queue(wanted_queue.as_deref())?;
                let source = SourceDocument::load(&pdf)?;
                let dir = runtime_dir();
                let fronts = dir.join(format!("{stem}-fronts.pdf"));
                let backs = dir.join(format!("{stem}-backs.pdf"));
                std::fs::write(&fronts, source.build_pass(&job_plan.fronts, false)?)?;
                std::fs::write(&backs, source.build_pass(&job_plan.backs, job_plan.rotate_backs)?)?;
                let label =
                    if job_plan.needs_second_pass() { i18n_f("{title} (fronts)", &[("title", &title)]) } else { title };
                let id = cups::submit(&queue, &fronts, &label, &job_options)?;
                Ok((Passes { plan: job_plan, fronts, backs, queue, options: job_options }, id))
            })
            .await;
            wizard.start_button.set_sensitive(true);
            match result {
                Ok((passes, id)) => {
                    let queue = passes.queue.clone();
                    let sheets = passes.plan.sheets;
                    let second = passes.plan.needs_second_pass();
                    *wizard.passes.borrow_mut() = Some(passes);
                    wizard.stage.set(Stage::Fronts);
                    let title = if second { i18n("Printing the fronts") } else { i18n("Printing") };
                    wizard.show_progress(title, &sheets_to_print(sheets));
                    wizard.watch(queue, id);
                }
                Err(error) => wizard.ctx.error(i18n("Printing failed"), error),
            }
        });
    }

    fn print_backs(self: &Rc<Self>, button: gtk::Button) {
        let Some((queue, backs, options, sheets)) =
            self.passes.borrow().as_ref().map(|p| (p.queue.clone(), p.backs.clone(), p.options.clone(), p.plan.sheets))
        else {
            return;
        };
        let title = i18n_f("{title} (backs)", &[("title", &self.job.title)]);
        let wizard = self.clone();
        glib::spawn_future_local(async move {
            let (q, o) = (queue.clone(), options.clone());
            match background(move || cups::submit(&q, &backs, &title, &o)).await {
                Ok(id) => {
                    wizard.stage.set(Stage::Backs);
                    wizard.show_progress(i18n("Printing the backs"), &sheets_to_print(sheets));
                    wizard.watch(queue, id);
                }
                Err(error) => {
                    button.set_sensitive(true);
                    wizard.ctx.error(i18n("Printing failed"), error);
                }
            }
        });
    }

    fn reprint_fronts(self: &Rc<Self>) {
        let Some((queue, fronts, options)) =
            self.passes.borrow().as_ref().map(|p| (p.queue.clone(), p.fronts.clone(), p.options.clone()))
        else {
            return;
        };
        let title = i18n_f("{title} (fronts)", &[("title", &self.job.title)]);
        let wizard = self.clone();
        glib::spawn_future_local(async move {
            let q = queue.clone();
            match background(move || cups::submit(&q, &fronts, &title, &options)).await {
                Ok(id) => {
                    wizard.stage.set(Stage::Fronts);
                    wizard.show_progress(i18n("Printing the fronts"), i18n("New attempt"));
                    wizard.watch(queue, id);
                }
                Err(error) => wizard.ctx.error(i18n("Printing failed"), error),
            }
        });
    }

    fn show_progress(self: &Rc<Self>, title: &str, status: &str) {
        let spinner = adw::Spinner::builder().width_request(48).height_request(48).halign(gtk::Align::Center).build();
        let heading = gtk::Label::builder().label(title).wrap(true).build();
        heading.add_css_class("title-2");
        let status = gtk::Label::builder().label(status).wrap(true).justify(gtk::Justification::Center).build();
        status.add_css_class("dim-label");
        let problem = gtk::Label::builder().wrap(true).justify(gtk::Justification::Center).visible(false).build();
        problem.add_css_class("warning");
        problem.add_css_class("heading");
        let cancel = pill(i18n("Cancel printing"), false);
        let wizard = Rc::downgrade(self);
        cancel.connect_clicked(move |_| {
            if let Some(wizard) = wizard.upgrade() {
                wizard.ask_abort();
            }
        });

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .valign(gtk::Align::Center)
            .margin_start(24)
            .margin_end(24)
            .margin_top(24)
            .margin_bottom(24)
            .build();
        for widget in [
            spinner.upcast_ref::<gtk::Widget>(),
            heading.upcast_ref(),
            status.upcast_ref(),
            problem.upcast_ref(),
            cancel.upcast_ref(),
        ] {
            content.append(widget);
        }
        *self.progress.borrow_mut() = Some(ProgressWidgets { status, problem });
        self.show(&page(i18n("Printing"), &content));
    }

    /// Follows the CUPS job until it ends, then moves on to the next step.
    fn watch(self: &Rc<Self>, queue: String, id: String) {
        *self.cups_job.borrow_mut() = Some((queue.clone(), id.clone()));
        let wizard = self.clone();
        glib::spawn_future_local(async move {
            loop {
                glib::timeout_future(Duration::from_millis(1000)).await;
                if wizard.closed.get() || wizard.cups_job.borrow().as_ref().map(|(_, j)| j) != Some(&id) {
                    return;
                }
                let (q, j) = (queue.clone(), id.clone());
                let state = background(move || {
                    let progress = cups::job_progress(&q, &j);
                    let status = cups::printer_status(&q).ok();
                    (progress, status)
                })
                .await;
                match state {
                    (Ok(JobProgress::Done), _) => break,
                    (Ok(progress), status) => {
                        if let Some(widgets) = wizard.progress.borrow().as_ref() {
                            widgets.status.set_label(match progress {
                                JobProgress::Queued => i18n("Waiting for the printer…"),
                                _ => i18n("Printing…"),
                            });
                            let problem = status.and_then(|s| s.problem());
                            widgets.problem.set_visible(problem.is_some());
                            widgets.problem.set_label(problem.as_deref().unwrap_or(""));
                        }
                    }
                    (Err(error), _) => {
                        if let Some(widgets) = wizard.progress.borrow().as_ref() {
                            widgets.problem.set_visible(true);
                            widgets.problem.set_label(&error.to_string());
                        }
                    }
                }
            }
            *wizard.cups_job.borrow_mut() = None;
            wizard.pass_finished();
        });
    }

    fn pass_finished(self: &Rc<Self>) {
        let second = self.passes.borrow().as_ref().is_some_and(|p| p.plan.needs_second_pass());
        match self.stage.get() {
            Stage::Fronts if second => self.show_flip(),
            Stage::Fronts | Stage::Backs => self.finish(),
            _ => {}
        }
    }

    // ------------------------------------------------------------------
    // Step 3: flipping the stack
    // ------------------------------------------------------------------

    fn show_flip(self: &Rc<Self>) {
        self.stage.set(Stage::Flip);
        let (sheets, blank_last) = self
            .passes
            .borrow()
            .as_ref()
            .map(|p| (p.plan.sheets, self.page_count.get() % 2 == 1))
            .unwrap_or((0, false));

        let title = gtk::Label::builder().label(i18n("Flip the stack of sheets")).wrap(true).build();
        title.add_css_class("title-2");
        let steps = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).build();
        let texts = [
            ni18n_f(
                "Wait until the sheet is out, then take it from the output tray.",
                "Wait until the {count} sheets are out, then take the whole stack from the output tray without changing the order of the sheets.",
                sheets as u64,
                &[("count", &sheets)],
            ),
            i18n("Flip the stack over: the printed side must face the back of the printer.").into(),
            i18n("Slide it into the rear tray with the top of the pages going in first, as in the first pass.").into(),
            i18n("Click “Print the backs”.").into(),
        ];
        for (index, text) in texts.iter().enumerate() {
            let number = gtk::Label::builder().label(format!("{}", index + 1)).valign(gtk::Align::Start).build();
            number.add_css_class("heading");
            number.add_css_class("accent");
            let label = gtk::Label::builder().label(text).wrap(true).xalign(0.0).hexpand(true).build();
            let row = gtk::Box::builder().spacing(12).build();
            row.append(&number);
            row.append(&label);
            steps.append(&row);
        }
        if blank_last {
            let note = gtk::Label::builder()
                .label(i18n("The document has an odd number of pages: the last sheet will come out with a blank back."))
                .wrap(true)
                .xalign(0.0)
                .build();
            note.add_css_class("dim-label");
            steps.append(&note);
        }

        let print = pill(i18n("Print the backs"), true);
        let reprint = pill(i18n("Reprint the fronts"), false);
        reprint.add_css_class("flat");
        let weak = Rc::downgrade(self);
        print.connect_clicked(move |button| {
            if let Some(wizard) = weak.upgrade() {
                button.set_sensitive(false);
                wizard.print_backs(button.clone());
            }
        });
        let weak = Rc::downgrade(self);
        reprint.connect_clicked(move |_| {
            if let Some(wizard) = weak.upgrade() {
                wizard.reprint_fronts();
            }
        });

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .margin_top(12)
            .margin_bottom(24)
            .margin_start(12)
            .margin_end(12)
            .build();
        content.append(&illustration::widget());
        content.append(&title);
        content.append(&steps);
        content.append(&print);
        content.append(&reprint);

        self.show(&page(i18n("Flipping"), &scrolled(&content)));
        self.ctx.notify_if_inactive(
            "ts3350-flip",
            i18n("Flip the stack of sheets"),
            i18n("The fronts are printed: put the stack back in the rear tray to print the backs."),
        );
    }

    // ------------------------------------------------------------------
    // End
    // ------------------------------------------------------------------

    fn finish(self: &Rc<Self>) {
        self.stage.set(Stage::Done);
        self.cleanup_files();
        if let Some(job) = &self.job.spool {
            let _ = job.remove();
        }
        (self.on_finished)();

        if self.job.calibration {
            self.show_calibration_questions();
            return;
        }
        let status = adw::StatusPage::builder()
            .icon_name("object-select-symbolic")
            .title(i18n("Printing complete"))
            .description(i18n_f("“{title}” was printed on both sides.", &[("title", &self.job.title)]))
            .build();
        let close = pill(i18n("Close"), true);
        let dialog = self.dialog.clone();
        close.connect_clicked(move |_| {
            dialog.force_close();
        });
        status.set_child(Some(&close));
        self.show(&page(i18n("Done"), &status));
        self.ctx.notify_if_inactive("ts3350-done", i18n("Two-sided printing complete"), &self.job.title);
    }

    fn show_calibration_questions(self: &Rc<Self>) {
        let order = adw::SwitchRow::builder().title(i18n("The back of sheet 1 reads “SHEET 1”")).active(true).build();
        let rotation = adw::SwitchRow::builder()
            .title(i18n("On each sheet, the “TOP” arrows of the front and back point to the same edge"))
            .active(true)
            .build();
        let group = adw::PreferencesGroup::builder()
            .title(i18n("Check the 2 printed sheets"))
            .description(i18n("Untick what is not right: the setting will be corrected automatically."))
            .build();
        group.add(&order);
        group.add(&rotation);

        let save = pill(i18n("Save the setting"), true);
        let wizard = self.clone();
        save.connect_clicked(move |_| {
            let (order_ok, rotation_ok) = (order.is_active(), rotation.is_active());
            wizard.ctx.update_config(|c| {
                if !order_ok {
                    c.duplex.reverse_backs = !c.duplex.reverse_backs;
                }
                if !rotation_ok {
                    c.duplex.rotate_backs = !c.duplex.rotate_backs;
                }
            });
            wizard.ctx.toast(if order_ok && rotation_ok {
                i18n("The current setting is correct")
            } else {
                i18n("Duplex setting corrected: run the test again to check")
            });
            (wizard.on_finished)();
            wizard.dialog.force_close();
        });

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(24)
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(12)
            .margin_end(12)
            .build();
        content.append(&group);
        content.append(&save);
        self.show(&page(i18n("Test result"), &scrolled(&content)));
    }

    fn cleanup_files(&self) {
        if let Some(passes) = self.passes.borrow().as_ref() {
            let _ = std::fs::remove_file(&passes.fronts);
            let _ = std::fs::remove_file(&passes.backs);
        }
        if self.job.calibration {
            let _ = std::fs::remove_file(&self.job.pdf);
        }
    }

    // ------------------------------------------------------------------
    // Closing
    // ------------------------------------------------------------------

    fn connect_close(self: &Rc<Self>) {
        self.dialog.set_can_close(false);
        let weak = Rc::downgrade(self);
        self.dialog.connect_close_attempt(move |dialog| {
            let Some(wizard) = weak.upgrade() else {
                dialog.force_close();
                return;
            };
            match wizard.stage.get() {
                Stage::Summary | Stage::Done => wizard.close_now(),
                _ => wizard.ask_abort(),
            }
        });
    }

    fn ask_abort(self: &Rc<Self>) {
        let printing = self.cups_job.borrow().is_some();
        let body = if self.job.spool.is_some() {
            i18n("The job stays in the “Pending jobs” list: you can start it again from the beginning.")
        } else {
            i18n("Printing will have to start again from the beginning.")
        };
        let dialog = adw::AlertDialog::new(Some(i18n("Abandon printing?")), Some(body));
        dialog.add_responses(&[("continue", i18n("Continue")), ("abort", i18n("Abandon"))]);
        dialog.set_response_appearance("abort", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("continue"));
        let wizard = self.clone();
        glib::spawn_future_local(async move {
            if dialog.choose_future(Some(&wizard.dialog)).await != "abort" {
                return;
            }
            let job = wizard.cups_job.borrow_mut().take();
            if printing && let Some((_, id)) = job {
                let _ = background(move || cups::cancel(&id)).await;
            }
            wizard.close_now();
        });
    }

    fn close_now(&self) {
        self.closed.set(true);
        self.cleanup_files();
        self.dialog.force_close();
    }
}
