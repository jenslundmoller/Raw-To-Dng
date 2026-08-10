//! Proving a written DNG holds the same raw data as the NEF it came from.
//!
//! This exists so that "converted without errors" can be relied on when
//! deciding to delete originals. Verification runs against the `.part` file
//! before it is renamed, so a `.dng` only ever appears once it has been proven
//! to match.

use std::fmt;
use std::path::Path;

use rawler::rawimage::RawImageData;

use crate::exif_tags::{read_lens_tags, COMPARED};

/// Why a written file could not be shown to match its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// The written file could not be decoded at all.
    Unreadable(String),
    /// The images describe different geometry.
    Dimensions {
        source: (usize, usize),
        written: (usize, usize),
    },
    /// Different number of samples, so they cannot be the same image.
    SampleCount { source: usize, written: usize },
    /// Same shape, different content. The dangerous case: a plausible file
    /// holding wrong data.
    Pixels { differing: usize, total: usize },
    /// One is integer data and the other floating point.
    Format,
    /// Colour, level or geometry metadata differs.
    Metadata {
        field: &'static str,
        source: String,
        written: String,
    },
}

/// How far white balance coefficients may drift and still be considered equal.
///
/// DNG stores `AsShotNeutral` as rationals, so exact float equality is not
/// achievable by design; a Nikon Z 6 file round-trips with about 1.4e-5 relative
/// error. This bound is comfortably above that and still thousands of times
/// tighter than any visually meaningful white balance difference.
pub const WB_RELATIVE_TOLERANCE: f32 = 1e-4;

/// Whether two white balance coefficient sets agree within tolerance.
///
/// Unused channels are `NaN`, and two `NaN`s in the same position count as
/// agreeing rather than as a mismatch.
pub fn white_balance_matches(source: &[f32; 4], written: &[f32; 4]) -> bool {
    source.iter().zip(written.iter()).all(|(a, b)| {
        match (a.is_nan(), b.is_nan()) {
            // Both channels unused: agreement, not a mismatch.
            (true, true) => true,
            // One side gained or lost a channel: a real difference.
            (true, false) | (false, true) => false,
            (false, false) => {
                let scale = a.abs().max(b.abs()).max(1.0);
                (a - b).abs() <= WB_RELATIVE_TOLERANCE * scale
            }
        }
    })
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(e) => write!(f, "verification failed: DNG could not be read back ({e})"),
            Self::Dimensions { source, written } => write!(
                f,
                "verification failed: {}x{} written from a {}x{} source",
                written.0, written.1, source.0, source.1
            ),
            Self::SampleCount { source, written } => write!(
                f,
                "verification failed: {written} samples written from {source}"
            ),
            Self::Pixels { differing, total } => write!(
                f,
                "verification failed: {differing} of {total} samples differ from the original"
            ),
            Self::Format => write!(f, "verification failed: sample format changed"),
            Self::Metadata {
                field,
                source,
                written,
            } => write!(
                f,
                "verification failed: {field} differs (original {source}, written {written})"
            ),
        }
    }
}

impl std::error::Error for VerifyError {}

/// Compare the raw samples of a source and a written image.
pub fn compare_samples(source: &RawImageData, written: &RawImageData) -> Result<(), VerifyError> {
    match (source, written) {
        (RawImageData::Integer(a), RawImageData::Integer(b)) => count_differences(a, b, |x, y| x == y),
        // Bit equality, not approximate equality: a lossless conversion that
        // merely rounds to something close has still lost data.
        (RawImageData::Float(a), RawImageData::Float(b)) => {
            count_differences(a, b, |x, y| x.to_bits() == y.to_bits())
        }
        _ => Err(VerifyError::Format),
    }
}

fn count_differences<T>(
    source: &[T],
    written: &[T],
    equal: impl Fn(&T, &T) -> bool,
) -> Result<(), VerifyError> {
    if source.len() != written.len() {
        return Err(VerifyError::SampleCount {
            source: source.len(),
            written: written.len(),
        });
    }

    let differing = source
        .iter()
        .zip(written.iter())
        .filter(|(a, b)| !equal(a, b))
        .count();

    if differing == 0 {
        Ok(())
    } else {
        Err(VerifyError::Pixels {
            differing,
            total: source.len(),
        })
    }
}

/// Compare the metadata a renderer depends on: colour, levels and geometry.
///
/// Everything here round-trips exactly on a real Nikon file, so equality is
/// demanded, with the sole exception of white balance.
pub fn compare_metadata(
    src: &rawler::rawimage::RawImage,
    dst: &rawler::rawimage::RawImage,
) -> Result<(), VerifyError> {
    macro_rules! same {
        ($field:expr, $a:expr, $b:expr) => {
            if $a != $b {
                return Err(VerifyError::Metadata {
                    field: $field,
                    source: format!("{:?}", $a),
                    written: format!("{:?}", $b),
                });
            }
        };
    }

    // Colour: the matrices decide how the raw values become colour at all.
    let mut illuminants: Vec<_> = src.color_matrix.keys().collect();
    illuminants.sort_by_key(|i| format!("{i:?}"));
    for illuminant in illuminants {
        match dst.color_matrix.get(illuminant) {
            Some(written) => same!(
                "colour matrix",
                (format!("{illuminant:?}"), &src.color_matrix[illuminant]),
                (format!("{illuminant:?}"), written)
            ),
            None => {
                return Err(VerifyError::Metadata {
                    field: "colour matrix",
                    source: format!("{illuminant:?} present"),
                    written: format!("{illuminant:?} missing"),
                })
            }
        }
    }

    if !white_balance_matches(&src.wb_coeffs, &dst.wb_coeffs) {
        return Err(VerifyError::Metadata {
            field: "white balance",
            source: format!("{:?}", src.wb_coeffs),
            written: format!("{:?}", dst.wb_coeffs),
        });
    }

    // Levels: wrong black or white levels shift exposure and clip highlights.
    same!("white level", &src.whitelevel, &dst.whitelevel);
    same!("black level", &src.blacklevel, &dst.blacklevel);

    // Geometry and layout: a wrong CFA pattern swaps the colour channels.
    same!(
        "sensor colour filter layout",
        format!("{:?}", src.photometric),
        format!("{:?}", dst.photometric)
    );
    same!("components per pixel", src.cpp, dst.cpp);
    // RawImage.orientation is deliberately not compared. rawler's NEF decoder
    // never populates it, leaving Normal, while its DNG decoder reads the real
    // tag — so every portrait shot compared Normal against Rotate90 and was
    // rejected despite converting correctly. Orientation is still verified, from
    // the authoritative EXIF tag, in compare_exif.
    same!("active area", src.active_area, dst.active_area);
    same!("crop area", src.crop_area, dst.crop_area);

    Ok(())
}

fn raw_metadata(path: &Path) -> Result<rawler::decoders::RawMetadata, VerifyError> {
    let source =
        rawler::rawsource::RawSource::new(path).map_err(|e| VerifyError::Unreadable(e.to_string()))?;
    let decoder =
        rawler::get_decoder(&source).map_err(|e| VerifyError::Unreadable(e.to_string()))?;
    decoder
        .raw_metadata(&source, &rawler::decoders::RawDecodeParams::default())
        .map_err(|e| VerifyError::Unreadable(e.to_string()))
}

/// Compare the shot data a catalogue depends on: timestamps, exposure, lens and
/// GPS.
///
/// Only fields measured to round-trip are compared. `modify_date` is excluded
/// because the conversion legitimately sets it to now. Lens make, model and
/// specification are excluded here and checked from the file's tags instead,
/// because rawler reads them from a NEF but not from a DNG even though the DNG
/// carries them.
pub fn compare_exif(source: &Path, written: &Path) -> Result<(), VerifyError> {
    let a = raw_metadata(source)?;
    let b = raw_metadata(written)?;

    macro_rules! same {
        ($field:expr, $a:expr, $b:expr) => {
            if $a != $b {
                return Err(VerifyError::Metadata {
                    field: $field,
                    source: format!("{:?}", $a),
                    written: format!("{:?}", $b),
                });
            }
        };
    }

    same!("camera make", a.make, b.make);
    same!("camera model", a.model, b.model);

    let (x, y) = (&a.exif, &b.exif);

    // When the photograph was taken.
    same!("capture time", x.date_time_original, y.date_time_original);
    same!("creation time", x.create_date, y.create_date);
    same!("capture sub-seconds", x.sub_sec_time_original, y.sub_sec_time_original);
    same!("capture timezone", x.offset_time_original, y.offset_time_original);

    // Who and what took it.
    same!("camera serial number", x.serial_number, y.serial_number);
    same!("artist", x.artist, y.artist);
    same!("copyright", x.copyright, y.copyright);
    same!("owner name", x.owner_name, y.owner_name);
    same!("user comment", x.user_comment, y.user_comment);

    // Exposure.
    same!("exposure time", x.exposure_time, y.exposure_time);
    same!("f-number", x.fnumber, y.fnumber);
    same!("aperture", x.aperture_value, y.aperture_value);
    same!("ISO", x.iso_speed_ratings, y.iso_speed_ratings);
    same!("ISO speed", x.iso_speed, y.iso_speed);
    same!("exposure bias", x.exposure_bias, y.exposure_bias);
    same!("exposure program", x.exposure_program, y.exposure_program);
    same!("exposure mode", x.exposure_mode, y.exposure_mode);
    same!("metering mode", x.metering_mode, y.metering_mode);
    same!("shutter speed", x.shutter_speed_value, y.shutter_speed_value);
    same!("max aperture", x.max_aperture_value, y.max_aperture_value);
    same!("flash", x.flash, y.flash);
    same!("light source", x.light_source, y.light_source);
    same!("white balance mode", x.white_balance, y.white_balance);
    same!("brightness", x.brightness_value, y.brightness_value);
    same!("sensitivity type", x.sensitivity_type, y.sensitivity_type);
    same!(
        "recommended exposure index",
        x.recommended_exposure_index,
        y.recommended_exposure_index
    );

    // Lens, less the fields rawler cannot read back from a DNG.
    same!("focal length", x.focal_length, y.focal_length);
    same!("subject distance", x.subject_distance, y.subject_distance);

    // Where it was taken.
    same!("GPS", x.gps, y.gps);

    same!("EXIF orientation", x.orientation, y.orientation);
    same!("colour space", x.color_space, y.color_space);
    same!("scene capture type", x.scene_capture_type, y.scene_capture_type);

    compare_lens_tags(source, written)
}

/// Compare lens tags read straight from the files.
///
/// DNG normalises the case of lens names and re-encodes rationals, so values are
/// compared by meaning rather than byte equality. A tag missing from the source
/// is not required in the output; a tag present in the source must survive.
pub fn compare_lens_tags(source: &Path, written: &Path) -> Result<(), VerifyError> {
    let a = read_lens_tags(source);
    let b = read_lens_tags(written);

    for (tag, name) in COMPARED {
        let Some(expected) = a.get(&tag) else {
            continue;
        };
        match b.get(&tag) {
            Some(actual) if expected.equivalent(actual) => {}
            Some(actual) => {
                return Err(VerifyError::Metadata {
                    field: name,
                    source: expected.describe(),
                    written: actual.describe(),
                })
            }
            None => {
                return Err(VerifyError::Metadata {
                    field: name,
                    source: expected.describe(),
                    written: "missing".to_string(),
                })
            }
        }
    }

    Ok(())
}

/// Decode `written` and prove it carries the same raw data as `source`.
pub fn verify_against_source(source: &Path, written: &Path) -> Result<(), VerifyError> {
    let src = rawler::decode_file(source).map_err(|e| VerifyError::Unreadable(e.to_string()))?;
    let dst = rawler::decode_file(written).map_err(|e| VerifyError::Unreadable(e.to_string()))?;

    if (src.width, src.height) != (dst.width, dst.height) {
        return Err(VerifyError::Dimensions {
            source: (src.width, src.height),
            written: (dst.width, dst.height),
        });
    }

    compare_samples(&src.data, &dst.data)?;
    compare_metadata(&src, &dst)?;
    compare_exif(source, written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_white_balance_matches() {
        let a = [1.5820313, 1.0, 1.4882813, f32::NAN];
        assert!(white_balance_matches(&a, &a));
    }

    #[test]
    fn the_rational_rounding_dng_applies_is_tolerated() {
        // Values measured from a real Nikon Z 6 NEF and the DNG made from it.
        let nef = [1.5820313, 1.0, 1.4882813, f32::NAN];
        let dng = [1.5820533, 1.0, 1.4882946, f32::NAN];

        assert!(
            white_balance_matches(&nef, &dng),
            "DNG stores AsShotNeutral as rationals, so this drift is unavoidable"
        );
    }

    #[test]
    fn a_meaningfully_different_white_balance_is_rejected() {
        let nef = [1.5820313, 1.0, 1.4882813, f32::NAN];
        let wrong = [1.6, 1.0, 1.4882813, f32::NAN];

        assert!(
            !white_balance_matches(&nef, &wrong),
            "a 1% shift is a real difference, not rounding"
        );
    }

    #[test]
    fn an_unused_channel_becoming_a_number_is_a_mismatch() {
        let nef = [1.58, 1.0, 1.48, f32::NAN];
        let written = [1.58, 1.0, 1.48, 1.0];

        assert!(!white_balance_matches(&nef, &written));
        assert!(!white_balance_matches(&written, &nef));
    }

    #[test]
    fn identical_samples_verify() {
        let a = RawImageData::Integer(vec![1, 2, 3, 4]);
        let b = RawImageData::Integer(vec![1, 2, 3, 4]);

        assert_eq!(compare_samples(&a, &b), Ok(()));
    }

    #[test]
    fn a_single_differing_sample_is_caught() {
        // The case a size check cannot see: right shape, wrong content.
        let a = RawImageData::Integer(vec![1, 2, 3, 4]);
        let b = RawImageData::Integer(vec![1, 2, 9, 4]);

        assert_eq!(
            compare_samples(&a, &b),
            Err(VerifyError::Pixels {
                differing: 1,
                total: 4
            })
        );
    }

    #[test]
    fn a_truncated_sample_run_is_caught() {
        let a = RawImageData::Integer(vec![1, 2, 3, 4]);
        let b = RawImageData::Integer(vec![1, 2]);

        assert_eq!(
            compare_samples(&a, &b),
            Err(VerifyError::SampleCount {
                source: 4,
                written: 2
            })
        );
    }

    #[test]
    fn a_change_of_sample_format_is_caught() {
        let a = RawImageData::Integer(vec![1, 2]);
        let b = RawImageData::Float(vec![1.0, 2.0]);

        assert_eq!(compare_samples(&a, &b), Err(VerifyError::Format));
    }

    #[test]
    fn float_samples_compare_by_exact_bits() {
        let a = RawImageData::Float(vec![1.5, 2.5]);
        let b = RawImageData::Float(vec![1.5, 2.5]);
        assert_eq!(compare_samples(&a, &b), Ok(()));

        // One ULP away — the smallest representable difference, which a literal
        // like 2.5000001 cannot express because it rounds back to 2.5.
        let one_ulp_off = f32::from_bits(2.5f32.to_bits() + 1);
        let c = RawImageData::Float(vec![1.5, one_ulp_off]);
        assert!(
            compare_samples(&a, &c).is_err(),
            "a lossless conversion must reproduce bits exactly, not approximately"
        );
    }
}
