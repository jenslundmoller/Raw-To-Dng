//! How one file's result is shown in the queue.
//!
//! Kept separate from the widgets so the wording, the icon and the
//! failed/not-failed decision can be tested without a display.

use gtk::glib;

use crate::convert::Outcome;

/// Icon shown at the left of a finished row.
pub const ICON_DONE: &str = "emblem-ok-symbolic";
pub const ICON_FAILED: &str = "dialog-error-symbolic";
pub const ICON_SKIPPED: &str = "action-unavailable-symbolic";

/// Everything the queue needs to render one finished row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowPresentation {
    /// Short text shown on the right of the row.
    pub status: String,
    pub icon: &'static str,
    /// Whether to style the row as an error and include it when filtering.
    pub failed: bool,
    /// Full, untruncated text for the tooltip. `None` when the status already
    /// says everything.
    pub tooltip: Option<String>,
}

/// Describe one file's result.
pub fn present(result: &Result<Outcome, String>) -> RowPresentation {
    match result {
        Ok(Outcome::Converted {
            source_bytes,
            target_bytes,
        }) => RowPresentation {
            // "verified" is the load-bearing word: it says the DNG was read back
            // and proven to hold the same raw data, not merely that writing
            // finished without complaint.
            status: format!(
                "Done · verified · {} → {}",
                glib::format_size(*source_bytes),
                glib::format_size(*target_bytes)
            ),
            icon: ICON_DONE,
            failed: false,
            tooltip: None,
        },
        Ok(Outcome::Skipped) => RowPresentation {
            status: "Already exists".to_string(),
            icon: ICON_SKIPPED,
            failed: false,
            tooltip: Some("A DNG is already at the target path. Enable Overwrite existing to replace it.".to_string()),
        },
        // The row label is ellipsized, so the full text is kept for the tooltip.
        Err(message) => RowPresentation {
            status: message.clone(),
            icon: ICON_FAILED,
            failed: true,
            tooltip: Some(message.clone()),
        },
    }
}

/// How the failures filter button should look for a given queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureIndicator {
    pub visible: bool,
    pub label: String,
    /// Whether the filter may stay switched on. Filtering to failures when
    /// there are none leaves an empty list with a non-empty queue.
    pub filter_allowed: bool,
}

/// Derive the failures button from the queue, rather than tracking it
/// separately and having to remember every place that changes the queue.
pub fn failure_indicator(failed: u32) -> FailureIndicator {
    FailureIndicator {
        visible: failed > 0,
        label: format!("{failed} failed"),
        filter_allowed: failed > 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_failures_means_no_button_and_no_filtering() {
        // The regression: clearing the queue left "2 failed" showing, and the
        // filter still on, so newly added files appeared as an empty list.
        let i = failure_indicator(0);

        assert!(!i.visible, "nothing failed, so nothing to offer");
        assert!(
            !i.filter_allowed,
            "filtering to failures with none would hide a non-empty queue"
        );
    }

    #[test]
    fn failures_are_counted_in_the_button() {
        assert_eq!(failure_indicator(2).label, "2 failed");
        assert!(failure_indicator(2).visible);
        assert!(failure_indicator(2).filter_allowed);
    }

    #[test]
    fn a_single_failure_reads_naturally() {
        assert_eq!(failure_indicator(1).label, "1 failed");
    }

    #[test]
    fn a_converted_file_shows_both_sizes_and_no_error_styling() {
        let p = present(&Ok(Outcome::Converted {
            source_bytes: 47_000_000,
            target_bytes: 28_000_000,
        }));

        assert!(p.status.starts_with("Done"), "{}", p.status);
        assert!(
            p.status.contains("verified"),
            "success must state that the data was checked, not just that it finished: {}",
            p.status
        );
        assert!(p.status.contains('→'), "should show the transition: {}", p.status);
        assert_eq!(p.icon, ICON_DONE);
        assert!(!p.failed);
    }

    #[test]
    fn a_skipped_file_is_visually_distinct_from_a_converted_one() {
        let p = present(&Ok(Outcome::Skipped));

        assert_eq!(p.icon, ICON_SKIPPED);
        assert_ne!(p.icon, ICON_DONE, "a re-run must not look like a conversion");
        assert!(!p.failed, "skipping is not failing");
    }

    #[test]
    fn a_failure_is_marked_failed_and_keeps_the_full_message_for_the_tooltip() {
        let long = "corrupt or unsupported file (subview_padded(): Offset \
                    6272000+42920960 is behind EOF)";
        let p = present(&Err(long.to_string()));

        assert!(p.failed);
        assert_eq!(p.icon, ICON_FAILED);
        assert_eq!(
            p.tooltip.as_deref(),
            Some(long),
            "the untruncated message must survive for the tooltip"
        );
    }

    #[test]
    fn successful_rows_carry_no_tooltip() {
        let p = present(&Ok(Outcome::Converted {
            source_bytes: 10,
            target_bytes: 5,
        }));

        assert_eq!(p.tooltip, None, "nothing to explain on success");
    }
}
