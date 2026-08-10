//! Proving a written DNG holds the same raw data as the NEF it came from.
//!
//! This exists so that "converted without errors" can be relied on when
//! deciding to delete originals. Verification runs against the `.part` file
//! before it is renamed, so a `.dng` only ever appears once it has been proven
//! to match.

use std::fmt;
use std::path::Path;

use rawler::rawimage::RawImageData;

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

    compare_samples(&src.data, &dst.data)
}

#[cfg(test)]
mod tests {
    use super::*;

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
