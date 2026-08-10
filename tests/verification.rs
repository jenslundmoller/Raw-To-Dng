//! Proof that verification catches the failure this feature exists for: a DNG
//! that looks fine but does not hold the original's data.
//!
//! Set NEFTODNG_TEST_DIR to a folder with at least two NEFs to run these.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use neftodng::convert::{convert_file, Outcome};
use neftodng::exif_tags::{read_lens_tags, LENS_MODEL};
use neftodng::verify::{compare_exif, compare_lens_tags, compare_metadata, verify_against_source};

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
fn colour_and_level_metadata_survives_a_real_conversion() {
    let Some((nef, _)) = two_fixtures() else {
        eprintln!("skipping: set NEFTODNG_TEST_DIR to a folder with two NEFs");
        return;
    };
    let dir = scratch("meta");
    let dng = dir.join("out.dng");
    convert_file(&nef, &dng, false).expect("conversion should succeed");

    let src = rawler::decode_file(&nef).expect("decode nef");
    let dst = rawler::decode_file(&dng).expect("decode dng");

    assert_eq!(
        compare_metadata(&src, &dst),
        Ok(()),
        "colour matrices, levels and geometry must survive"
    );
    // Both illuminants must actually be present, or the check above would be
    // passing vacuously.
    assert!(
        src.color_matrix.len() >= 2,
        "expected a dual-illuminant camera profile, got {}",
        src.color_matrix.len()
    );
}

#[test]
fn a_wrong_white_level_is_detected() {
    let Some((nef, _)) = two_fixtures() else {
        eprintln!("skipping: set NEFTODNG_TEST_DIR to a folder with two NEFs");
        return;
    };
    let dir = scratch("whitelevel");
    let dng = dir.join("out.dng");
    convert_file(&nef, &dng, false).expect("conversion should succeed");

    let src = rawler::decode_file(&nef).expect("decode nef");
    let mut dst = rawler::decode_file(&dng).expect("decode dng");

    // A wrong white level clips highlights and shifts exposure, while leaving
    // every pixel value untouched — invisible to a sample comparison.
    dst.whitelevel = rawler::rawimage::WhiteLevel(vec![12000]);

    let result = compare_metadata(&src, &dst);
    assert!(result.is_err(), "a shifted white level must be caught");
    eprintln!("detected: {}", result.unwrap_err());
}

#[test]
fn a_wrong_colour_matrix_is_detected() {
    let Some((nef, _)) = two_fixtures() else {
        eprintln!("skipping: set NEFTODNG_TEST_DIR to a folder with two NEFs");
        return;
    };
    let dir = scratch("matrix");
    let dng = dir.join("out.dng");
    convert_file(&nef, &dng, false).expect("conversion should succeed");

    let src = rawler::decode_file(&nef).expect("decode nef");
    let mut dst = rawler::decode_file(&dng).expect("decode dng");

    // Perturb one coefficient: colours would render wrong, pixels identical.
    let key = *src.color_matrix.keys().next().expect("an illuminant");
    if let Some(m) = dst.color_matrix.get_mut(&key) {
        m[0] += 0.25;
    }

    let result = compare_metadata(&src, &dst);
    assert!(result.is_err(), "a perturbed colour matrix must be caught");
    eprintln!("detected: {}", result.unwrap_err());
}

#[test]
fn lens_and_shot_data_survive_a_real_conversion() {
    let Some((nef, _)) = two_fixtures() else {
        eprintln!("skipping: set NEFTODNG_TEST_DIR to a folder with two NEFs");
        return;
    };
    let dir = scratch("exif");
    let dng = dir.join("out.dng");
    convert_file(&nef, &dng, false).expect("conversion should succeed");

    // Non-vacuous: the source must actually carry the tags, or a check that
    // compares nothing would pass and prove nothing.
    let tags = read_lens_tags(&nef);
    assert!(
        tags.contains_key(&LENS_MODEL),
        "fixture has no lens model to compare; this test would be meaningless"
    );

    assert_eq!(
        compare_lens_tags(&nef, &dng),
        Ok(()),
        "lens tags must survive despite DNG normalising their case"
    );
    assert_eq!(
        compare_exif(&nef, &dng),
        Ok(()),
        "timestamps, exposure and GPS must survive"
    );
}

#[test]
fn a_file_that_dropped_the_lens_tags_is_rejected() {
    let Some((nef, _)) = two_fixtures() else {
        eprintln!("skipping: set NEFTODNG_TEST_DIR to a folder with two NEFs");
        return;
    };
    let dir = scratch("nolens");
    let stripped = dir.join("no-lens.tif");

    // A valid little-endian TIFF with an empty IFD: parses, carries no lens.
    let mut bytes = vec![0x49, 0x49, 0x2A, 0x00];
    bytes.extend_from_slice(&8u32.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    fs::write(&stripped, &bytes).expect("write stripped file");

    let result = compare_lens_tags(&nef, &stripped);
    assert!(
        result.is_err(),
        "losing the lens model must be caught, not ignored: {result:?}"
    );
    eprintln!("detected: {}", result.unwrap_err());
}

/// Regression: portrait shots were rejected. rawler's NEF decoder never
/// populates `RawImage.orientation`, leaving it `Normal`, while its DNG decoder
/// reads the real tag and reports `Rotate90`. Comparing that field failed every
/// portrait photograph even though the EXIF orientation round-tripped correctly.
///
/// Set NEFTODNG_TEST_PORTRAIT_NEF to a portrait raw file to run this.
#[test]
fn a_portrait_photograph_verifies() {
    let Ok(fixture) = std::env::var("NEFTODNG_TEST_PORTRAIT_NEF") else {
        eprintln!("skipping: set NEFTODNG_TEST_PORTRAIT_NEF to a portrait raw file");
        return;
    };
    let nef = PathBuf::from(fixture);
    let dir = scratch("portrait");
    let dng = dir.join("out.dng");

    // Non-vacuous: this must really be a rotated shot, or it proves nothing.
    let meta = rawler::decoders::RawMetadata::default();
    let _ = meta;
    let src_orientation = {
        let s = rawler::rawsource::RawSource::new(&nef).expect("open");
        let d = rawler::get_decoder(&s).expect("decoder");
        d.raw_metadata(&s, &rawler::decoders::RawDecodeParams::default())
            .expect("metadata")
            .exif
            .orientation
    };
    assert!(
        matches!(src_orientation, Some(o) if o != 1),
        "fixture is not a rotated shot (orientation {src_orientation:?}); test would be meaningless"
    );

    let outcome = convert_file(&nef, &dng, false);
    assert!(
        outcome.is_ok(),
        "a portrait photograph must convert and verify: {outcome:?}"
    );

    // And the orientation must genuinely survive, via the EXIF tag.
    assert_eq!(compare_exif(&nef, &dng), Ok(()));
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
