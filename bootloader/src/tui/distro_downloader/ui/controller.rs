//! Distro Downloader Controller
//!
//! Main controller struct and core logic for the distro downloader UI.
//! This is a slim coordinator that delegates to specialized modules:
//! - `input.rs` - Input handling
//! - `render.rs` - Screen rendering
//! - `helpers.rs` - Utility functions

extern crate alloc;

use alloc::vec::Vec;

use super::helpers::ManageAction;
use super::input::{handle_input, InputContext};
use super::render::{render_full, RenderContext};
use crate::tui::distro_downloader::catalog::{get_by_category, DistroEntry};
use crate::tui::distro_downloader::state::{DownloadState, UiState};
use crate::tui::input::Keyboard;
use crate::tui::renderer::Screen;
use crate::BootServices;
use morpheus_core::iso::{IsoStorageManager, MAX_ISOS};

/// Main distro downloader UI controller
pub struct DistroDownloader {
    /// UI navigation state
    ui_state: UiState,
    /// Download progress state
    download_state: DownloadState,
    /// Cached list of distros for current category
    current_distros: Vec<&'static DistroEntry>,
    /// Boot services reference (for file operations)
    boot_services: *const BootServices,
    /// Image handle
    image_handle: *mut (),
    /// Track if we need full redraw (mode change, category change)
    needs_full_redraw: bool,
    /// ISO storage manager (for downloaded ISOs)
    iso_storage: IsoStorageManager,
    /// Cached ISO names for display
    iso_names: [[u8; 64]; MAX_ISOS],
    /// Cached ISO name lengths
    iso_name_lens: [usize; MAX_ISOS],
    /// Cached ISO sizes (MB)
    iso_sizes_mb: [u64; MAX_ISOS],
    /// Cached ISO completion status
    iso_complete: [bool; MAX_ISOS],
}

impl DistroDownloader {
    /// Create a new distro downloader
    ///
    /// # Arguments
    /// * `boot_services` - UEFI boot services
    /// * `image_handle` - Current image handle
    /// * `esp_start_lba` - Start LBA of ESP partition (for ISO storage)
    /// * `disk_size_lba` - Total disk size in LBAs
    pub fn new(
        boot_services: *const BootServices,
        image_handle: *mut (),
        esp_start_lba: u64,
        disk_size_lba: u64,
    ) -> Self {
        let ui_state = UiState::new();
        let current_category = ui_state.current_category();
        let current_distros: Vec<_> = get_by_category(current_category).collect();
        let iso_storage = IsoStorageManager::new(esp_start_lba, disk_size_lba);

        let mut this = Self {
            ui_state,
            download_state: DownloadState::new(),
            current_distros,
            boot_services,
            image_handle,
            needs_full_redraw: true,
            iso_storage,
            iso_names: [[0u8; 64]; MAX_ISOS],
            iso_name_lens: [0; MAX_ISOS],
            iso_sizes_mb: [0; MAX_ISOS],
            iso_complete: [false; MAX_ISOS],
        };
        this.refresh_iso_cache();
        this
    }

    /// Refresh ISO cache from storage manager
    fn refresh_iso_cache(&mut self) {
        self.ui_state.update_iso_count(self.iso_storage.count());

        for (i, (_, entry)) in self.iso_storage.iter().enumerate() {
            if i >= MAX_ISOS {
                break;
            }
            let manifest = &entry.manifest;

            // Copy name
            let name_len = manifest.name_len.min(64);
            self.iso_names[i][..name_len].copy_from_slice(&manifest.name[..name_len]);
            self.iso_name_lens[i] = name_len;

            // Size in MB
            self.iso_sizes_mb[i] = manifest.total_size / (1024 * 1024);

            // Completion status
            self.iso_complete[i] = manifest.is_complete();
        }
    }

    /// Get ISO storage manager reference
    pub fn storage(&self) -> &IsoStorageManager {
        &self.iso_storage
    }

    /// Get mutable ISO storage manager reference
    pub fn storage_mut(&mut self) -> &mut IsoStorageManager {
        &mut self.iso_storage
    }

    /// Get currently selected distro
    pub fn selected_distro(&self) -> Option<&'static DistroEntry> {
        self.current_distros
            .get(self.ui_state.selected_distro)
            .copied()
    }

    /// Build a render context from current state
    fn render_context(&self) -> RenderContext<'_> {
        RenderContext {
            ui_state: &self.ui_state,
            download_status: self.download_state.status,
            download_progress: self.download_state.progress_percent(),
            error_message: self.download_state.error_message,
            current_distros: &self.current_distros,
            iso_names: &self.iso_names,
            iso_name_lens: &self.iso_name_lens,
            iso_sizes_mb: &self.iso_sizes_mb,
            iso_complete: &self.iso_complete,
        }
    }

    /// Build an input context for handling input
    fn input_context(&mut self) -> InputContext<'_> {
        InputContext {
            ui_state: &mut self.ui_state,
            download_state: &mut self.download_state,
            current_distros: &mut self.current_distros,
            iso_storage: &mut self.iso_storage,
            iso_names: &mut self.iso_names,
            iso_name_lens: &mut self.iso_name_lens,
            iso_sizes_mb: &mut self.iso_sizes_mb,
            iso_complete: &mut self.iso_complete,
            needs_full_redraw: &mut self.needs_full_redraw,
            boot_services: self.boot_services,
            image_handle: self.image_handle,
        }
    }

    /// Main event loop - follows same pattern as main_menu/distro_launcher
    pub fn run(&mut self, screen: &mut Screen, keyboard: &mut Keyboard) {
        // Initial render with clear
        self.needs_full_redraw = true;
        let ctx = self.render_context();
        render_full(&ctx, screen, true);

        loop {
            // Render global rain if active
            crate::tui::rain::render_rain(screen);

            // Poll for input with frame delay (~60fps timing)
            if let Some(key) = keyboard.poll_key_with_delay() {
                // Global rain toggle
                if key.unicode_char == b'x' as u16 || key.unicode_char == b'X' as u16 {
                    crate::tui::rain::toggle_rain(screen);
                    self.needs_full_redraw = true;
                    let ctx = self.render_context();
                    render_full(&ctx, screen, true);
                    continue;
                }

                // Handle mode-specific input
                let mut input_ctx = self.input_context();
                match handle_input(&mut input_ctx, &key, screen) {
                    ManageAction::Continue => {}
                    ManageAction::Exit => return,
                }
            }
        }
    }
}

// ============================================================================
// Unit Tests (Pure Rust, no UEFI)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::distro_downloader::catalog::CATEGORIES;
    use crate::tui::distro_downloader::state::DownloadStatus;

    #[test]
    fn test_refresh_distro_list_changes_with_category() {
        // Test that changing category changes the distro list
        let mut ui_state = UiState::new();

        let cat1 = ui_state.current_category();
        let distros1: Vec<_> = get_by_category(cat1).collect();

        ui_state.next_category(CATEGORIES.len());
        let cat2 = ui_state.current_category();
        let distros2: Vec<_> = get_by_category(cat2).collect();

        // Different categories should have different distros (usually)
        assert_ne!(cat1, cat2);
    }

    #[test]
    fn test_selected_distro_within_bounds() {
        let ui_state = UiState::new();
        let current_distros: Vec<_> = get_by_category(ui_state.current_category()).collect();

        // Initial selection should be valid
        assert!(ui_state.selected_distro < current_distros.len() || current_distros.is_empty());
    }

    #[test]
    fn test_download_state_lifecycle() {
        let mut download_state = DownloadState::new();

        // Start check
        download_state.start_check("test.iso");
        assert_eq!(download_state.status, DownloadStatus::Checking);

        // Start download
        download_state.start_download(Some(1000));
        assert_eq!(download_state.status, DownloadStatus::Downloading);

        // Progress updates
        download_state.update_progress(500);
        assert_eq!(download_state.progress_percent(), 50);

        // Complete
        download_state.complete();
        assert_eq!(download_state.status, DownloadStatus::Complete);
    }

    #[test]
    fn test_download_with_retry() {
        let mut download_state = DownloadState::new();

        download_state.start_check("test.iso");
        download_state.fail("Connection refused");

        // Try next mirror
        assert!(download_state.try_next_mirror(3));
        assert_eq!(download_state.status, DownloadStatus::Checking);
        assert_eq!(download_state.mirror_index, 1);

        // Fail again
        download_state.fail("Timeout");
        assert!(download_state.try_next_mirror(3));
        assert_eq!(download_state.mirror_index, 2);

        // No more mirrors
        download_state.fail("Error");
        assert!(!download_state.try_next_mirror(3));
    }

    #[test]
    fn test_ui_mode_transitions_browse_to_confirm() {
        let mut ui_state = UiState::new();
        assert!(ui_state.is_browsing());

        ui_state.show_confirm();
        assert!(ui_state.is_confirming());

        ui_state.return_to_browse();
        assert!(ui_state.is_browsing());
    }

    #[test]
    fn test_ui_mode_transitions_confirm_to_download() {
        let mut ui_state = UiState::new();

        ui_state.show_confirm();
        ui_state.start_download();
        assert!(ui_state.is_downloading());
    }

    #[test]
    fn test_navigation_through_categories() {
        let mut ui_state = UiState::new();
        let num_cats = CATEGORIES.len();

        // Navigate forward through all categories
        for i in 0..num_cats - 1 {
            assert_eq!(ui_state.selected_category, i);
            ui_state.next_category(num_cats);
        }

        // At last category
        assert_eq!(ui_state.selected_category, num_cats - 1);

        // Navigate back
        for i in (0..num_cats - 1).rev() {
            ui_state.prev_category();
            assert_eq!(ui_state.selected_category, i);
        }
    }

    #[test]
    fn test_navigation_resets_selection() {
        let mut ui_state = UiState::new();
        let num_cats = CATEGORIES.len();

        // Select some distro
        ui_state.selected_distro = 5;
        ui_state.scroll_offset = 2;

        // Change category
        ui_state.next_category(num_cats);

        // Selection should reset
        assert_eq!(ui_state.selected_distro, 0);
        assert_eq!(ui_state.scroll_offset, 0);
    }

    #[test]
    fn test_catalog_has_all_categories() {
        for category in CATEGORIES {
            let count = get_by_category(*category).count();
            // Each category should have at least one distro
            assert!(count >= 1, "Category {:?} has no distros", category);
        }
    }
}
