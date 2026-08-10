//! Converting one NEF to one DNG, safely.
//!
//! Two hazards are handled here. `rawler` deliberately prefers panicking over
//! defensive error handling on malformed input, so a single corrupt file could
//! otherwise take down a whole batch. And a conversion interrupted by a crash,
//! a cancel or a full disk must never leave a truncated `.dng` behind that could
//! later be mistaken for a good one.

use std::fmt;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;

use rawler::dng::convert::{convert_raw_file, ConvertParams};

use crate::paths::part_path;
use crate::verify::{verify_against_source, VerifyError};

/// What happened to one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A DNG was written, with the sizes of both files so the batch can report
    /// how much space was saved.
    Converted { source_bytes: u64, target_bytes: u64 },
    /// The target already existed and overwriting was not requested.
    Skipped,
}

/// Why one file could not be converted.
#[derive(Debug)]
pub enum ConvertError {
    /// The raw file could not be decoded, or the DNG could not be encoded.
    Decode(String),
    /// `rawler` panicked on malformed input; contained rather than fatal.
    Panicked(String),
    /// The output could not be created, written or renamed.
    Io(std::io::Error),
    /// The DNG was written but could not be proven to match its source, so it
    /// was discarded rather than presented as a successful conversion.
    Unverified(VerifyError),
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(m) => write!(f, "could not convert: {m}"),
            Self::Panicked(m) => write!(f, "corrupt or unsupported file ({m})"),
            Self::Io(e) => write!(f, "write failed: {e}"),
            Self::Unverified(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ConvertError {}

/// Convert `source` to a DNG at `target`.
///
/// Writes via a hidden `.part` file and renames on success, so `target` either
/// does not exist or is complete. Never modifies or removes `source`.
pub fn convert_file(
    source: &Path,
    target: &Path,
    overwrite: bool,
) -> Result<Outcome, ConvertError> {
    if target.exists() && !overwrite {
        return Ok(Outcome::Skipped);
    }

    if let Some(dir) = target.parent() {
        fs::create_dir_all(dir).map_err(ConvertError::Io)?;
    }

    let part = part_path(target);
    // A .part here is debris from a previous run that was killed mid-write;
    // nothing may be appended to it.
    let _ = fs::remove_file(&part);

    let params = ConvertParams {
        // The source is never deleted, so embedding a second copy of it inside
        // every DNG would roughly double the archive for no benefit.
        embedded: false,
        software: concat!("NefToDng ", env!("CARGO_PKG_VERSION")).to_string(),
        ..Default::default()
    };

    // rawler panics on some malformed input by design, so a corrupt file must
    // not be allowed to unwind past here and kill the batch.
    let written = panic::catch_unwind(AssertUnwindSafe(|| -> Result<(), ConvertError> {
        let file = File::create(&part).map_err(ConvertError::Io)?;
        let mut writer = BufWriter::new(file);

        convert_raw_file(source, &mut writer, &params)
            .map_err(|e| ConvertError::Decode(format!("{e}")))?;

        writer.flush().map_err(ConvertError::Io)?;
        writer
            .into_inner()
            .map_err(|e| ConvertError::Io(e.into_error()))?
            .sync_all()
            .map_err(ConvertError::Io)
    }));

    // Prove the written file carries the same raw samples as the source before
    // it is allowed to become a .dng. Verifying the .part means a failure never
    // leaves a file that looks like a finished conversion.
    let written = match written {
        Ok(Ok(())) => match verify_against_source(source, &part) {
            Ok(()) => Ok(Ok(())),
            Err(e) => Ok(Err(ConvertError::Unverified(e))),
        },
        other => other,
    };

    let failure = match written {
        Ok(Ok(())) => match fs::rename(&part, target) {
            Ok(()) => {
                // Measured after the rename so the figures describe files that
                // actually exist; unreadable metadata reports zero rather than
                // failing a conversion that succeeded.
                let source_bytes = fs::metadata(source).map(|m| m.len()).unwrap_or(0);
                let target_bytes = fs::metadata(target).map(|m| m.len()).unwrap_or(0);
                return Ok(Outcome::Converted {
                    source_bytes,
                    target_bytes,
                });
            }
            Err(e) => ConvertError::Io(e),
        },
        Ok(Err(e)) => e,
        Err(payload) => ConvertError::Panicked(panic_message(&payload)),
    };

    // Nothing half-written may survive a failure.
    let _ = fs::remove_file(&part);
    Err(failure)
}

fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}
