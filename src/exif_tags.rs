//! A deliberately small TIFF/EXIF tag reader.
//!
//! `rawler` decodes NEF lens tags but does not populate them when reading a DNG
//! back, so lens metadata cannot be compared through its API even though the
//! tags are present in both files. This reads the handful of tags needed to
//! compare them directly.
//!
//! Only what verification needs is supported: ASCII and RATIONAL values in IFD0
//! and the EXIF sub-IFD.

use std::collections::BTreeMap;
use std::path::Path;

/// EXIF tag numbers this module knows about.
pub const LENS_MAKE: u16 = 0xA433;
pub const LENS_MODEL: u16 = 0xA434;
pub const LENS_SERIAL: u16 = 0xA435;
pub const LENS_SPEC: u16 = 0xA432;

/// The tags compared during verification, with names for error messages.
pub const COMPARED: [(u16, &str); 4] = [
    (LENS_MAKE, "lens make"),
    (LENS_MODEL, "lens model"),
    (LENS_SERIAL, "lens serial number"),
    (LENS_SPEC, "lens specification"),
];

/// A tag value in the only two shapes this reader needs.
#[derive(Debug, Clone, PartialEq)]
pub enum TagValue {
    Text(String),
    /// Rationals reduced to their numeric value, because DNG re-encodes them
    /// with different numerators and denominators for the same number.
    Numbers(Vec<f64>),
}

impl TagValue {
    /// Whether two values mean the same thing.
    ///
    /// Text is compared case-insensitively and ignoring surrounding whitespace,
    /// because DNG normalises `NIKON` to `Nikon`. Numbers are compared by value,
    /// because `240/10` and `24/1` are the same focal length.
    pub fn equivalent(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Text(a), Self::Text(b)) => a.trim().eq_ignore_ascii_case(b.trim()),
            (Self::Numbers(a), Self::Numbers(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b.iter())
                        .all(|(x, y)| (x - y).abs() <= 1e-6 * x.abs().max(y.abs()).max(1.0))
            }
            _ => false,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Numbers(n) => format!("{n:?}"),
        }
    }
}

/// Read the tags in [`COMPARED`] from a TIFF-based raw file.
///
/// Returns whatever was found; a malformed or unexpected file yields an empty
/// map rather than an error, because this is a supplementary check and must
/// never fail a conversion on its own.
pub fn read_lens_tags(path: &Path) -> BTreeMap<u16, TagValue> {
    let Ok(bytes) = std::fs::read(path) else {
        return BTreeMap::new();
    };
    read_lens_tags_from(&bytes)
}

pub fn read_lens_tags_from(bytes: &[u8]) -> BTreeMap<u16, TagValue> {
    let mut found = BTreeMap::new();

    let Some(little) = endianness(bytes) else {
        return found;
    };
    let Some(ifd0) = read_u32(bytes, 4, little) else {
        return found;
    };

    collect(bytes, ifd0 as usize, little, &mut found);

    // Lens tags normally live in the EXIF sub-IFD, pointed at by tag 0x8769.
    if let Some(exif_off) = pointer(bytes, ifd0 as usize, little, 0x8769) {
        collect(bytes, exif_off as usize, little, &mut found);
    }

    found
}

fn endianness(b: &[u8]) -> Option<bool> {
    match b.get(..4)? {
        [0x49, 0x49, 0x2A, 0x00] => Some(true),
        [0x4D, 0x4D, 0x00, 0x2A] => Some(false),
        _ => None,
    }
}

fn read_u16(b: &[u8], at: usize, little: bool) -> Option<u16> {
    let s: [u8; 2] = b.get(at..at + 2)?.try_into().ok()?;
    Some(if little {
        u16::from_le_bytes(s)
    } else {
        u16::from_be_bytes(s)
    })
}

fn read_u32(b: &[u8], at: usize, little: bool) -> Option<u32> {
    let s: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
    Some(if little {
        u32::from_le_bytes(s)
    } else {
        u32::from_be_bytes(s)
    })
}

/// Entry layout: tag, type, count, then value or offset.
fn entries(b: &[u8], ifd: usize, little: bool) -> impl Iterator<Item = (u16, u16, u32, usize)> + '_ {
    let count = read_u16(b, ifd, little).unwrap_or(0) as usize;
    (0..count).filter_map(move |i| {
        let at = ifd + 2 + i * 12;
        let tag = read_u16(b, at, little)?;
        let typ = read_u16(b, at + 2, little)?;
        let cnt = read_u32(b, at + 4, little)?;
        Some((tag, typ, cnt, at + 8))
    })
}

fn pointer(b: &[u8], ifd: usize, little: bool, want: u16) -> Option<u32> {
    entries(b, ifd, little)
        .find(|(tag, ..)| *tag == want)
        .and_then(|(_, _, _, at)| read_u32(b, at, little))
}

fn collect(b: &[u8], ifd: usize, little: bool, out: &mut BTreeMap<u16, TagValue>) {
    for (tag, typ, cnt, value_at) in entries(b, ifd, little) {
        if !COMPARED.iter().any(|(t, _)| *t == tag) {
            continue;
        }

        let size = match typ {
            2 => cnt as usize,      // ASCII
            5 | 10 => cnt as usize * 8, // RATIONAL / SRATIONAL
            _ => continue,
        };

        // Values of four bytes or fewer are stored inline.
        let start = if size > 4 {
            match read_u32(b, value_at, little) {
                Some(o) => o as usize,
                None => continue,
            }
        } else {
            value_at
        };
        let Some(raw) = b.get(start..start + size) else {
            continue;
        };

        let value = match typ {
            2 => TagValue::Text(
                String::from_utf8_lossy(raw)
                    .trim_end_matches('\0')
                    .to_string(),
            ),
            5 => TagValue::Numbers(
                (0..cnt as usize)
                    .filter_map(|i| {
                        let n = read_u32(raw, i * 8, little)? as f64;
                        let d = read_u32(raw, i * 8 + 4, little)? as f64;
                        Some(if d == 0.0 { 0.0 } else { n / d })
                    })
                    .collect(),
            ),
            10 => TagValue::Numbers(
                (0..cnt as usize)
                    .filter_map(|i| {
                        let n = read_u32(raw, i * 8, little)? as i32 as f64;
                        let d = read_u32(raw, i * 8 + 4, little)? as i32 as f64;
                        Some(if d == 0.0 { 0.0 } else { n / d })
                    })
                    .collect(),
            ),
            _ => continue,
        };

        out.insert(tag, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dng_case_normalisation_is_not_a_difference() {
        // Measured: the NEF says NIKON, the DNG says Nikon.
        let nef = TagValue::Text("NIKON".to_string());
        let dng = TagValue::Text("Nikon".to_string());

        assert!(nef.equivalent(&dng));
    }

    #[test]
    fn trailing_padding_is_not_a_difference() {
        let a = TagValue::Text("Nikon ".to_string());
        let b = TagValue::Text("Nikon".to_string());

        assert!(a.equivalent(&b));
    }

    #[test]
    fn a_genuinely_different_lens_is_a_difference() {
        let a = TagValue::Text("NIKKOR Z 24-200mm f/4-6.3 VR".to_string());
        let b = TagValue::Text("NIKKOR Z 50mm f/1.8 S".to_string());

        assert!(!a.equivalent(&b));
    }

    #[test]
    fn rationals_are_compared_by_value_not_representation() {
        // Measured: the NEF stores 240/10, the DNG stores 24/1.
        let nef = TagValue::Numbers(vec![24.0, 200.0, 4.0, 6.3]);
        let dng = TagValue::Numbers(vec![24.0, 200.0, 4.0, 6.3]);
        assert!(nef.equivalent(&dng));

        let different = TagValue::Numbers(vec![24.0, 70.0, 4.0, 6.3]);
        assert!(!nef.equivalent(&different));
    }

    #[test]
    fn text_and_numbers_never_match_each_other() {
        let a = TagValue::Text("24".to_string());
        let b = TagValue::Numbers(vec![24.0]);

        assert!(!a.equivalent(&b));
    }

    #[test]
    fn a_file_that_is_not_tiff_yields_nothing_rather_than_failing() {
        assert!(read_lens_tags_from(b"not a tiff at all").is_empty());
        assert!(read_lens_tags_from(&[]).is_empty());
    }

    #[test]
    fn a_truncated_ifd_does_not_panic() {
        // Valid header claiming an IFD far beyond the end of the buffer.
        let mut b = vec![0x49, 0x49, 0x2A, 0x00];
        b.extend_from_slice(&9999u32.to_le_bytes());
        assert!(read_lens_tags_from(&b).is_empty());
    }
}
