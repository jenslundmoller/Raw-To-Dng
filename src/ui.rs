//! The window: queue, output root, and running a batch.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use rayon::prelude::*;

use crate::convert::{convert_file, Outcome};
use crate::model::FileRow;
use crate::paths::{base_for_folder, is_nikon_raw, AddSource};
use crate::presentation::present;
use crate::scan::collect_raw_files;
use crate::summary::{completion_message, BatchSummary};

/// Sent from the worker pool back to the main loop.
enum Msg {
    Started(u32),
    /// The whole result travels back so the main loop owns both the wording
    /// shown on the row and the running totals, keeping them consistent.
    Finished {
        index: u32,
        result: Result<Outcome, String>,
    },
    AllDone,
}

/// Default output root: `~/Pictures/DNG`, falling back to the home directory.
fn default_out_root() -> PathBuf {
    glib::user_special_dir(glib::UserDirectory::Pictures)
        .unwrap_or_else(glib::home_dir)
        .join("DNG")
}

/// Adds paths to the queue of an already-built window. Returned by [`build`] so
/// files opened from the file manager land in the existing window instead of
/// spawning a second one.
pub type AddPaths = Rc<dyn Fn(Vec<PathBuf>)>;

pub fn build(app: &adw::Application) -> AddPaths {
    let store = gio::ListStore::new::<FileRow>();
    let out_root = Rc::new(RefCell::new(default_out_root()));
    let cancel = Arc::new(AtomicBool::new(false));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("NEF → DNG")
        .default_width(760)
        .default_height(580)
        .build();

    // ---- queue view -------------------------------------------------------
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("ListItem");

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.set_margin_top(8);
        row.set_margin_bottom(8);
        row.set_margin_start(12);
        row.set_margin_end(12);

        let spinner = gtk::Spinner::new();
        let icon = gtk::Image::new();
        let name = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .build();
        let status = gtk::Label::builder()
            .xalign(1.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(44)
            .build();
        row.append(&spinner);
        row.append(&icon);
        row.append(&name);
        row.append(&status);
        item.set_child(Some(&row));

        // Expressions keep the widgets in sync with whichever FileRow is bound
        // here, with no bind/unbind bookkeeping.
        item.property_expression("item")
            .chain_property::<FileRow>("name")
            .bind(&name, "label", gtk::Widget::NONE);
        item.property_expression("item")
            .chain_property::<FileRow>("status")
            .bind(&status, "label", gtk::Widget::NONE);
        item.property_expression("item")
            .chain_property::<FileRow>("busy")
            .bind(&spinner, "spinning", gtk::Widget::NONE);
        item.property_expression("item")
            .chain_property::<FileRow>("busy")
            .bind(&spinner, "visible", gtk::Widget::NONE);

        item.property_expression("item")
            .chain_property::<FileRow>("icon")
            .bind(&icon, "icon-name", gtk::Widget::NONE);

        // A queued row has no icon yet, and an empty icon-name would render as
        // a broken image rather than as nothing.
        item.property_expression("item")
            .chain_property::<FileRow>("icon")
            .chain_closure::<bool>(glib::closure!(|_: Option<glib::Object>, icon: String| {
                !icon.is_empty()
            }))
            .bind(&icon, "visible", gtk::Widget::NONE);

        // Failures are tinted with libadwaita's error style; everything else
        // stays dimmed so the failures are what the eye lands on.
        item.property_expression("item")
            .chain_property::<FileRow>("failed")
            .chain_closure::<glib::StrV>(glib::closure!(
                |_: Option<glib::Object>, failed: bool| {
                    if failed {
                        glib::StrV::from(vec!["error"])
                    } else {
                        glib::StrV::from(vec!["dim-label"])
                    }
                }
            ))
            .bind(&status, "css-classes", gtk::Widget::NONE);

        item.property_expression("item")
            .chain_property::<FileRow>("tooltip")
            .bind(&row, "tooltip-text", gtk::Widget::NONE);
    });

    // The filter is only attached while the failures toggle is on, so the
    // normal case walks the store directly.
    let failure_filter = gtk::CustomFilter::new(|obj| {
        obj.downcast_ref::<FileRow>().is_some_and(FileRow::failed)
    });
    let filter_model = gtk::FilterListModel::new(Some(store.clone()), None::<gtk::CustomFilter>);

    let selection = gtk::NoSelection::new(Some(filter_model.clone()));
    let list = gtk::ListView::new(Some(selection), Some(factory));
    let scroller = gtk::ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .build();

    let empty = adw::StatusPage::builder()
        .icon_name("camera-photo-symbolic")
        .title("Drop NEF files here")
        .description("Or use Add Files and Add Folder. Your originals are never modified.")
        .vexpand(true)
        .build();

    let stack = gtk::Stack::new();
    stack.add_named(&empty, Some("empty"));
    stack.add_named(&scroller, Some("list"));

    // ---- header -----------------------------------------------------------
    let header = adw::HeaderBar::new();
    let add_files_btn = gtk::Button::with_label("Add Files");
    let add_folder_btn = gtk::Button::with_label("Add Folder");
    let clear_btn = gtk::Button::from_icon_name("edit-clear-all-symbolic");
    clear_btn.set_tooltip_text(Some("Clear the queue"));

    // Appears only once something has failed; toggling it hides everything else.
    let failures_btn = gtk::ToggleButton::new();
    failures_btn.set_visible(false);
    failures_btn.add_css_class("error");
    failures_btn.set_tooltip_text(Some("Show only the files that failed"));

    header.pack_start(&add_files_btn);
    header.pack_start(&add_folder_btn);
    header.pack_end(&clear_btn);
    header.pack_end(&failures_btn);

    // ---- bottom bar -------------------------------------------------------
    let out_btn = gtk::Button::new();
    out_btn.set_tooltip_text(Some("Choose where DNGs are written"));
    let overwrite = gtk::CheckButton::with_label("Overwrite existing");
    let convert_btn = gtk::Button::with_label("Convert");
    convert_btn.add_css_class("suggested-action");
    let cancel_btn = gtk::Button::with_label("Cancel");
    cancel_btn.add_css_class("destructive-action");
    cancel_btn.set_visible(false);

    let progress = gtk::ProgressBar::builder().hexpand(true).build();
    progress.set_visible(false);

    let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    bottom.set_margin_top(10);
    bottom.set_margin_bottom(10);
    bottom.set_margin_start(12);
    bottom.set_margin_end(12);
    bottom.append(&out_btn);
    bottom.append(&overwrite);
    bottom.append(&progress);
    bottom.append(&cancel_btn);
    bottom.append(&convert_btn);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));
    toolbar.add_bottom_bar(&bottom);

    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&toolbar));
    window.set_content(Some(&toasts));

    // ---- shared UI updates ------------------------------------------------
    let refresh_out_label = {
        let out_root = out_root.clone();
        let out_btn = out_btn.clone();
        move || {
            let path = out_root.borrow();
            let shown = shorten_home(&path);
            out_btn.set_label(&format!("Output: {shown}"));
        }
    };
    refresh_out_label();

    let refresh_stack = {
        let store = store.clone();
        let stack = stack.clone();
        let convert_btn = convert_btn.clone();
        move || {
            let n = store.n_items();
            stack.set_visible_child_name(if n == 0 { "empty" } else { "list" });
            convert_btn.set_sensitive(n > 0);
            convert_btn.set_label(&if n == 0 {
                "Convert".to_string()
            } else {
                format!("Convert {n}")
            });
        }
    };
    refresh_stack();

    {
        let filter_model = filter_model.clone();
        let failure_filter = failure_filter.clone();
        failures_btn.connect_toggled(move |btn| {
            if btn.is_active() {
                filter_model.set_filter(Some(&failure_filter));
            } else {
                filter_model.set_filter(None::<&gtk::CustomFilter>);
            }
        });
    }

    // ---- adding files -----------------------------------------------------
    let add_paths: AddPaths = {
        let store = store.clone();
        let out_root = out_root.clone();
        let refresh_stack = refresh_stack.clone();
        Rc::new(move |dropped: Vec<PathBuf>| {
            let out = out_root.borrow().clone();

            let mut seen: HashSet<PathBuf> = HashSet::new();
            for i in 0..store.n_items() {
                if let Some(row) = store.item(i).and_downcast::<FileRow>() {
                    seen.insert(row.source_path());
                }
            }

            for path in dropped {
                if path.is_dir() {
                    let add = AddSource::Folder {
                        base: base_for_folder(&path),
                    };
                    for file in collect_raw_files(&path, &out) {
                        if seen.insert(file.clone()) {
                            store.append(&FileRow::new(&file, &add, &out));
                        }
                    }
                } else if is_nikon_raw(&path) && seen.insert(path.clone()) {
                    store.append(&FileRow::new(&path, &AddSource::Files, &out));
                }
            }

            refresh_stack();
        })
    };

    // drag and drop
    let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    {
        let add_paths = add_paths.clone();
        drop.connect_drop(move |_, value, _, _| match value.get::<gdk::FileList>() {
            Ok(list) => {
                add_paths(list.files().iter().filter_map(|f| f.path()).collect());
                true
            }
            Err(_) => false,
        });
    }
    window.add_controller(drop);

    // Add Files
    {
        let add_paths = add_paths.clone();
        let window = window.clone();
        add_files_btn.connect_clicked(move |_| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Nikon raw (NEF, NRW)"));
            filter.add_pattern("*.nef");
            filter.add_pattern("*.NEF");
            filter.add_pattern("*.nrw");
            filter.add_pattern("*.NRW");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);

            let dialog = gtk::FileDialog::builder()
                .title("Add NEF files")
                .filters(&filters)
                .build();

            let add_paths = add_paths.clone();
            dialog.open_multiple(Some(&window), gio::Cancellable::NONE, move |result| {
                if let Ok(files) = result {
                    let paths: Vec<PathBuf> = (0..files.n_items())
                        .filter_map(|i| files.item(i).and_downcast::<gio::File>())
                        .filter_map(|f| f.path())
                        .collect();
                    add_paths(paths);
                }
            });
        });
    }

    // Add Folder
    {
        let add_paths = add_paths.clone();
        let window = window.clone();
        add_folder_btn.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::builder().title("Add a folder").build();
            let add_paths = add_paths.clone();
            dialog.select_folder(Some(&window), gio::Cancellable::NONE, move |result| {
                if let Ok(folder) = result {
                    if let Some(path) = folder.path() {
                        add_paths(vec![path]);
                    }
                }
            });
        });
    }

    // Clear
    {
        let store = store.clone();
        let refresh_stack = refresh_stack.clone();
        clear_btn.connect_clicked(move |_| {
            store.remove_all();
            refresh_stack();
        });
    }

    // Output root
    {
        let window = window.clone();
        let out_root = out_root.clone();
        let store = store.clone();
        let refresh_out_label = refresh_out_label.clone();
        out_btn.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::builder().title("Output folder").build();
            dialog.set_initial_folder(Some(&gio::File::for_path(&*out_root.borrow())));

            let out_root = out_root.clone();
            let store = store.clone();
            let refresh_out_label = refresh_out_label.clone();
            dialog.select_folder(Some(&window), gio::Cancellable::NONE, move |result| {
                if let Ok(folder) = result {
                    if let Some(path) = folder.path() {
                        *out_root.borrow_mut() = path.clone();
                        // targets are derived from the root, so they all move
                        for i in 0..store.n_items() {
                            if let Some(row) = store.item(i).and_downcast::<FileRow>() {
                                row.recompute_target(&path);
                            }
                        }
                        refresh_out_label();
                    }
                }
            });
        });
    }

    // ---- running a batch --------------------------------------------------
    {
        let store = store.clone();
        let cancel = cancel.clone();
        let convert_btn_c = convert_btn.clone();
        let cancel_btn_c = cancel_btn.clone();
        let progress_c = progress.clone();
        let overwrite_c = overwrite.clone();
        let window_c = window.clone();
        let failures_btn_c = failures_btn.clone();
        let failure_filter_c = failure_filter.clone();
        let add_files_btn_c = add_files_btn.clone();
        let add_folder_btn_c = add_folder_btn.clone();
        let clear_btn_c = clear_btn.clone();

        convert_btn.connect_clicked(move |_| {
            let total = store.n_items();
            if total == 0 {
                return;
            }

            let jobs: Vec<(u32, PathBuf, PathBuf)> = (0..total)
                .filter_map(|i| {
                    store
                        .item(i)
                        .and_downcast::<FileRow>()
                        .map(|r| (i, r.source_path(), r.target_path()))
                })
                .collect();

            // A re-run starts from a clean slate, so stale icons and errors from
            // a previous attempt cannot be mistaken for this one's results.
            for i in 0..total {
                if let Some(row) = store.item(i).and_downcast::<FileRow>() {
                    row.set_status("Queued");
                    row.set_failed(false);
                    row.set_busy(false);
                    row.set_icon("");
                    row.set_tooltip("");
                }
            }
            failures_btn_c.set_active(false);
            failures_btn_c.set_visible(false);

            cancel.store(false, Ordering::SeqCst);
            convert_btn_c.set_visible(false);
            cancel_btn_c.set_visible(true);
            progress_c.set_visible(true);
            progress_c.set_fraction(0.0);
            add_files_btn_c.set_sensitive(false);
            add_folder_btn_c.set_sensitive(false);
            clear_btn_c.set_sensitive(false);

            let (tx, rx) = async_channel::unbounded::<Msg>();
            let overwrite_flag = overwrite_c.is_active();
            let cancel_flag = cancel.clone();

            std::thread::spawn(move || {
                let threads = num_cpus::get().max(1);
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .expect("build thread pool");

                pool.install(|| {
                    jobs.par_iter().for_each(|(index, source, target)| {
                        if cancel_flag.load(Ordering::SeqCst) {
                            return;
                        }
                        let _ = tx.send_blocking(Msg::Started(*index));

                        let result = convert_file(source, target, overwrite_flag)
                            .map_err(|e| e.to_string());

                        let _ = tx.send_blocking(Msg::Finished {
                            index: *index,
                            result,
                        });
                    });
                });

                let _ = tx.send_blocking(Msg::AllDone);
            });

            // coordinator: the only place worker state touches UI state
            let store = store.clone();
            let convert_btn = convert_btn_c.clone();
            let cancel_btn = cancel_btn_c.clone();
            let progress = progress_c.clone();
            let window_for_dialog = window_c.clone();
            let failures_btn = failures_btn_c.clone();
            let failure_filter = failure_filter_c.clone();
            let add_files_btn = add_files_btn_c.clone();
            let add_folder_btn = add_folder_btn_c.clone();
            let clear_btn = clear_btn_c.clone();

            glib::spawn_future_local(async move {
                let mut summary = BatchSummary::default();

                while let Ok(msg) = rx.recv().await {
                    match msg {
                        Msg::Started(i) => {
                            if let Some(row) = store.item(i).and_downcast::<FileRow>() {
                                row.set_busy(true);
                                row.set_status("Converting…");
                            }
                        }
                        Msg::Finished { index, result } => {
                            let shown = present(&result);

                            match &result {
                                Ok(outcome) => summary.record(outcome),
                                Err(_) => summary.record_failure(),
                            }

                            if let Some(row) = store.item(index).and_downcast::<FileRow>() {
                                row.set_busy(false);
                                row.set_failed(shown.failed);
                                row.set_status(shown.status);
                                row.set_icon(shown.icon);
                                row.set_tooltip(shown.tooltip.unwrap_or_default());
                            }

                            if shown.failed {
                                failures_btn.set_visible(true);
                                failures_btn.set_label(&format!("{} failed", summary.failed));
                                // The row only became a match now, so a filter
                                // that is already applied must re-evaluate.
                                if failures_btn.is_active() {
                                    failure_filter.changed(gtk::FilterChange::LessStrict);
                                }
                            }

                            progress.set_fraction(
                                f64::from(summary.total()) / f64::from(total.max(1)),
                            );
                        }
                        Msg::AllDone => break,
                    }
                }

                convert_btn.set_visible(true);
                cancel_btn.set_visible(false);
                progress.set_visible(false);
                add_files_btn.set_sensitive(true);
                add_folder_btn.set_sensitive(true);
                clear_btn.set_sensitive(true);

                let (heading, body) = completion_message(&summary, total);
                let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));
                dialog.add_response("close", "Close");
                dialog.set_default_response(Some("close"));
                dialog.set_close_response("close");

                // Offer the shortcut only when there is something to look at.
                if summary.failed > 0 {
                    dialog.add_response("failures", "Show Failures");
                    dialog.set_response_appearance(
                        "failures",
                        adw::ResponseAppearance::Suggested,
                    );
                    let failures_btn = failures_btn.clone();
                    dialog.connect_response(None, move |_, response| {
                        if response == "failures" {
                            failures_btn.set_active(true);
                        }
                    });
                }

                dialog.present(Some(&window_for_dialog));
            });
        });
    }

    // Cancel
    {
        let cancel = cancel.clone();
        cancel_btn.connect_clicked(move |btn| {
            cancel.store(true, Ordering::SeqCst);
            btn.set_sensitive(false);
            btn.set_label("Cancelling…");
        });
    }

    window.present();
    add_paths
}

/// `/home/jens/Pictures/DNG` → `~/Pictures/DNG`, for a readable button label.
fn shorten_home(path: &Path) -> String {
    let home = glib::home_dir();
    match path.strip_prefix(&home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}
