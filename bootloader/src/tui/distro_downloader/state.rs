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

use super::catalog::{DistroCategory, CATEGORIES};

/// Download status enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStatus {
    /// No download in progress
    Idle,
    /// Checking file existence/size
    Checking,
    /// Download in progress
    Downloading,
    /// Download complete
    Complete,
    /// Download failed
    Failed,
}

impl DownloadStatus {
    /// Get display string for status
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "Ready",
            Self::Checking => "Checking...",
            Self::Downloading => "Downloading...",
            Self::Complete => "Complete",
            Self::Failed => "Failed",
        }
    }

    /// Check if download is active
    pub const fn is_active(&self) -> bool {
        matches!(self, Self::Checking | Self::Downloading)
    }

    /// Check if download is finished (success or failure)
    pub const fn is_finished(&self) -> bool {
        matches!(self, Self::Complete | Self::Failed)
    }
}

/// Download state tracking
#[derive(Debug, Clone)]
pub struct DownloadState {
    /// Current status
    pub status: DownloadStatus,
    /// Current file being downloaded
    pub current_file: Option<&'static str>,
    /// Bytes downloaded so far
    pub bytes_downloaded: usize,
    /// Total bytes expected (if known)
    pub total_bytes: Option<usize>,
    /// Error message if failed
    pub error_message: Option<&'static str>,
    /// Current mirror index being used
    pub mirror_index: usize,
    /// Number of retry attempts
    pub retry_count: usize,
}

impl DownloadState {
    /// Create new idle download state
    pub fn new() -> Self {
        Self {
            status: DownloadStatus::Idle,
            current_file: None,
            bytes_downloaded: 0,
            total_bytes: None,
            error_message: None,
            mirror_index: 0,
            retry_count: 0,
        }
    }

    /// Start checking a file
    pub fn start_check(&mut self, filename: &'static str) {
        morpheus_core::logger::log("DownloadState::start_check()");
        self.status = DownloadStatus::Checking;
        self.current_file = Some(filename);
        self.bytes_downloaded = 0;
        self.total_bytes = None;
        self.error_message = None;
    }

    /// Start downloading after check
    pub fn start_download(&mut self, total: Option<usize>) {
        morpheus_core::logger::log("DownloadState::start_download()");
        self.status = DownloadStatus::Downloading;
        self.total_bytes = total;
        self.bytes_downloaded = 0;
    }

    /// Update download progress
    pub fn update_progress(&mut self, bytes: usize) {
        self.bytes_downloaded = bytes;
    }

    /// Mark download as complete
    pub fn complete(&mut self) {
        morpheus_core::logger::log("DownloadState::complete()");
        self.status = DownloadStatus::Complete;
        if let Some(total) = self.total_bytes {
            self.bytes_downloaded = total;
        }
    }

    /// Mark download as failed
    pub fn fail(&mut self, message: &'static str) {
        morpheus_core::logger::log("DownloadState::fail()");
        self.status = DownloadStatus::Failed;
        self.error_message = Some(message);
    }

    /// Reset to idle state
    pub fn reset(&mut self) {
        morpheus_core::logger::log("DownloadState::reset()");
        self.status = DownloadStatus::Idle;
        self.current_file = None;
        self.bytes_downloaded = 0;
        self.total_bytes = None;
        self.error_message = None;
        self.mirror_index = 0;
        self.retry_count = 0;
    }

    /// Try next mirror
    pub fn try_next_mirror(&mut self, max_mirrors: usize) -> bool {
        if self.mirror_index + 1 < max_mirrors {
            self.mirror_index += 1;
            self.retry_count += 1;
            self.status = DownloadStatus::Checking;
            self.error_message = None;
            morpheus_core::logger::log("DownloadState::try_next_mirror() - switching mirror");
            true
        } else {
            false
        }
    }

    /// Get progress percentage (0-100)
    pub fn progress_percent(&self) -> usize {
        match self.total_bytes {
            Some(total) if total > 0 => {
                let percent = (self.bytes_downloaded * 100) / total;
                percent.min(100)
            }
            _ => 0,
        }
    }

    /// Get bytes remaining
    pub fn bytes_remaining(&self) -> Option<usize> {
        self.total_bytes
            .map(|t| t.saturating_sub(self.bytes_downloaded))
    }

    /// Format progress as string (e.g., "150 MB / 500 MB")
    pub fn progress_string(&self) -> (&'static str, &'static str) {
        // Returns static strings for simplicity in no_std
        let downloaded = Self::size_bucket(self.bytes_downloaded);
        let total = self.total_bytes.map(Self::size_bucket).unwrap_or("???");
        (downloaded, total)
    }

    /// Bucket size into human readable string
    fn size_bucket(bytes: usize) -> &'static str {
        if bytes < 1024 * 1024 {
            "< 1 MB"
        } else if bytes < 10 * 1024 * 1024 {
            "1-10 MB"
        } else if bytes < 50 * 1024 * 1024 {
            "10-50 MB"
        } else if bytes < 100 * 1024 * 1024 {
            "50-100 MB"
        } else if bytes < 250 * 1024 * 1024 {
            "100-250 MB"
        } else if bytes < 500 * 1024 * 1024 {
            "250-500 MB"
        } else if bytes < 1024 * 1024 * 1024 {
            "500 MB - 1 GB"
        } else if bytes < 2 * 1024 * 1024 * 1024 {
            "1-2 GB"
        } else {
            "> 2 GB"
        }
    }
}

impl Default for DownloadState {
    fn default() -> Self {
        Self::new()
    }
}

/// UI mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiMode {
    /// Browsing distro list for download
    Browse,
    /// Showing confirmation dialog
    Confirm,
    /// Download in progress
    Downloading,
    /// Showing result (success/error)
    Result,
    /// Managing downloaded ISOs
    Manage,
    /// Confirm delete ISO
    ConfirmDelete,
}

impl UiMode {
    /// Get display string
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Browse => "Browse",
            Self::Confirm => "Confirm",
            Self::Downloading => "Downloading",
            Self::Result => "Result",
            Self::Manage => "Manage",
            Self::ConfirmDelete => "Confirm Delete",
        }
    }

    /// Check if in management submenu
    pub const fn is_manage_related(&self) -> bool {
        matches!(self, Self::Manage | Self::ConfirmDelete)
    }
}

/// UI state for navigation
#[derive(Debug, Clone)]
pub struct UiState {
    /// Currently selected category index
    pub selected_category: usize,
    /// Currently selected distro index within category
    pub selected_distro: usize,
    /// Scroll offset for distro list
    pub scroll_offset: usize,
    /// Current UI mode
    pub mode: UiMode,
    /// Status message to display
    pub status_message: Option<&'static str>,
    /// Selected ISO index in manage view
    pub selected_iso: usize,
    /// Total ISO count (cached from storage)
    pub iso_count: usize,
}

impl UiState {
    /// Maximum visible items in distro list
    pub const VISIBLE_ITEMS: usize = 8;

    /// Create new UI state
    pub fn new() -> Self {
        morpheus_core::logger::log("UiState::new()");
        Self {
            selected_category: 0,
            selected_distro: 0,
            scroll_offset: 0,
            mode: UiMode::Browse,
            status_message: None,
            selected_iso: 0,
            iso_count: 0,
        }
    }

    /// Move to next category
    pub fn next_category(&mut self, num_categories: usize) {
        if self.selected_category + 1 < num_categories {
            self.selected_category += 1;
            self.selected_distro = 0;
            self.scroll_offset = 0;
            morpheus_core::logger::log("UiState::next_category()");
        }
    }

    /// Move to previous category
    pub fn prev_category(&mut self) {
        if self.selected_category > 0 {
            self.selected_category -= 1;
            self.selected_distro = 0;
            self.scroll_offset = 0;
            morpheus_core::logger::log("UiState::prev_category()");
        }
    }

    /// Move to next distro in list
    pub fn next_distro(&mut self, num_distros: usize) {
        if num_distros == 0 {
            return;
        }
        if self.selected_distro + 1 < num_distros {
            self.selected_distro += 1;
            // Scroll if needed
            if self.selected_distro >= self.scroll_offset + Self::VISIBLE_ITEMS {
                self.scroll_offset = self.selected_distro - Self::VISIBLE_ITEMS + 1;
            }
        }
    }

    /// Move to previous distro in list
    pub fn prev_distro(&mut self) {
        if self.selected_distro > 0 {
            self.selected_distro -= 1;
            // Scroll if needed
            if self.selected_distro < self.scroll_offset {
                self.scroll_offset = self.selected_distro;
            }
        }
    }

    /// Start download mode
    pub fn start_download(&mut self) {
        morpheus_core::logger::log("UiState::start_download()");
        self.mode = UiMode::Downloading;
        self.status_message = Some("Starting download...");
    }

    /// Show confirmation dialog
    pub fn show_confirm(&mut self) {
        morpheus_core::logger::log("UiState::show_confirm()");
        self.mode = UiMode::Confirm;
    }

    /// Return to browse mode
    pub fn return_to_browse(&mut self) {
        morpheus_core::logger::log("UiState::return_to_browse()");
        self.mode = UiMode::Browse;
        self.status_message = None;
    }

    /// Show result mode
    pub fn show_result(&mut self, message: &'static str) {
        morpheus_core::logger::log("UiState::show_result()");
        self.mode = UiMode::Result;
        self.status_message = Some(message);
    }

    /// Get current category
    pub fn current_category(&self) -> DistroCategory {
        CATEGORIES[self.selected_category.min(CATEGORIES.len() - 1)]
    }

    /// Set status message
    pub fn set_status(&mut self, message: &'static str) {
        self.status_message = Some(message);
    }

    /// Clear status message
    pub fn clear_status(&mut self) {
        self.status_message = None;
    }

    /// Check if in browsable state
    pub fn is_browsing(&self) -> bool {
        matches!(self.mode, UiMode::Browse)
    }

    /// Check if showing confirmation
    pub fn is_confirming(&self) -> bool {
        matches!(self.mode, UiMode::Confirm)
    }

    /// Check if downloading
    pub fn is_downloading(&self) -> bool {
        matches!(self.mode, UiMode::Downloading)
    }

    // --- ISO Management ---

    /// Switch to ISO management view
    pub fn show_manage(&mut self) {
        morpheus_core::logger::log("UiState::show_manage()");
        self.mode = UiMode::Manage;
        self.selected_iso = 0;
    }

    /// Return to browse from manage
    pub fn return_from_manage(&mut self) {
        morpheus_core::logger::log("UiState::return_from_manage()");
        self.mode = UiMode::Browse;
    }

    /// Update ISO count (call after storage changes)
    pub fn update_iso_count(&mut self, count: usize) {
        self.iso_count = count;
        if self.selected_iso >= count && count > 0 {
            self.selected_iso = count - 1;
        }
    }

    /// Move to next ISO in manage list
    pub fn next_iso(&mut self) {
        if self.iso_count > 0 && self.selected_iso + 1 < self.iso_count {
            self.selected_iso += 1;
        }
    }

    /// Move to previous ISO in manage list
    pub fn prev_iso(&mut self) {
        if self.selected_iso > 0 {
            self.selected_iso -= 1;
        }
    }

    /// Show confirm delete dialog
    pub fn show_confirm_delete(&mut self) {
        if self.iso_count > 0 {
            self.mode = UiMode::ConfirmDelete;
        }
    }

    /// Cancel confirmation and return to manage
    pub fn cancel_confirm(&mut self) {
        self.mode = UiMode::Manage;
    }

    /// Check if in manage mode
    pub fn is_managing(&self) -> bool {
        self.mode.is_manage_related()
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
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
