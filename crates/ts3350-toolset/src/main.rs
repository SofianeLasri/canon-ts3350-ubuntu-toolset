//! Canon TS3350 Toolset: scanning (eSCL) and manual duplex printing on GNOME.

mod app;
mod context;
mod duplex;
mod preferences;
mod scan;
mod util;
mod window;

fn main() -> gtk::glib::ExitCode {
    app::run()
}
