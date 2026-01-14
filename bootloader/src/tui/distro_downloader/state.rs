//! State management for Distro Downloader
//!
//! Defines state machines for UI navigation and download progress.
//! Pure Rust with no UEFI dependencies - fully unit testable.
//!
//! # State Machines
//!
//! ## UI State Machine
//! ```text
//! ┌─────────┐  select  ┌─────────┐  confirm  ┌─────────────┐
//! │ Browse  │─────────▶│ Confirm │──────────▶│ Downloading │
//! └─────────┘          └─────────┘           └─────────────┘
//!      ▲                    │                      │
//!      │     cancel         │                      │ complete/fail
//!      └────────────────────┴──────────────────────┘
//! ```
//!
//! ## Download State Machine
//! ```text
//! ┌──────┐  start  ┌─────────────┐  done   ┌──────────┐
//! │ Idle │────────▶│ Downloading │────────▶│ Complete │
//! └──────┘         └─────────────┘         └──────────┘
//!                        │
//!                        │ error
//!                        ▼
//!                   ┌────────┐
//!                   │ Failed │
//!                   └────────┘
//! ```

mod download_state;
mod download_status;
mod ui_mode;
mod ui_state;

pub use download_state::DownloadState;
pub use download_status::DownloadStatus;
pub use ui_mode::UiMode;
pub use ui_state::UiState;

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::super::catalog::CATEGORIES;
    use super::*;

    // --- DownloadStatus Tests ---

    #[test]
    fn test_status_as_str() {
        assert_eq!(DownloadStatus::Idle.as_str(), "Ready");
        assert_eq!(DownloadStatus::Checking.as_str(), "Checking...");
        assert_eq!(DownloadStatus::Downloading.as_str(), "Downloading...");
        assert_eq!(DownloadStatus::Complete.as_str(), "Complete");
        assert_eq!(DownloadStatus::Failed.as_str(), "Failed");
    }

    #[test]
    fn test_status_is_active() {
        assert!(!DownloadStatus::Idle.is_active());
        assert!(DownloadStatus::Checking.is_active());
        assert!(DownloadStatus::Downloading.is_active());
        assert!(!DownloadStatus::Complete.is_active());
        assert!(!DownloadStatus::Failed.is_active());
    }

    #[test]
    fn test_status_is_finished() {
        assert!(!DownloadStatus::Idle.is_finished());
        assert!(!DownloadStatus::Checking.is_finished());
        assert!(!DownloadStatus::Downloading.is_finished());
        assert!(DownloadStatus::Complete.is_finished());
        assert!(DownloadStatus::Failed.is_finished());
    }

    // --- DownloadState Tests ---

    #[test]
    fn test_download_state_new() {
        let state = DownloadState::new();
        assert_eq!(state.status, DownloadStatus::Idle);
        assert!(state.current_file.is_none());
        assert_eq!(state.bytes_downloaded, 0);
        assert!(state.total_bytes.is_none());
        assert!(state.error_message.is_none());
        assert_eq!(state.mirror_index, 0);
        assert_eq!(state.retry_count, 0);
    }

    #[test]
    fn test_download_state_start_check() {
        let mut state = DownloadState::new();
        state.start_check("test.iso");

        assert_eq!(state.status, DownloadStatus::Checking);
        assert_eq!(state.current_file, Some("test.iso"));
        assert_eq!(state.bytes_downloaded, 0);
    }

    #[test]
    fn test_download_state_start_download() {
        let mut state = DownloadState::new();
        state.start_check("test.iso");
        state.start_download(Some(1_000_000));

        assert_eq!(state.status, DownloadStatus::Downloading);
        assert_eq!(state.total_bytes, Some(1_000_000));
    }

    #[test]
    fn test_download_state_progress() {
        let mut state = DownloadState::new();
        state.start_check("test.iso");
        state.start_download(Some(1_000_000));
        state.update_progress(500_000);

        assert_eq!(state.bytes_downloaded, 500_000);
        assert_eq!(state.progress_percent(), 50);
    }

    #[test]
    fn test_download_state_progress_unknown_total() {
        let mut state = DownloadState::new();
        state.start_check("test.iso");
        state.start_download(None);
        state.update_progress(500_000);

        assert_eq!(state.bytes_downloaded, 500_000);
        assert_eq!(state.progress_percent(), 0); // Unknown = 0%
    }

    #[test]
    fn test_download_state_progress_boundary() {
        let mut state = DownloadState::new();
        state.start_check("test.iso");
        state.start_download(Some(100));

        state.update_progress(0);
        assert_eq!(state.progress_percent(), 0);

        state.update_progress(100);
        assert_eq!(state.progress_percent(), 100);

        state.update_progress(150);
        assert_eq!(state.progress_percent(), 100); // Capped at 100
    }

    #[test]
    fn test_download_state_complete() {
        let mut state = DownloadState::new();
        state.start_check("test.iso");
        state.start_download(Some(1_000_000));
        state.update_progress(1_000_000);
        state.complete();

        assert_eq!(state.status, DownloadStatus::Complete);
    }

    #[test]
    fn test_download_state_fail() {
        let mut state = DownloadState::new();
        state.start_check("test.iso");
        state.start_download(Some(1_000_000));
        state.fail("Network error");

        assert_eq!(state.status, DownloadStatus::Failed);
        assert_eq!(state.error_message, Some("Network error"));
    }

    #[test]
    fn test_download_state_reset() {
        let mut state = DownloadState::new();
        state.start_check("test.iso");
        state.start_download(Some(1_000_000));
        state.update_progress(500_000);
        state.mirror_index = 2;
        state.retry_count = 3;
        state.reset();

        assert_eq!(state.status, DownloadStatus::Idle);
        assert!(state.current_file.is_none());
        assert_eq!(state.bytes_downloaded, 0);
        assert_eq!(state.mirror_index, 0);
        assert_eq!(state.retry_count, 0);
    }

    #[test]
    fn test_download_state_try_next_mirror() {
        let mut state = DownloadState::new();
        state.start_check("test.iso");
        state.fail("Mirror 1 failed");

        assert!(state.try_next_mirror(3)); // Has more mirrors
        assert_eq!(state.mirror_index, 1);
        assert_eq!(state.status, DownloadStatus::Checking);
        assert!(state.error_message.is_none());

        assert!(state.try_next_mirror(3)); // Has more mirrors
        assert_eq!(state.mirror_index, 2);

        assert!(!state.try_next_mirror(3)); // No more mirrors
        assert_eq!(state.mirror_index, 2);
    }

    #[test]
    fn test_download_state_bytes_remaining() {
        let mut state = DownloadState::new();
        state.start_download(Some(1000));
        state.update_progress(300);

        assert_eq!(state.bytes_remaining(), Some(700));
    }

    #[test]
    fn test_download_state_bytes_remaining_none() {
        let mut state = DownloadState::new();
        state.start_download(None);
        state.update_progress(300);

        assert_eq!(state.bytes_remaining(), None);
    }

    // --- UiMode Tests ---

    #[test]
    fn test_ui_mode_as_str() {
        assert_eq!(UiMode::Browse.as_str(), "Browse");
        assert_eq!(UiMode::Confirm.as_str(), "Confirm");
        assert_eq!(UiMode::Downloading.as_str(), "Downloading");
        assert_eq!(UiMode::Result.as_str(), "Result");
    }

    // --- UiState Tests ---

    #[test]
    fn test_ui_state_new() {
        let state = UiState::new();
        assert_eq!(state.selected_category, 0);
        assert_eq!(state.selected_distro, 0);
        assert_eq!(state.scroll_offset, 0);
        assert_eq!(state.mode, UiMode::Browse);
        assert!(state.status_message.is_none());
    }

    #[test]
    fn test_ui_state_next_category() {
        let mut state = UiState::new();
        let num_cats = CATEGORIES.len();

        state.selected_distro = 5;
        state.scroll_offset = 2;

        state.next_category(num_cats);
        assert_eq!(state.selected_category, 1);
        assert_eq!(state.selected_distro, 0); // Reset
        assert_eq!(state.scroll_offset, 0); // Reset
    }

    #[test]
    fn test_ui_state_next_category_boundary() {
        let mut state = UiState::new();
        let num_cats = CATEGORIES.len();

        // Go to last category
        for _ in 0..num_cats {
            state.next_category(num_cats);
        }

        assert_eq!(state.selected_category, num_cats - 1); // Stays at last
    }

    #[test]
    fn test_ui_state_prev_category() {
        let mut state = UiState::new();
        state.selected_category = 2;
        state.selected_distro = 3;

        state.prev_category();
        assert_eq!(state.selected_category, 1);
        assert_eq!(state.selected_distro, 0); // Reset
    }

    #[test]
    fn test_ui_state_prev_category_at_zero() {
        let mut state = UiState::new();
        state.prev_category();
        assert_eq!(state.selected_category, 0); // Stays at 0
    }

    #[test]
    fn test_ui_state_next_distro() {
        let mut state = UiState::new();
        state.next_distro(10);
        assert_eq!(state.selected_distro, 1);
    }

    #[test]
    fn test_ui_state_next_distro_scrolls() {
        let mut state = UiState::new();
        let visible = UiState::VISIBLE_ITEMS;

        // Move past visible items
        for _ in 0..visible + 2 {
            state.next_distro(20);
        }

        assert!(state.scroll_offset > 0);
    }

    #[test]
    fn test_ui_state_next_distro_boundary() {
        let mut state = UiState::new();
        for _ in 0..15 {
            state.next_distro(10);
        }
        assert_eq!(state.selected_distro, 9); // Capped at max
    }

    #[test]
    fn test_ui_state_next_distro_empty() {
        let mut state = UiState::new();
        state.next_distro(0);
        assert_eq!(state.selected_distro, 0); // No change
    }

    #[test]
    fn test_ui_state_prev_distro() {
        let mut state = UiState::new();
        state.selected_distro = 5;
        state.prev_distro();
        assert_eq!(state.selected_distro, 4);
    }

    #[test]
    fn test_ui_state_prev_distro_at_zero() {
        let mut state = UiState::new();
        state.prev_distro();
        assert_eq!(state.selected_distro, 0);
    }

    #[test]
    fn test_ui_state_prev_distro_scrolls_up() {
        let mut state = UiState::new();
        state.selected_distro = 5;
        state.scroll_offset = 5;

        state.prev_distro();
        assert_eq!(state.selected_distro, 4);
        assert_eq!(state.scroll_offset, 4); // Scrolled up
    }

    #[test]
    fn test_ui_state_mode_transitions() {
        let mut state = UiState::new();
        assert!(state.is_browsing());

        state.show_confirm();
        assert!(state.is_confirming());
        assert_eq!(state.mode, UiMode::Confirm);

        state.start_download();
        assert!(state.is_downloading());
        assert_eq!(state.mode, UiMode::Downloading);

        state.show_result("Done!");
        assert_eq!(state.mode, UiMode::Result);
        assert_eq!(state.status_message, Some("Done!"));

        state.return_to_browse();
        assert!(state.is_browsing());
        assert!(state.status_message.is_none());
    }

    #[test]
    fn test_ui_state_current_category() {
        let mut state = UiState::new();
        assert_eq!(state.current_category(), CATEGORIES[0]);

        state.next_category(CATEGORIES.len());
        assert_eq!(state.current_category(), CATEGORIES[1]);
    }

    #[test]
    fn test_ui_state_status_messages() {
        let mut state = UiState::new();

        state.set_status("Loading...");
        assert_eq!(state.status_message, Some("Loading..."));

        state.clear_status();
        assert!(state.status_message.is_none());
    }
}
