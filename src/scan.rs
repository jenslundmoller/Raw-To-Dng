//! Turning a dropped folder into a list of files to convert.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::paths::{is_supported_raw, is_within};

/// How deep a folder walk will descend. Deliberately finite so a symlink loop or
/// an accidentally dropped `/` cannot hang the UI indefinitely.
const MAX_DEPTH: usize = 12;

/// Every supported raw file beneath `root`, excluding anything inside `out_root`.
///
/// Results are sorted so a batch runs in a predictable order.
pub fn collect_raw_files(root: &Path, out_root: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = WalkDir::new(root)
        .max_depth(MAX_DEPTH)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_within(e.path(), out_root))
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| is_supported_raw(p))
        .collect();

    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn scratch() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("neftodng-scan-{}-{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    #[test]
    fn finds_raw_files_recursively_and_ignores_everything_else() {
        let root = scratch();
        fs::create_dir_all(root.join("2026/07/21")).unwrap();
        fs::write(root.join("2026/07/21/AMB_2657.NEF"), b"x").unwrap();
        fs::write(root.join("2026/07/21/AMB_2658.nef"), b"x").unwrap();
        fs::write(root.join("2026/07/21/AMB_2657.JPG"), b"x").unwrap();
        fs::write(root.join("2026/notes.txt"), b"x").unwrap();

        let found = collect_raw_files(&root, Path::new("/nonexistent"));

        assert_eq!(
            found,
            vec![
                root.join("2026/07/21/AMB_2657.NEF"),
                root.join("2026/07/21/AMB_2658.nef"),
            ],
            "only raw files, sorted"
        );
    }

    #[test]
    fn skips_the_output_root_so_it_can_live_inside_the_pictures_tree() {
        let root = scratch();
        let out = root.join("DNG");
        fs::create_dir_all(&out).unwrap();
        fs::create_dir_all(root.join("2026")).unwrap();
        fs::write(root.join("2026/real.NEF"), b"x").unwrap();
        // a stray raw file that somehow ended up in the output tree
        fs::write(out.join("stray.NEF"), b"x").unwrap();

        let found = collect_raw_files(&root, &out);

        assert_eq!(found, vec![root.join("2026/real.NEF")]);
    }
}
