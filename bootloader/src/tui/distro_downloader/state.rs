//! State management for Distro Downloader
//!
//! Defines state machines for UI navigation and download progress.

use super::catalog::{DistroCategory, CATEGORIES};

/// Download status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStatus {
    /// No download in progress
    Idle,
    /// Checking file size
    Checking,
    /// Download in progress
    Downloading,
    /// Download complete
    Complete,
    /// Download failed
    Failed,
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
        }
    }

    /// Start a new download
    pub fn start(&mut self, filename: &'static str, total: Option<usize>) {
        self.status = DownloadStatus::Downloading;
        self.current_file = Some(filename);
        self.bytes_downloaded = 0;
        self.total_bytes = total;
        self.error_message = None;
    }

    /// Update download progress
    pub fn update_progress(&mut self, bytes: usize) {
        self.bytes_downloaded = bytes;
    }

    /// Mark download as complete
    pub fn complete(&mut self) {
        self.status = DownloadStatus::Complete;
        if let Some(total) = self.total_bytes {
            self.bytes_downloaded = total;
        }
    }

    /// Mark download as failed
    pub fn fail(&mut self, message: &'static str) {
        self.status = DownloadStatus::Failed;
        self.error_message = Some(message);
    }

    /// Reset to idle state
    pub fn reset(&mut self) {
        self.status = DownloadStatus::Idle;
        self.current_file = None;
        self.bytes_downloaded = 0;
        self.total_bytes = None;
        self.error_message = None;
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
}

impl Default for DownloadState {
    fn default() -> Self {
        Self::new()
    }
}

/// UI mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiMode {
    /// Browsing distro list
    Browse,
    /// Showing confirmation dialog
    Confirm,
    /// Download in progress
    Downloading,
    /// Showing result (success/error)
    Result,
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
}

impl UiState {
    /// Create new UI state
    pub fn new() -> Self {
        Self {
            selected_category: 0,
            selected_distro: 0,
            scroll_offset: 0,
            mode: UiMode::Browse,
        }
    }

    /// Move to next category
    pub fn next_category(&mut self, num_categories: usize) {
        if self.selected_category + 1 < num_categories {
            self.selected_category += 1;
            // Reset selection when changing category
            self.selected_distro = 0;
            self.scroll_offset = 0;
        }
    }

    /// Move to previous category
    pub fn prev_category(&mut self) {
        if self.selected_category > 0 {
            self.selected_category -= 1;
            // Reset selection when changing category
            self.selected_distro = 0;
            self.scroll_offset = 0;
        }
    }

    /// Move to next distro in list
    pub fn next_distro(&mut self, num_distros: usize, visible_items: usize) {
        if num_distros == 0 {
            return;
        }
        if self.selected_distro + 1 < num_distros {
            self.selected_distro += 1;
            // Scroll if needed
            if self.selected_distro >= self.scroll_offset + visible_items {
                self.scroll_offset = self.selected_distro - visible_items + 1;
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
        self.mode = UiMode::Downloading;
    }

    /// Show confirmation dialog
    pub fn show_confirm(&mut self) {
        self.mode = UiMode::Confirm;
    }

    /// Return to browse mode
    pub fn return_to_browse(&mut self) {
        self.mode = UiMode::Browse;
    }

    /// Show result mode
    pub fn show_result(&mut self) {
        self.mode = UiMode::Result;
    }

    /// Get current category
    pub fn current_category(&self) -> DistroCategory {
        CATEGORIES[self.selected_category]
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_download_state_progress_boundary() {
        let mut state = DownloadState::new();
        state.start("test.iso", Some(100));
        
        // Test 0%
        state.update_progress(0);
        assert_eq!(state.progress_percent(), 0);
        
        // Test 100%
        state.update_progress(100);
        assert_eq!(state.progress_percent(), 100);
        
        // Test over 100% (should cap)
        state.update_progress(150);
        assert_eq!(state.progress_percent(), 100);
    }

    #[test]
    fn test_ui_state_scroll_up() {
        let mut state = UiState::new();
        let num_distros = 10;
        let visible = 5;
        
        // Scroll down first
        for _ in 0..7 {
            state.next_distro(num_distros, visible);
        }
        assert_eq!(state.selected_distro, 7);
        assert!(state.scroll_offset > 0);
        
        // Scroll back up
        for _ in 0..7 {
            state.prev_distro();
        }
        assert_eq!(state.selected_distro, 0);
        assert_eq!(state.scroll_offset, 0);
    }
}
