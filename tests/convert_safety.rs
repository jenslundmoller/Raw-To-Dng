//! The safety properties that protect an archive. These matter more than any
//! feature: they are what stands between a batch run and a lost negative.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use neftodng::convert::{convert_file, Outcome};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A unique scratch directory, removed by the OS eventually; kept simple so the
/// tests have no dependencies.
fn scratch(name: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("neftodng-test-{}-{}-{}", std::process::id(), n, name));
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Leftover `.part` files anywhere under `dir`.
fn part_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.to_string_lossy().ends_with(".part") {
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn corrupt_input_is_reported_not_fatal_and_leaves_nothing_behind() {
    let dir = scratch("corrupt");
    let src = dir.join("broken.NEF");
    let dst = dir.join("broken.dng");

    // Plausible enough to get past an extension check, garbage to a decoder.
    let mut bytes = b"II*\0".to_vec();
    bytes.extend(std::iter::repeat_n(0xA5u8, 4096));
    fs::write(&src, &bytes).expect("write corrupt input");

    let result = convert_file(&src, &dst, false);

    assert!(result.is_err(), "corrupt input must be an error, got {result:?}");
    assert!(!dst.exists(), "no .dng may be left after a failure");
    assert!(part_files(&dir).is_empty(), "no .part may be left after a failure");
    assert!(src.exists(), "the source must never be removed");
}

#[test]
fn existing_target_is_skipped_rather_than_overwritten() {
    let dir = scratch("existing");
    let src = dir.join("a.NEF");
    let dst = dir.join("a.dng");

    fs::write(&src, b"II*\0whatever").expect("write source");
    fs::write(&dst, b"PRECIOUS EXISTING DNG").expect("write existing target");

    let outcome = convert_file(&src, &dst, false).expect("skip must not be an error");

    assert_eq!(outcome, Outcome::Skipped);
    assert_eq!(
        fs::read(&dst).expect("read target"),
        b"PRECIOUS EXISTING DNG",
        "existing file must be untouched"
    );
}

/// The case the whole panic-isolation design exists for: a real NEF whose data
/// is cut short, which is what a failed card read or an interrupted copy looks
/// like. Set NEFTODNG_TEST_NEF to run it.
#[test]
fn truncated_real_nef_is_contained_and_does_not_kill_the_process() {
    let Ok(fixture) = std::env::var("NEFTODNG_TEST_NEF") else {
        eprintln!("skipping: set NEFTODNG_TEST_NEF to a .NEF path to run this test");
        return;
    };

    let dir = scratch("truncated");
    let src = dir.join("truncated.NEF");
    let dst = dir.join("truncated.dng");

    let whole = fs::read(&fixture).expect("read fixture");
    fs::write(&src, &whole[..whole.len() / 2]).expect("write truncated copy");

    let result = convert_file(&src, &dst, false);

    // The point is not which error comes back, but that one comes back at all
    // rather than unwinding out of the worker and taking the batch with it.
    assert!(result.is_err(), "truncated input must be an error, got {result:?}");
    eprintln!("truncated NEF produced: {}", result.unwrap_err());
    assert!(!dst.exists(), "no .dng may be left behind");
    assert!(part_files(&dir).is_empty(), "no .part may be left behind");
}

/// End-to-end against a real camera file. Set NEFTODNG_TEST_NEF to run it.
#[test]
fn real_nef_converts_to_a_dng_and_leaves_no_part_file() {
    let Ok(fixture) = std::env::var("NEFTODNG_TEST_NEF") else {
        eprintln!("skipping: set NEFTODNG_TEST_NEF to a .NEF path to run this test");
        return;
    };
    let src = PathBuf::from(fixture);
    assert!(src.exists(), "fixture {src:?} does not exist");

    let dir = scratch("real");
    let dst = dir.join("out.dng");

    let outcome = convert_file(&src, &dst, false).expect("conversion should succeed");

    match outcome {
        Outcome::Converted {
            source_bytes,
            target_bytes,
        } => {
            assert_eq!(
                source_bytes,
                fs::metadata(&src).expect("source metadata").len(),
                "reported source size must match the file on disk"
            );
            assert_eq!(
                target_bytes,
                fs::metadata(&dst).expect("target metadata").len(),
                "reported target size must match the file on disk"
            );
        }
        other => panic!("expected Converted, got {other:?}"),
    }
    assert!(dst.exists(), "a .dng must exist");
    assert!(part_files(&dir).is_empty(), "no .part may survive a success");

    let head = fs::read(&dst).expect("read dng");
    assert_eq!(&head[..2], b"II", "output must be a little-endian TIFF/DNG");
    assert!(head.len() > 1_000_000, "output looks implausibly small");

    let src_len = fs::metadata(&src).expect("source metadata").len();
    assert!(src_len > 0, "the source must still be intact");
}
