//! “Duplex” tab: virtual printer status, pending jobs, calibration.

mod illustration;
mod wizard;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use ts3350_core::cups;
use ts3350_core::i18n::{i18n, i18n_f};
use ts3350_core::spool::{self, SpoolJob};

pub use wizard::WizardJob;

use crate::context::AppContext;
use crate::util::{background, format_time};

pub struct DuplexPage {
    pub root: adw::PreferencesPage,
    ctx: Rc<AppContext>,
    virtual_row: adw::ActionRow,
    target_row: adw::ActionRow,
    jobs_group: adw::PreferencesGroup,
    job_rows: RefCell<Vec<gtk::Widget>>,
    profile_row: adw::ActionRow,
}

impl DuplexPage {
    pub fn new(ctx: Rc<AppContext>) -> Rc<DuplexPage> {
        let virtual_row =
            adw::ActionRow::builder().title(i18n("“Canon TS3350” printer")).subtitle(i18n("Checking…")).build();
        virtual_row.add_prefix(&gtk::Image::from_icon_name("printer-symbolic"));
        let target_row = adw::ActionRow::builder().title(i18n("Physical printer")).subtitle(i18n("Searching…")).build();
        target_row.add_css_class("property");
        let printer_group = adw::PreferencesGroup::builder()
            .title(i18n("Printer"))
            .description(i18n(
                "From any application, print to “Canon TS3350” and choose “Two-sided: long edge” or “short edge”: this wizard opens by itself.",
            ))
            .build();
        printer_group.add(&virtual_row);
        printer_group.add(&target_row);

        let open_row =
            adw::ButtonRow::builder().title(i18n("Choose a PDF…")).start_icon_name("document-open-symbolic").build();
        let open_group = adw::PreferencesGroup::builder().title(i18n("Print a PDF on both sides")).build();
        open_group.add(&open_row);

        let jobs_group = adw::PreferencesGroup::builder()
            .title(i18n("Pending jobs"))
            .description(i18n("Two-sided jobs sent to “Canon TS3350” and not finished yet"))
            .build();

        let profile_row = adw::ActionRow::builder().title(i18n("Current setting")).build();
        profile_row.add_css_class("property");
        let calibrate_row = adw::ButtonRow::builder()
            .title(i18n("Print a test page…"))
            .start_icon_name("document-print-symbolic")
            .action_name("app.calibrate")
            .build();
        let profile_group = adw::PreferencesGroup::builder()
            .title(i18n("Sheet flipping"))
            .description(i18n(
                "If the backs come out in the wrong order or upside down, print the test page (2 sheets): the setting is corrected from your answers.",
            ))
            .build();
        profile_group.add(&profile_row);
        profile_group.add(&calibrate_row);

        let root = adw::PreferencesPage::new();
        root.add(&printer_group);
        root.add(&open_group);
        root.add(&jobs_group);
        root.add(&profile_group);

        let page = Rc::new(DuplexPage {
            root,
            ctx,
            virtual_row,
            target_row,
            jobs_group,
            job_rows: RefCell::new(Vec::new()),
            profile_row,
        });

        let weak = Rc::downgrade(&page);
        open_row.connect_activated(move |_| {
            if let Some(page) = weak.upgrade() {
                page.choose_pdf();
            }
        });
        page.refresh();
        page
    }

    /// Reloads the CUPS and spool state.
    pub fn refresh(self: &Rc<Self>) {
        self.update_profile();
        let user = glib::user_name().to_string_lossy().into_owned();
        let preferred = self.ctx.config.borrow().target_queue.clone();
        let page = self.clone();
        glib::spawn_future_local(async move {
            let (installed, target, jobs) = background(move || {
                (
                    cups::virtual_queue_installed(),
                    cups::find_target_queue(preferred.as_deref()),
                    spool::list_jobs(&user),
                )
            })
            .await;
            page.virtual_row.set_subtitle(if installed {
                i18n("Installed: shown in the print dialog of your applications")
            } else {
                i18n("Not installed: run ./install.sh to add it to CUPS")
            });
            match target {
                Ok(queue) => page.target_row.set_subtitle(&queue),
                Err(error) => page.target_row.set_subtitle(&error.to_string()),
            }
            page.set_jobs(jobs);
        });
    }

    fn update_profile(&self) {
        let profile = self.ctx.config.borrow().duplex;
        self.profile_row.set_subtitle(&i18n_f(
            "Order of the backs: {order} · Rotation of the backs: {rotation}",
            &[
                ("order", &if profile.reverse_backs { i18n("reversed") } else { i18n("normal") }),
                ("rotation", &if profile.rotate_backs { "180°" } else { i18n("none") }),
            ],
        ));
    }

    fn set_jobs(self: &Rc<Self>, jobs: Vec<SpoolJob>) {
        for row in self.job_rows.borrow_mut().drain(..) {
            self.jobs_group.remove(&row);
        }
        let mut rows = Vec::new();
        if jobs.is_empty() {
            let empty = adw::ActionRow::builder().title(i18n("No pending job")).build();
            empty.add_css_class("dim-label");
            rows.push(empty.upcast::<gtk::Widget>());
        }
        for job in jobs {
            let row = adw::ActionRow::builder()
                .title(&job.meta.title)
                .title_lines(1)
                .subtitle(i18n_f(
                    "Received {date} · {binding}",
                    &[("date", &format_time(job.meta.received)), ("binding", &job.meta.binding.label())],
                ))
                .use_markup(false)
                .build();
            let resume = gtk::Button::builder().label(i18n("Print")).valign(gtk::Align::Center).build();
            resume.add_css_class("suggested-action");
            let delete = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text(i18n("Delete this job"))
                .valign(gtk::Align::Center)
                .build();
            delete.add_css_class("flat");
            row.add_suffix(&resume);
            row.add_suffix(&delete);

            let weak = Rc::downgrade(self);
            let resume_job = job.clone();
            resume.connect_clicked(move |_| {
                if let Some(page) = weak.upgrade() {
                    page.open_job(WizardJob::from_spool(resume_job.clone()));
                }
            });
            let weak = Rc::downgrade(self);
            delete.connect_clicked(move |_| {
                if let Some(page) = weak.upgrade() {
                    if let Err(error) = job.remove() {
                        page.ctx.error(i18n("Deletion failed"), error);
                    }
                    page.refresh();
                }
            });
            rows.push(row.upcast());
        }
        for row in &rows {
            self.jobs_group.add(row);
        }
        *self.job_rows.borrow_mut() = rows;
    }

    pub fn open_job(self: &Rc<Self>, job: WizardJob) {
        let weak = Rc::downgrade(self);
        wizard::open(
            self.ctx.clone(),
            job,
            Rc::new(move || {
                if let Some(page) = weak.upgrade() {
                    page.refresh();
                }
            }),
        );
    }

    /// Opens a file received through “Open with” or from the CUPS backend.
    pub fn open_path(self: &Rc<Self>, path: PathBuf) {
        if spool::is_spool_path(&path) {
            match SpoolJob::from_pdf(&path) {
                Ok(job) => self.open_job(WizardJob::from_spool(job)),
                Err(error) => self.ctx.error(i18n("Job not found"), error),
            }
        } else {
            self.open_job(WizardJob::from_file(path));
        }
        self.refresh();
    }

    fn choose_pdf(self: &Rc<Self>) {
        let filter = gtk::FileFilter::new();
        filter.add_mime_type("application/pdf");
        filter.set_name(Some(i18n("PDF documents")));
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let chooser =
            gtk::FileDialog::builder().title(i18n("Print a PDF on both sides")).filters(&filters).modal(true).build();
        let page = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(file) = chooser.open_future(Some(&page.ctx.window)).await
                && let Some(path) = file.path()
            {
                page.open_job(WizardJob::from_file(path));
            }
        });
    }

    pub fn start_calibration(self: &Rc<Self>) {
        match WizardJob::calibration() {
            Ok(job) => self.open_job(job),
            Err(error) => self.ctx.error(i18n("Cannot create the test page"), error),
        }
    }
}
