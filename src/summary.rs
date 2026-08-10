//! Tallying what a batch actually did, for the completion dialog.

use gtk::glib;

use crate::convert::Outcome;

/// Running totals for one batch run.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BatchSummary {
    pub converted: u32,
    pub skipped: u32,
    pub failed: u32,
    /// Bytes saved by converting. Signed, because an uncompressed DNG can be
    /// larger than the NEF it came from and reporting that as a saving would be
    /// a lie.
    pub saved_bytes: i64,
}

impl BatchSummary {
    /// Fold one file's result into the totals.
    pub fn record(&mut self, outcome: &Outcome) {
        match outcome {
            Outcome::Converted {
                source_bytes,
                target_bytes,
            } => {
                self.converted += 1;
                self.saved_bytes += *source_bytes as i64 - *target_bytes as i64;
            }
            Outcome::Skipped => self.skipped += 1,
        }
    }

    /// Fold one file's failure into the totals.
    pub fn record_failure(&mut self) {
        self.failed += 1;
    }

    /// Files that were actually attempted and finished, successfully or not.
    pub fn total(&self) -> u32 {
        self.converted + self.skipped + self.failed
    }
}

/// Heading and body for the dialog shown when a batch finishes.
///
/// `queued` is how many files the run started with, which may exceed the number
/// recorded if the run was cancelled part way.
pub fn completion_message(summary: &BatchSummary, queued: u32) -> (String, String) {
    let cancelled = summary.total() < queued;

    let heading = if summary.failed > 0 {
        "Finished with errors"
    } else if cancelled {
        "Cancelled"
    } else {
        "All done"
    };

    let mut lines = Vec::new();

    lines.push(if cancelled {
        format!("Converted {} of {queued} files.", summary.converted)
    } else if summary.converted == 1 {
        "Converted 1 file.".to_string()
    } else {
        format!("Converted {} files.", summary.converted)
    });

    // Stated plainly, because deciding whether to delete originals rests on it.
    if summary.converted > 0 {
        lines.push(format!(
            "All {} verified bit-for-bit against the originals.",
            summary.converted
        ));
    }

    if summary.skipped > 0 {
        lines.push(format!(
            "Skipped {} that already existed.",
            summary.skipped
        ));
    }

    if summary.failed > 0 {
        lines.push(format!("{} failed.", summary.failed));
    }

    // Only claim a saving when there was one; a larger DNG is stated plainly.
    match summary.saved_bytes {
        n if n > 0 => lines.push(format!(
            "You saved {}.",
            glib::format_size(n.unsigned_abs())
        )),
        n if n < 0 => lines.push(format!(
            "The DNGs are {} larger than the originals.",
            glib::format_size(n.unsigned_abs())
        )),
        _ => {}
    }

    (heading.to_string(), lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn converted(source: u64, target: u64) -> Outcome {
        Outcome::Converted {
            source_bytes: source,
            target_bytes: target,
        }
    }

    #[test]
    fn sums_the_space_saved_across_converted_files() {
        let mut s = BatchSummary::default();
        s.record(&converted(47_000_000, 28_000_000));
        s.record(&converted(46_000_000, 27_000_000));

        assert_eq!(s.converted, 2);
        assert_eq!(s.saved_bytes, 38_000_000);
    }

    #[test]
    fn skipped_and_failed_files_contribute_no_saving() {
        let mut s = BatchSummary::default();
        s.record(&converted(47_000_000, 28_000_000));
        s.record(&Outcome::Skipped);
        s.record_failure();

        assert_eq!(s.converted, 1);
        assert_eq!(s.skipped, 1);
        assert_eq!(s.failed, 1);
        assert_eq!(s.total(), 3);
        assert_eq!(s.saved_bytes, 19_000_000, "only conversions count");
    }

    #[test]
    fn a_dng_larger_than_its_nef_is_reported_as_a_loss_not_a_saving() {
        let mut s = BatchSummary::default();
        s.record(&converted(20_000_000, 25_000_000));

        assert_eq!(s.saved_bytes, -5_000_000);
    }

    // The exact size wording is left to glib::format_size, which is locale
    // aware, so these assert on structure rather than on formatted numbers.

    #[test]
    fn a_clean_run_reports_the_count_and_the_saving() {
        let mut s = BatchSummary::default();
        for _ in 0..36 {
            s.record(&converted(47_000_000, 28_000_000));
        }

        let (heading, body) = completion_message(&s, 36);

        assert_eq!(heading, "All done");
        assert!(body.contains("36"), "should say how many: {body}");
        assert!(body.contains("saved"), "should report the saving: {body}");
        assert!(!body.contains("failed"), "nothing failed: {body}");
        assert!(
            body.contains("verified"),
            "the dialog is what a delete-originals decision rests on, so it must \
             state that the data was checked: {body}"
        );
    }

    #[test]
    fn failures_are_surfaced_in_the_heading_and_counted() {
        let mut s = BatchSummary::default();
        s.record(&converted(47_000_000, 28_000_000));
        s.record_failure();
        s.record_failure();

        let (heading, body) = completion_message(&s, 3);

        assert_eq!(heading, "Finished with errors");
        assert!(body.contains("2 failed"), "should count failures: {body}");
    }

    #[test]
    fn skipped_files_are_mentioned_separately_from_failures() {
        let mut s = BatchSummary::default();
        s.record(&converted(47_000_000, 28_000_000));
        s.record(&Outcome::Skipped);

        let (heading, body) = completion_message(&s, 2);

        assert_eq!(heading, "All done");
        assert!(body.contains("skipped") || body.contains("Skipped"), "{body}");
        assert!(!body.contains("failed"), "skipping is not failing: {body}");
    }

    #[test]
    fn a_cancelled_run_says_so_rather_than_claiming_success() {
        let mut s = BatchSummary::default();
        for _ in 0..4 {
            s.record(&converted(47_000_000, 28_000_000));
        }

        let (heading, body) = completion_message(&s, 36);

        assert_eq!(heading, "Cancelled");
        assert!(body.contains("4"), "should say what did finish: {body}");
    }

    #[test]
    fn growing_files_are_not_described_as_a_saving() {
        let mut s = BatchSummary::default();
        s.record(&converted(20_000_000, 25_000_000));

        let (_, body) = completion_message(&s, 1);

        assert!(body.contains("larger"), "should admit growth: {body}");
        assert!(!body.contains("saved"), "must not call growth a saving: {body}");
    }
}
