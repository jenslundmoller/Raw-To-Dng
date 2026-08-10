//! Deciding where a converted DNG is written.
//!
//! This is the only logic in the app that can quietly corrupt an archive by
//! writing to the wrong place, so it is kept free of GUI and I/O concerns and
//! tested directly.

use std::path::{Path, PathBuf};

/// How a batch of files entered the queue.
///
/// A dropped *folder* mirrors its subtree beneath the output root; individually
/// chosen *files* land flat at the output root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddSource {
    /// Files picked one by one. No structure to preserve.
    Files,
    /// A folder was added; `base` is the directory that relative paths are
    /// computed against.
    Folder { base: PathBuf },
}

/// Work out where `source` should be written beneath `out_root`.
pub fn output_path(source: &Path, add: &AddSource, out_root: &Path) -> PathBuf {
    let relative = match add {
        AddSource::Folder { base } => source.strip_prefix(base).ok(),
        AddSource::Files => None,
    };

    match relative {
        Some(rel) => out_root.join(rel).with_extension("dng"),
        // A source with no file name cannot occur via the folder walk or the
        // file chooser, but this decides where bytes get written, so it degrades
        // instead of panicking.
        None => match source.file_name() {
            Some(name) => out_root.join(name).with_extension("dng"),
            None => out_root.join("converted.dng"),
        },
    }
}

/// The base a dropped folder's contents are made relative to.
///
/// Dropping `~/Pictures/2026` should yield `<out>/2026/...`, so the folder's own
/// name must survive — meaning the base is its *parent*, not the folder itself.
pub fn base_for_folder(dropped: &Path) -> PathBuf {
    dropped.parent().unwrap_or(dropped).to_path_buf()
}

/// Whether a path is a Nikon raw file we can convert, judged by extension.
pub fn is_nikon_raw(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => ext.eq_ignore_ascii_case("nef") || ext.eq_ignore_ascii_case("nrw"),
        None => false,
    }
}

/// The temporary file a conversion writes to before being renamed into place.
///
/// Writing `.NAME.dng.part` and renaming on success means a crash, a cancel or a
/// full disk can never leave a truncated `.dng` that later passes for a good one.
pub fn part_path(final_path: &Path) -> PathBuf {
    let name = final_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output.dng".to_string());

    let hidden = format!(".{name}.part");
    match final_path.parent() {
        Some(dir) => dir.join(hidden),
        None => PathBuf::from(hidden),
    }
}

/// Whether `path` lies inside `root`, used to keep the output root out of source
/// folder walks so it can safely live inside the pictures tree.
pub fn is_within(path: &Path, root: &Path) -> bool {
    // Path::starts_with compares whole components, so "DNG-old" is correctly
    // not treated as being inside "DNG".
    path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pathological_source_does_not_panic() {
        // Nothing in the app should be able to bring down a batch by deciding
        // where to write. A source with no file name must degrade, not panic.
        let got = output_path(Path::new("/"), &AddSource::Files, Path::new("/out"));

        assert!(got.starts_with("/out"), "must stay under the output root");
    }

    #[test]
    fn source_outside_its_base_falls_back_to_flat_output() {
        // Guard: a mismatched base must never escape the output root, e.g. by
        // producing "/out/../.." from a bad strip_prefix.
        let add = AddSource::Folder {
            base: PathBuf::from("/some/other/tree"),
        };

        let got = output_path(
            Path::new("/home/jens/Pictures/2026/AMB_2657.NEF"),
            &add,
            Path::new("/out"),
        );

        assert_eq!(got, PathBuf::from("/out/AMB_2657.dng"));
    }

    #[test]
    fn recognises_nikon_raw_regardless_of_extension_case() {
        assert!(is_nikon_raw(Path::new("/x/AMB_2657.NEF")));
        assert!(is_nikon_raw(Path::new("/x/AMB_2657.nef")));
        assert!(is_nikon_raw(Path::new("/x/DSCN0001.NRW")));
        assert!(!is_nikon_raw(Path::new("/x/AMB_2657.dng")));
        assert!(!is_nikon_raw(Path::new("/x/AMB_2657.jpg")));
        assert!(!is_nikon_raw(Path::new("/x/NEF")));
    }

    #[test]
    fn part_file_is_hidden_and_sits_beside_its_target() {
        let got = part_path(Path::new("/out/2026/07/AMB_2657.dng"));

        assert_eq!(got, PathBuf::from("/out/2026/07/.AMB_2657.dng.part"));
        assert_eq!(got.parent(), Path::new("/out/2026/07/AMB_2657.dng").parent());
    }

    #[test]
    fn output_root_inside_source_tree_is_excluded_from_walks() {
        let out = Path::new("/home/jens/Pictures/DNG");

        assert!(is_within(Path::new("/home/jens/Pictures/DNG/2026/a.dng"), out));
        assert!(is_within(out, out));
        assert!(!is_within(Path::new("/home/jens/Pictures/2026/a.NEF"), out));
        // a sibling that merely shares a name prefix is not inside
        assert!(!is_within(Path::new("/home/jens/Pictures/DNG-old/a.dng"), out));
    }

    #[test]
    fn dropped_folder_keeps_its_own_name_in_the_output() {
        let base = base_for_folder(Path::new("/home/jens/Pictures/2026"));

        assert_eq!(base, PathBuf::from("/home/jens/Pictures"));

        // and end-to-end: the "2026" segment must survive into the output
        let got = output_path(
            Path::new("/home/jens/Pictures/2026/07/21/AMB_2657.NEF"),
            &AddSource::Folder { base },
            Path::new("/home/jens/Pictures/DNG"),
        );
        assert_eq!(
            got,
            PathBuf::from("/home/jens/Pictures/DNG/2026/07/21/AMB_2657.dng")
        );
    }

    #[test]
    fn folder_add_mirrors_subtree_beneath_output_root() {
        let add = AddSource::Folder {
            base: PathBuf::from("/home/jens/Pictures"),
        };

        let got = output_path(
            Path::new("/home/jens/Pictures/2026/07/21/AMB_2657.NEF"),
            &add,
            Path::new("/home/jens/Pictures/DNG"),
        );

        assert_eq!(
            got,
            PathBuf::from("/home/jens/Pictures/DNG/2026/07/21/AMB_2657.dng")
        );
    }
}
