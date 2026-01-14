//! UI state for navigation

use super::super::catalog::{DistroCategory, CATEGORIES};
use super::UiMode;

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
