//! NefToDng — batch convert Nikon NEF raw files to DNG.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use neftodng::ui::{self, AddPaths};

const APP_ID: &str = "dk.lundmoller.NefToDng";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        // Declared in the .desktop file, so files opened from the file manager
        // must actually be accepted rather than rejected as unsupported.
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    // Built lazily on first activation, then reused so a second launch adds to
    // the existing queue instead of opening another window.
    let adder: Rc<RefCell<Option<AddPaths>>> = Rc::new(RefCell::new(None));

    // Must run after GTK is initialised, which `startup` guarantees and the
    // pre-`run` position did not.
    app.connect_startup(|_| {
        gtk::Window::set_default_icon_name(APP_ID);
    });

    {
        let adder = adder.clone();
        app.connect_activate(move |app| {
            if adder.borrow().is_some() {
                if let Some(window) = app.active_window() {
                    window.present();
                }
                return;
            }
            let add = ui::build(app);
            *adder.borrow_mut() = Some(add);
        });
    }

    {
        let adder = adder.clone();
        app.connect_open(move |app, files, _hint| {
            if adder.borrow().is_none() {
                let add = ui::build(app);
                *adder.borrow_mut() = Some(add);
            }

            let paths: Vec<PathBuf> = files.iter().filter_map(|f| f.path()).collect();
            let add = adder.borrow().clone();
            if let Some(add) = add {
                add(paths);
            }

            if let Some(window) = app.active_window() {
                window.present();
            }
        });
    }

    app.run()
}
