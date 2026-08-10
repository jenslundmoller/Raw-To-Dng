//! NefToDng — batch convert Nikon NEF raw files to DNG.

use adw::prelude::*;
use gtk::glib;

use neftodng::ui;

const APP_ID: &str = "dk.lundmoller.NefToDng";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .build();

    app.connect_activate(ui::build);
    app.run()
}
