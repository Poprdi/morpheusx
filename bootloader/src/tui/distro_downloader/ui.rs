//! Distro Downloader UI
//!
//! Main TUI for browsing and downloading Linux distributions.
//! Integrates catalog, state, and renderer modules.

use alloc::vec::Vec;

use super::catalog::{DistroEntry, DistroCategory, CATEGORIES, DISTRO_CATALOG, get_by_category};
use super::state::{DownloadState, DownloadStatus, UiState, UiMode};
use super::renderer::DownloaderRenderer;
use crate::tui::input::Keyboard;
use crate::tui::renderer::Screen;
use crate::BootServices;

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
}

impl DistroDownloader {
    /// Create a new distro downloader
    pub fn new(boot_services: *const BootServices, image_handle: *mut ()) -> Self {
        morpheus_core::logger::log("DistroDownloader::new()");

        let ui_state = UiState::new();
        let current_category = ui_state.current_category();
        let current_distros: Vec<_> = get_by_category(current_category).collect();

        morpheus_core::logger::log("DistroDownloader: catalog loaded");

        Self {
            ui_state,
            download_state: DownloadState::new(),
            current_distros,
            boot_services,
            image_handle,
        }
    }

    /// Refresh the distro list for current category
    fn refresh_distro_list(&mut self) {
        let category = self.ui_state.current_category();
        self.current_distros = get_by_category(category).collect();
        morpheus_core::logger::log("DistroDownloader: refreshed distro list");
        // Log count for debugging
        if self.current_distros.is_empty() {
            morpheus_core::logger::log("DistroDownloader: WARNING - no distros in category");
        }
    }

    /// Get currently selected distro
    pub fn selected_distro(&self) -> Option<&'static DistroEntry> {
        self.current_distros.get(self.ui_state.selected_distro).copied()
    }

    /// Handle navigation input
    fn handle_navigation(&mut self, scan_code: u16, unicode_char: u16) -> bool {
        match scan_code {
            // Up arrow
            0x01 => {
                morpheus_core::logger::log("DistroDownloader: nav UP");
                self.ui_state.prev_distro();
                true
            }
            // Down arrow
            0x02 => {
                morpheus_core::logger::log("DistroDownloader: nav DOWN");
                let count = self.current_distros.len();
                self.ui_state.next_distro(count);
                true
            }
            // Left arrow - previous category
            0x04 => {
                morpheus_core::logger::log("DistroDownloader: nav LEFT (prev category)");
                self.ui_state.prev_category();
                self.refresh_distro_list();
                true
            }
            // Right arrow - next category
            0x03 => {
                morpheus_core::logger::log("DistroDownloader: nav RIGHT (next category)");
                self.ui_state.next_category(CATEGORIES.len());
                self.refresh_distro_list();
                true
            }
            // ESC
            0x17 => {
                morpheus_core::logger::log("DistroDownloader: ESC pressed - exiting browse");
                false // Signal to exit
            }
            _ => {
                // Enter key
                if unicode_char == 0x0D {
                    if self.ui_state.is_browsing() {
                        if let Some(distro) = self.selected_distro() {
                            morpheus_core::logger::log("DistroDownloader: ENTER - confirm dialog");
                        }
                        self.ui_state.show_confirm();
                    }
                }
                true
            }
        }
    }

    /// Handle confirmation dialog input
    fn handle_confirm(&mut self, scan_code: u16, unicode_char: u16) -> bool {
        // ESC - cancel
        if scan_code == 0x17 {
            morpheus_core::logger::log("DistroDownloader: confirm ESC - cancelled");
            self.ui_state.return_to_browse();
            return true;
        }

        // Y/y - confirm download
        if unicode_char == b'y' as u16 || unicode_char == b'Y' as u16 {
            morpheus_core::logger::log("DistroDownloader: confirm Y - starting download");
            if let Some(distro) = self.selected_distro() {
                self.start_download(distro);
            } else {
                morpheus_core::logger::log("DistroDownloader: ERROR - no distro selected");
            }
            return true;
        }

        // N/n - cancel
        if unicode_char == b'n' as u16 || unicode_char == b'N' as u16 {
            morpheus_core::logger::log("DistroDownloader: confirm N - cancelled");
            self.ui_state.return_to_browse();
            return true;
        }

        true
    }

    /// Handle result screen input
    fn handle_result(&mut self, scan_code: u16, _unicode_char: u16) -> bool {
        // Any key returns to browse
        if scan_code == 0x17 || scan_code != 0 {
            morpheus_core::logger::log("DistroDownloader: result screen - returning to browse");
            self.ui_state.return_to_browse();
            self.download_state.reset();
            return true;
        }
        true
    }

    /// Start downloading a distribution
    fn start_download(&mut self, distro: &'static DistroEntry) {
        morpheus_core::logger::log("DistroDownloader: === DOWNLOAD START ===");
        morpheus_core::logger::log("DistroDownloader: distro selected");
        // Log URL info
        if distro.url.starts_with("https://") {
            morpheus_core::logger::log("DistroDownloader: URL is HTTPS");
        } else {
            morpheus_core::logger::log("DistroDownloader: URL is HTTP");
        }
        
        self.ui_state.start_download();
        self.download_state.start_check(distro.filename);
        morpheus_core::logger::log("DistroDownloader: state -> Downloading");

        // TODO: Actual HTTP download integration
        // For now, simulate the download process
        self.simulate_download(distro);
    }

    /// Simulate download for testing (will be replaced with real HTTP)
    fn simulate_download(&mut self, distro: &'static DistroEntry) {
        morpheus_core::logger::log("DistroDownloader: [SIMULATED] download flow");
        morpheus_core::logger::log("DistroDownloader: [SIMULATED] 1. Would init HTTP client");
        morpheus_core::logger::log("DistroDownloader: [SIMULATED] 2. Would send HEAD request");

        // Simulate checking
        self.download_state.start_download(Some(distro.size_bytes as usize));
        morpheus_core::logger::log("DistroDownloader: [SIMULATED] 3. Would send GET request");
        morpheus_core::logger::log("DistroDownloader: [SIMULATED] 4. Would write to /isos/");

        // For actual implementation, this would:
        // 1. Initialize HTTP client via morpheus-network
        // 2. Send HEAD request to get Content-Length
        // 3. Create file on ESP: /isos/{filename}
        // 4. Send GET request with progress callback
        // 5. Write chunks to file, update progress
        // 6. Verify checksum if available

        // For now, mark as "would download" and show result
        morpheus_core::logger::log("DistroDownloader: [SIMULATED] download complete");
        self.ui_state.show_result("Download simulation complete");
        self.download_state.complete();
        morpheus_core::logger::log("DistroDownloader: === DOWNLOAD END ===");
    }

    /// Render the current UI state
    fn render(&self, screen: &mut Screen) {
        screen.clear();
        DownloaderRenderer::render_header(screen);

        let y_start = 8;

        match self.ui_state.mode {
            UiMode::Browse => {
                // Category tabs
                DownloaderRenderer::render_categories(
                    screen,
                    CATEGORIES,
                    self.ui_state.selected_category,
                    y_start,
                );

                // Distro list
                let distro_refs: Vec<&DistroEntry> = self.current_distros.iter().copied().collect();
                DownloaderRenderer::render_distro_list(
                    screen,
                    &distro_refs,
                    self.ui_state.selected_distro,
                    self.ui_state.scroll_offset,
                    y_start + 2,
                    UiState::VISIBLE_ITEMS,
                );

                // Details panel for selected distro
                if let Some(distro) = self.selected_distro() {
                    DownloaderRenderer::render_details(screen, distro, y_start + 12);
                }

                // Footer
                DownloaderRenderer::render_footer(screen, y_start + 18);
            }

            UiMode::Confirm => {
                if let Some(distro) = self.selected_distro() {
                    self.render_confirm_dialog(screen, distro, y_start);
                }
            }

            UiMode::Downloading => {
                if let Some(distro) = self.selected_distro() {
                    DownloaderRenderer::render_progress(
                        screen,
                        distro,
                        self.download_state.bytes_downloaded,
                        self.download_state.total_bytes,
                        self.download_state.status.as_str(),
                        y_start,
                    );
                }
            }

            UiMode::Result => {
                let y = y_start + 4;
                if self.download_state.status == DownloadStatus::Complete {
                    DownloaderRenderer::render_success(
                        screen,
                        self.ui_state.status_message.unwrap_or("Download complete!"),
                        y,
                    );
                } else if self.download_state.status == DownloadStatus::Failed {
                    DownloaderRenderer::render_error(
                        screen,
                        self.download_state.error_message.unwrap_or("Download failed"),
                        y,
                    );
                }
                // Show "Press any key to continue"
                screen.put_str_at(
                    2, y + 4,
                    "Press ESC to return to browser...",
                    crate::tui::renderer::EFI_GREEN,
                    crate::tui::renderer::EFI_BLACK,
                );
            }
        }
    }

    /// Render confirmation dialog
    fn render_confirm_dialog(&self, screen: &mut Screen, distro: &DistroEntry, y: usize) {
        let x = 10;

        screen.put_str_at(x, y, "╔════════════════════════════════════════════════════════╗",
            crate::tui::renderer::EFI_GREEN, crate::tui::renderer::EFI_BLACK);
        screen.put_str_at(x, y + 1, "║              CONFIRM DOWNLOAD                          ║",
            crate::tui::renderer::EFI_LIGHTGREEN, crate::tui::renderer::EFI_BLACK);
        screen.put_str_at(x, y + 2, "╠════════════════════════════════════════════════════════╣",
            crate::tui::renderer::EFI_GREEN, crate::tui::renderer::EFI_BLACK);
        screen.put_str_at(x, y + 3, "║                                                        ║",
            crate::tui::renderer::EFI_GREEN, crate::tui::renderer::EFI_BLACK);
        
        // Distro name
        screen.put_str_at(x + 3, y + 3, "Distro: ", 
            crate::tui::renderer::EFI_GREEN, crate::tui::renderer::EFI_BLACK);
        screen.put_str_at(x + 11, y + 3, distro.name,
            crate::tui::renderer::EFI_LIGHTGREEN, crate::tui::renderer::EFI_BLACK);

        screen.put_str_at(x, y + 4, "║                                                        ║",
            crate::tui::renderer::EFI_GREEN, crate::tui::renderer::EFI_BLACK);
        screen.put_str_at(x + 3, y + 4, "Size: ",
            crate::tui::renderer::EFI_GREEN, crate::tui::renderer::EFI_BLACK);
        screen.put_str_at(x + 9, y + 4, distro.size_str(),
            crate::tui::renderer::EFI_LIGHTGREEN, crate::tui::renderer::EFI_BLACK);

        screen.put_str_at(x, y + 5, "║                                                        ║",
            crate::tui::renderer::EFI_GREEN, crate::tui::renderer::EFI_BLACK);
        screen.put_str_at(x + 3, y + 5, "File: ",
            crate::tui::renderer::EFI_GREEN, crate::tui::renderer::EFI_BLACK);
        screen.put_str_at(x + 9, y + 5, distro.filename,
            crate::tui::renderer::EFI_GREEN, crate::tui::renderer::EFI_BLACK);

        screen.put_str_at(x, y + 6, "║                                                        ║",
            crate::tui::renderer::EFI_GREEN, crate::tui::renderer::EFI_BLACK);
        screen.put_str_at(x, y + 7, "╠════════════════════════════════════════════════════════╣",
            crate::tui::renderer::EFI_GREEN, crate::tui::renderer::EFI_BLACK);
        screen.put_str_at(x, y + 8, "║     Download to /isos/ on ESP?  [Y]es  [N]o            ║",
            crate::tui::renderer::EFI_GREEN, crate::tui::renderer::EFI_BLACK);
        screen.put_str_at(x, y + 9, "╚════════════════════════════════════════════════════════╝",
            crate::tui::renderer::EFI_GREEN, crate::tui::renderer::EFI_BLACK);
    }

    /// Main event loop
    pub fn run(&mut self, screen: &mut Screen, keyboard: &mut Keyboard) {
        morpheus_core::logger::log("DistroDownloader::run() - entering event loop");

        loop {
            // Render current state
            self.render(screen);

            // Render rain effect if active
            crate::tui::rain::render_rain(screen);

            // Poll for input
            if let Some(key) = keyboard.poll_key_with_delay() {
                // Global rain toggle
                if key.unicode_char == b'x' as u16 || key.unicode_char == b'X' as u16 {
                    crate::tui::rain::toggle_rain(screen);
                    continue;
                }

                // Handle input based on current mode
                let continue_loop = match self.ui_state.mode {
                    UiMode::Browse => self.handle_navigation(key.scan_code, key.unicode_char),
                    UiMode::Confirm => self.handle_confirm(key.scan_code, key.unicode_char),
                    UiMode::Downloading => {
                        // ESC cancels download
                        if key.scan_code == 0x17 {
                            self.download_state.fail("Cancelled by user");
                            self.ui_state.show_result("Download cancelled");
                        }
                        true
                    }
                    UiMode::Result => self.handle_result(key.scan_code, key.unicode_char),
                };

                if !continue_loop {
                    morpheus_core::logger::log("DistroDownloader::run() - exiting");
                    return;
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

    // Mock/test helpers - we test the logic without UEFI

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
        // Lists may or may not be same size, but categories differ
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
            // (Server might only have Ubuntu Server)
            assert!(count >= 1, "Category {:?} has no distros", category);
        }
    }
}
