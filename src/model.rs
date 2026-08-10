//! The queue's data model.
//!
//! Rows are GObjects so that mutating one from the coordinator redraws it
//! without any manual list bookkeeping.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::paths::{output_path, AddSource};

mod imp {
    use super::*;

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::FileRow)]
    pub struct FileRow {
        /// Display name, e.g. `AMB_2657.NEF`.
        #[property(get, set)]
        pub name: RefCell<String>,
        /// Absolute path of the source NEF.
        #[property(get, set)]
        pub source: RefCell<String>,
        /// Absolute path the DNG will be written to.
        #[property(get, set)]
        pub target: RefCell<String>,
        /// The folder this file's relative path is computed against. Empty when
        /// the file was chosen individually rather than via a folder.
        #[property(get, set)]
        pub base: RefCell<String>,
        /// Human-readable state shown on the right of the row.
        #[property(get, set)]
        pub status: RefCell<String>,
        /// Whether this file is being converted right now.
        #[property(get, set)]
        pub busy: Cell<bool>,
        /// Whether this file failed, used to tint the status label and to
        /// decide membership when filtering the queue down to failures.
        #[property(get, set)]
        pub failed: Cell<bool>,
        /// Symbolic icon for the finished state; empty while queued.
        #[property(get, set)]
        pub icon: RefCell<String>,
        /// Full, untruncated explanation shown on hover; empty when there is
        /// nothing to add beyond the status text.
        #[property(get, set)]
        pub tooltip: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FileRow {
        const NAME: &'static str = "RawToDngFileRow";
        type Type = super::FileRow;
    }

    #[glib::derived_properties]
    impl ObjectImpl for FileRow {}
}

glib::wrapper! {
    pub struct FileRow(ObjectSubclass<imp::FileRow>);
}

impl FileRow {
    pub fn new(source: &Path, add: &AddSource, out_root: &Path) -> Self {
        let obj: Self = glib::Object::new();
        obj.set_name(
            source
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "(unnamed)".to_string()),
        );
        obj.set_source(source.to_string_lossy().into_owned());
        obj.set_base(match add {
            AddSource::Folder { base } => base.to_string_lossy().into_owned(),
            AddSource::Files => String::new(),
        });
        obj.set_status("Queued");
        obj.recompute_target(out_root);
        obj
    }

    pub fn source_path(&self) -> PathBuf {
        PathBuf::from(self.source())
    }

    pub fn target_path(&self) -> PathBuf {
        PathBuf::from(self.target())
    }

    /// How this row's path was derived, so the target can be recomputed when the
    /// output root changes.
    pub fn add_source(&self) -> AddSource {
        let base = self.base();
        if base.is_empty() {
            AddSource::Files
        } else {
            AddSource::Folder {
                base: PathBuf::from(base),
            }
        }
    }

    pub fn recompute_target(&self, out_root: &Path) {
        let target = output_path(&self.source_path(), &self.add_source(), out_root);
        self.set_target(target.to_string_lossy().into_owned());
    }
}
