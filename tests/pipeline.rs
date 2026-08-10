//! The full path the Convert button drives: scan a folder, work out targets,
//! convert. Exercised without GTK so it can run in CI.
//!
//! Set NEFTODNG_TEST_DIR to a folder containing NEFs to run it.

use std::fs;
use std::path::{Path, PathBuf};

use neftodng::convert::{convert_file, Outcome};
use neftodng::paths::{base_for_folder, output_path, AddSource};
use neftodng::scan::collect_raw_files;
use neftodng::summary::BatchSummary;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("neftodng-pipeline-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch");
    dir
}

#[test]
fn scans_a_folder_and_converts_every_file_into_a_mirrored_tree() {
    let Ok(source_dir) = std::env::var("NEFTODNG_TEST_DIR") else {
        eprintln!("skipping: set NEFTODNG_TEST_DIR to a folder of NEFs to run this test");
        return;
    };
    let source_dir = PathBuf::from(source_dir);
    let out_root = scratch("out");

    let files = collect_raw_files(&source_dir, &out_root);
    assert!(!files.is_empty(), "no NEFs found in {source_dir:?}");

    let add = AddSource::Folder {
        base: base_for_folder(&source_dir),
    };

    let mut converted = 0;
    let mut summary = BatchSummary::default();
    for source in &files {
        let target = output_path(source, &add, &out_root);

        // every target must stay inside the output root
        assert!(
            target.starts_with(&out_root),
            "{target:?} escaped the output root"
        );
        // and mirror the source's own folder name
        let leaf = source_dir.file_name().expect("source dir name");
        assert!(
            target.to_string_lossy().contains(&*leaf.to_string_lossy()),
            "{target:?} did not mirror {leaf:?}"
        );

        match convert_file(source, &target, false).expect("conversion") {
            Outcome::Converted {
                source_bytes,
                target_bytes,
            } => {
                converted += 1;
                summary.record(&Outcome::Converted {
                    source_bytes,
                    target_bytes,
                });
            }
            Outcome::Skipped => panic!("nothing should pre-exist in a fresh output root"),
        }
        assert!(target.exists(), "{target:?} was not written");
    }

    assert_eq!(converted, files.len(), "every file should have converted");

    // the figure the completion dialog reports must match the files on disk
    let source_total: u64 = files
        .iter()
        .map(|p| fs::metadata(p).expect("source metadata").len())
        .sum();
    let target_total: u64 = files
        .iter()
        .map(|p| {
            fs::metadata(output_path(p, &add, &out_root))
                .expect("target metadata")
                .len()
        })
        .sum();
    assert_eq!(
        summary.saved_bytes,
        source_total as i64 - target_total as i64,
        "reported saving must match actual bytes on disk"
    );
    assert!(summary.saved_bytes > 0, "DNG should be smaller than NEF here");
    assert!(no_part_files(&out_root), "no .part files may survive");

    // sources must be exactly as they were
    for source in &files {
        assert!(source.exists(), "{source:?} was removed");
    }

    // running again must skip rather than overwrite
    let second = convert_file(&files[0], &output_path(&files[0], &add, &out_root), false)
        .expect("second run");
    assert_eq!(second, Outcome::Skipped, "a second run must not overwrite");
}

fn no_part_files(dir: &Path) -> bool {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.to_string_lossy().ends_with(".part") {
                return false;
            }
        }
    }
    true
}
