//! ISO Manager UI
//!
//! Main UI component for the ISO manager.

use super::state::{IsoManagerState, Action, ViewMode};
use super::renderer;
use crate::tui::input::Keyboard;
use crate::tui::renderer::Screen;
use morpheus_core::iso::IsoStorageManager;

/// ISO Manager TUI component
pub struct IsoManager {
    state: IsoManagerState,
    storage: IsoStorageManager,
}

impl IsoManager {
    /// Create a new ISO manager
    ///
    /// # Arguments
    /// * `esp_start_lba` - Start LBA of ESP partition
    /// * `disk_size_lba` - Total disk size in LBAs
    pub fn new(esp_start_lba: u64, disk_size_lba: u64) -> Self {
        let storage = IsoStorageManager::new(esp_start_lba, disk_size_lba);
        let mut state = IsoManagerState::new();
        state.load_from_manager(&storage);

        Self { state, storage }
    }

    /// Create with existing storage manager
    pub fn with_storage(storage: IsoStorageManager) -> Self {
        let mut state = IsoManagerState::new();
        state.load_from_manager(&storage);

        Self { state, storage }
    }

    /// Get reference to storage manager
    pub fn storage(&self) -> &IsoStorageManager {
        &self.storage
    }

    /// Get mutable reference to storage manager
    pub fn storage_mut(&mut self) -> &mut IsoStorageManager {
        &mut self.storage
    }

    /// Reload ISO list from storage
    pub fn refresh(&mut self) {
        self.state.load_from_manager(&self.storage);
        self.state.clear_error();
    }

    /// Run the ISO manager UI
    ///
    /// Returns when user presses ESC or selects boot action.
    /// Returns Some(index) if user wants to boot an ISO.
    pub fn run(&mut self, screen: &mut Screen, keyboard: &mut Keyboard) -> Option<usize> {
        screen.clear();
        renderer::render(screen, &self.state);

        loop {
            if let Some(key) = keyboard.poll_key_with_delay() {
                let action = self.state.handle_key(key.scan_code, key.unicode_char);

                match action {
                    Action::None => {
                        // Just re-render
                        renderer::render(screen, &self.state);
                    }
                    Action::Back => {
                        return None;
                    }
                    Action::Boot(idx) => {
                        return Some(idx);
                    }
                    Action::Delete(idx) => {
                        self.handle_delete(idx);
                        screen.clear();
                        renderer::render(screen, &self.state);
                    }
                    Action::Refresh => {
                        self.refresh();
                        screen.clear();
                        renderer::render(screen, &self.state);
                    }
                }
            }
        }
    }

    /// Handle delete action
    fn handle_delete(&mut self, idx: usize) {
        match self.storage.remove_entry(idx) {
            Ok(()) => {
                self.state.load_from_manager(&self.storage);
                self.state.clear_error();
            }
            Err(_) => {
                self.state.set_error("Failed to delete ISO");
            }
        }
    }

    /// Get read context for booting an ISO
    pub fn get_boot_context(
        &self,
        idx: usize,
    ) -> Result<morpheus_core::iso::IsoReadContext, morpheus_core::iso::IsoError> {
        self.storage.get_read_context(idx)
    }

    /// Check if an ISO is ready to boot
    pub fn is_bootable(&self, idx: usize) -> bool {
        if let Some(entry) = self.storage.get(idx) {
            entry.manifest.is_complete()
        } else {
            false
        }
    }

    /// Get ISO count
    pub fn count(&self) -> usize {
        self.storage.count()
    }

    /// Get ISO name by index
    pub fn get_name(&self, idx: usize) -> Option<&str> {
        self.storage.get(idx).map(|e| e.manifest.name_str())
    }
}
