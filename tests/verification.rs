//! Proof that verification catches the failure this feature exists for: a DNG
//! that looks fine but does not hold the original's data.
//!
//! Set NEFTODNG_TEST_DIR to a folder with at least two NEFs to run these.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use neftodng::convert::{convert_file, Outcome};
use neftodng::verify::verify_against_source;

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn scratch(name: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("neftodng-verify-{}-{n}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch");
    dir
}

/// Two different NEFs from the fixture folder.
fn two_fixtures() -> Option<(PathBuf, PathBuf)> {
    let dir = std::env::var("NEFTODNG_TEST_DIR").ok()?;
    let mut nefs: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("nef"))
        })
        .collect();
    nefs.sort();
    if nefs.len() < 2 {
        return None;
    }
    Some((nefs[0].clone(), nefs[1].clone()))
}

#[test]
fn a_real_conversion_verifies_against_its_source() {
    let Some((nef, _)) = two_fixtures() else {
        eprintln!("skipping: set NEFTODNG_TEST_DIR to a folder with two NEFs");
        return;
    };
    let dir = scratch("match");
    let dng = dir.join("out.dng");

    let outcome = convert_file(&nef, &dng, false).expect("conversion should succeed");
    assert!(matches!(outcome, Outcome::Converted { .. }));

    // convert_file already verified internally; doing it again explicitly
    // documents the guarantee.
    assert_eq!(
        verify_against_source(&nef, &dng),
        Ok(()),
        "a genuine conversion must verify bit-for-bit"
    );
}

#[test]
fn a_dng_holding_a_different_photo_is_rejected() {
    let Some((nef_a, nef_b)) = two_fixtures() else {
        eprintln!("skipping: set NEFTODNG_TEST_DIR to a folder with two NEFs");
        return;
    };
    let dir = scratch("wrongphoto");
    let dng = dir.join("from_a.dng");

    convert_file(&nef_a, &dng, false).expect("conversion should succeed");

    // Same camera, same dimensions, valid DNG — but the wrong picture. Only a
    // pixel-level check can tell these apart.
    let result = verify_against_source(&nef_b, &dng);
    assert!(
        result.is_err(),
        "a DNG of a different photo must not verify: {result:?}"
    );
}

#[test]
fn a_silently_corrupted_dng_is_rejected() {
    let Some((nef, _)) = two_fixtures() else {
        eprintln!("skipping: set NEFTODNG_TEST_DIR to a folder with two NEFs");
        return;
    };
    let dir = scratch("corrupted");
    let dng = dir.join("out.dng");

    convert_file(&nef, &dng, false).expect("conversion should succeed");
    let good = fs::metadata(&dng).expect("metadata").len();

    // Flip bits deep inside the image data, leaving the file the right size and
    // the header intact: exactly the "looks fine, is not" case.
    let mut bytes = fs::read(&dng).expect("read dng");
    let start = bytes.len() / 2;
    for b in bytes.iter_mut().skip(start).take(4096) {
        *b ^= 0xFF;
    }
    fs::write(&dng, &bytes).expect("write corrupted dng");

    assert_eq!(
        fs::metadata(&dng).expect("metadata").len(),
        good,
        "the corrupted file is the same size, so a size check would pass it"
    );

    let result = verify_against_source(&nef, &dng);
    assert!(
        result.is_err(),
        "a corrupted DNG must be rejected: {result:?}"
    );
    eprintln!("corruption detected as: {}", result.unwrap_err());
}
