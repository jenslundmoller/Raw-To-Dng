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

#[cfg(test)]
mod tests {
    use super::*;

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
