//! ISO Manager State
//!
//! State management for the ISO manager TUI.

use morpheus_core::iso::{IsoEntry, IsoStorageManager, MAX_ISOS};

/// View mode for the ISO manager
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    /// List view showing all ISOs
    List,
    /// Detail view for selected ISO
    Details,
    /// Confirm delete dialog
    ConfirmDelete,
    /// Confirm boot dialog
    ConfirmBoot,
}

/// Action result from user input
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// No action, continue
    None,
    /// Return to previous screen
    Back,
    /// Boot the selected ISO
    Boot(usize),
    /// Delete the selected ISO
    Delete(usize),
    /// Refresh the ISO list
    Refresh,
}

/// ISO manager state
pub struct IsoManagerState {
    /// Current view mode
    pub mode: ViewMode,
    /// Selected ISO index
    pub selected: usize,
    /// Total number of ISOs
    pub count: usize,
    /// Cached ISO names for display (avoid repeated string ops)
    pub names: [[u8; 64]; MAX_ISOS],
    /// Name lengths
    pub name_lens: [usize; MAX_ISOS],
    /// Cached sizes (in MB)
    pub sizes_mb: [u64; MAX_ISOS],
    /// Cached chunk counts
    pub chunk_counts: [usize; MAX_ISOS],
    /// Cached completion status
    pub complete: [bool; MAX_ISOS],
    /// Error message to display (if any)
    pub error_msg: Option<&'static str>,
}

impl IsoManagerState {
    /// Create new state
    pub fn new() -> Self {
        Self {
            mode: ViewMode::List,
            selected: 0,
            count: 0,
            names: [[0u8; 64]; MAX_ISOS],
            name_lens: [0; MAX_ISOS],
            sizes_mb: [0; MAX_ISOS],
            chunk_counts: [0; MAX_ISOS],
            complete: [false; MAX_ISOS],
            error_msg: None,
        }
    }

    /// Load ISO data from storage manager
    pub fn load_from_manager(&mut self, manager: &IsoStorageManager) {
        self.count = manager.count();
        self.selected = self.selected.min(self.count.saturating_sub(1));

        for (i, (_, entry)) in manager.iter().enumerate() {
            if i >= MAX_ISOS {
                break;
            }

            // Copy name
            let manifest = &entry.manifest;
            let name_len = manifest.name_len.min(64);
            self.names[i][..name_len].copy_from_slice(&manifest.name[..name_len]);
            self.name_lens[i] = name_len;

            // Size in MB
            self.sizes_mb[i] = manifest.total_size / (1024 * 1024);

            // Chunk count
            self.chunk_counts[i] = manifest.chunks.count;

            // Completion status
            self.complete[i] = manifest.is_complete();
        }
    }

    /// Get selected ISO name as str
    pub fn selected_name(&self) -> &str {
        if self.selected < self.count {
            core::str::from_utf8(&self.names[self.selected][..self.name_lens[self.selected]])
                .unwrap_or("???")
        } else {
            ""
        }
    }

    /// Get selected ISO size in MB
    pub fn selected_size_mb(&self) -> u64 {
        if self.selected < self.count {
            self.sizes_mb[self.selected]
        } else {
            0
        }
    }

    /// Get selected ISO chunk count
    pub fn selected_chunks(&self) -> usize {
        if self.selected < self.count {
            self.chunk_counts[self.selected]
        } else {
            0
        }
    }

    /// Check if selected ISO is complete
    pub fn selected_complete(&self) -> bool {
        if self.selected < self.count {
            self.complete[self.selected]
        } else {
            false
        }
    }

    /// Move selection up
    pub fn select_prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if self.count > 0 && self.selected < self.count - 1 {
            self.selected += 1;
        }
    }

    /// Handle key input, return action
    pub fn handle_key(&mut self, scan_code: u16, unicode: u16) -> Action {
        match self.mode {
            ViewMode::List => self.handle_list_key(scan_code, unicode),
            ViewMode::Details => self.handle_details_key(scan_code, unicode),
            ViewMode::ConfirmDelete => self.handle_confirm_delete_key(scan_code, unicode),
            ViewMode::ConfirmBoot => self.handle_confirm_boot_key(scan_code, unicode),
        }
    }

    fn handle_list_key(&mut self, scan_code: u16, unicode: u16) -> Action {
        // ESC - return to main menu
        if scan_code == 0x17 {
            return Action::Back;
        }

        // Up arrow
        if scan_code == 0x01 {
            self.select_prev();
            return Action::None;
        }

        // Down arrow
        if scan_code == 0x02 {
            self.select_next();
            return Action::None;
        }

        // Enter - show details
        if unicode == 0x0D && self.count > 0 {
            self.mode = ViewMode::Details;
            return Action::None;
        }

        // 'd' or 'D' - delete
        if (unicode == 0x64 || unicode == 0x44) && self.count > 0 {
            self.mode = ViewMode::ConfirmDelete;
            return Action::None;
        }

        // 'b' or 'B' - boot
        if (unicode == 0x62 || unicode == 0x42) && self.count > 0 && self.selected_complete() {
            self.mode = ViewMode::ConfirmBoot;
            return Action::None;
        }

        // 'r' or 'R' - refresh
        if unicode == 0x72 || unicode == 0x52 {
            return Action::Refresh;
        }

        Action::None
    }

    fn handle_details_key(&mut self, scan_code: u16, unicode: u16) -> Action {
        // ESC or Backspace - back to list
        if scan_code == 0x17 || unicode == 0x08 {
            self.mode = ViewMode::List;
            return Action::None;
        }

        // 'b' or 'B' - boot from details
        if (unicode == 0x62 || unicode == 0x42) && self.selected_complete() {
            self.mode = ViewMode::ConfirmBoot;
            return Action::None;
        }

        // 'd' or 'D' - delete from details
        if unicode == 0x64 || unicode == 0x44 {
            self.mode = ViewMode::ConfirmDelete;
            return Action::None;
        }

        Action::None
    }

    fn handle_confirm_delete_key(&mut self, _scan_code: u16, unicode: u16) -> Action {
        // 'y' or 'Y' - confirm delete
        if unicode == 0x79 || unicode == 0x59 {
            let idx = self.selected;
            self.mode = ViewMode::List;
            return Action::Delete(idx);
        }

        // 'n' or 'N' or ESC - cancel
        if unicode == 0x6E || unicode == 0x4E || unicode == 0x1B {
            self.mode = ViewMode::List;
            return Action::None;
        }

        Action::None
    }

    fn handle_confirm_boot_key(&mut self, _scan_code: u16, unicode: u16) -> Action {
        // 'y' or 'Y' - confirm boot
        if unicode == 0x79 || unicode == 0x59 {
            let idx = self.selected;
            self.mode = ViewMode::List;
            return Action::Boot(idx);
        }

        // 'n' or 'N' or ESC - cancel
        if unicode == 0x6E || unicode == 0x4E || unicode == 0x1B {
            self.mode = ViewMode::List;
            return Action::None;
        }

        Action::None
    }

    /// Set error message
    pub fn set_error(&mut self, msg: &'static str) {
        self.error_msg = Some(msg);
    }

    /// Clear error message
    pub fn clear_error(&mut self) {
        self.error_msg = None;
    }
}

impl Default for IsoManagerState {
    fn default() -> Self {
        Self::new()
    }
}
