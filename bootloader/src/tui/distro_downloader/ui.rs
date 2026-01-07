//! Distro Downloader UI
//!
//! Main TUI for browsing and downloading Linux distributions.
//! Follows the same rendering pattern as main_menu and distro_launcher:
//! - Clear screen once at start
//! - Render initial state
//! - Only re-render after handling input (no clear in render loop)

use alloc::vec::Vec;

use super::catalog::{DistroEntry, CATEGORIES, get_by_category};
use super::state::{DownloadState, DownloadStatus, UiState, UiMode};
use crate::tui::input::{InputKey, Keyboard};
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN, EFI_RED};
use crate::BootServices;

// Layout constants
const HEADER_Y: usize = 0;
const CATEGORY_Y: usize = 3;
const LIST_Y: usize = 5;
const DETAILS_Y: usize = 14;
const FOOTER_Y: usize = 19;
const VISIBLE_ITEMS: usize = 8;

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
}

impl DistroDownloader {
    /// Create a new distro downloader
    pub fn new(boot_services: *const BootServices, image_handle: *mut ()) -> Self {
        let ui_state = UiState::new();
        let current_category = ui_state.current_category();
        let current_distros: Vec<_> = get_by_category(current_category).collect();

        Self {
            ui_state,
            download_state: DownloadState::new(),
            current_distros,
            boot_services,
            image_handle,
            needs_full_redraw: true,
        }
    }

    /// Refresh the distro list for current category
    fn refresh_distro_list(&mut self) {
        let category = self.ui_state.current_category();
        self.current_distros = get_by_category(category).collect();
        self.needs_full_redraw = true;
    }

    /// Get currently selected distro
    pub fn selected_distro(&self) -> Option<&'static DistroEntry> {
        self.current_distros.get(self.ui_state.selected_distro).copied()
    }

    /// Handle input and return whether to continue the loop
    fn handle_input(&mut self, key: &InputKey, screen: &mut Screen) -> bool {
        match self.ui_state.mode {
            UiMode::Browse => self.handle_browse_input(key, screen),
            UiMode::Confirm => self.handle_confirm_input(key, screen),
            UiMode::Downloading => self.handle_download_input(key, screen),
            UiMode::Result => self.handle_result_input(key, screen),
        }
    }

    fn handle_browse_input(&mut self, key: &InputKey, screen: &mut Screen) -> bool {
        match key.scan_code {
            // Up arrow
            0x01 => {
                self.ui_state.prev_distro();
                self.render_list_and_details(screen);
            }
            // Down arrow
            0x02 => {
                let count = self.current_distros.len();
                self.ui_state.next_distro(count);
                self.render_list_and_details(screen);
            }
            // Left arrow - previous category
            0x04 => {
                self.ui_state.prev_category();
                self.refresh_distro_list();
                self.render_full(screen);
            }
            // Right arrow - next category
            0x03 => {
                self.ui_state.next_category(CATEGORIES.len());
                self.refresh_distro_list();
                self.render_full(screen);
            }
            // ESC - exit
            0x17 => {
                return false;
            }
            _ => {
                // Enter key - show confirm dialog
                if key.unicode_char == 0x0D && self.selected_distro().is_some() {
                    self.ui_state.show_confirm();
                    self.needs_full_redraw = true;
                    self.render_full(screen);
                }
            }
        }
        true
    }

    fn handle_confirm_input(&mut self, key: &InputKey, screen: &mut Screen) -> bool {
        // ESC - cancel
        if key.scan_code == 0x17 {
            self.ui_state.return_to_browse();
            self.needs_full_redraw = true;
            self.render_full(screen);
            return true;
        }

        // Y/y - confirm download
        if key.unicode_char == b'y' as u16 || key.unicode_char == b'Y' as u16 {
            if let Some(distro) = self.selected_distro() {
                self.start_download(distro, screen);
            }
            return true;
        }

        // N/n - cancel
        if key.unicode_char == b'n' as u16 || key.unicode_char == b'N' as u16 {
            self.ui_state.return_to_browse();
            self.needs_full_redraw = true;
            self.render_full(screen);
        }

        true
    }

    fn handle_download_input(&mut self, key: &InputKey, screen: &mut Screen) -> bool {
        // ESC cancels download
        if key.scan_code == 0x17 {
            self.download_state.fail("Cancelled by user");
            self.ui_state.show_result("Download cancelled");
            self.needs_full_redraw = true;
            self.render_full(screen);
        }
        true
    }

    fn handle_result_input(&mut self, key: &InputKey, screen: &mut Screen) -> bool {
        // Any key returns to browse
        if key.scan_code != 0 || key.unicode_char != 0 {
            self.ui_state.return_to_browse();
            self.download_state.reset();
            self.needs_full_redraw = true;
            self.render_full(screen);
        }
        true
    }

    /// Start downloading a distribution
    fn start_download(&mut self, distro: &'static DistroEntry, screen: &mut Screen) {
        self.ui_state.start_download();
        self.download_state.start_check(distro.filename);
        self.needs_full_redraw = true;
        self.render_full(screen);

        // TODO: Actual HTTP download integration
        // For now, simulate the download process
        self.simulate_download(distro, screen);
    }

    /// Simulate download for testing (will be replaced with real HTTP)
    fn simulate_download(&mut self, distro: &'static DistroEntry, screen: &mut Screen) {
        self.download_state.start_download(Some(distro.size_bytes as usize));
        self.render_progress_only(screen);

        // For actual implementation, this would:
        // 1. Initialize HTTP client via morpheus-network
        // 2. Send HEAD request to get Content-Length
        // 3. Create file on ESP: /isos/{filename}
        // 4. Send GET request with progress callback
        // 5. Write chunks to file, update progress
        // 6. Verify checksum if available

        // For now, mark as complete and show result
        self.ui_state.show_result("Download simulation complete");
        self.download_state.complete();
        self.needs_full_redraw = true;
        self.render_full(screen);
    }

    /// Full render - clears screen if needed and draws everything
    fn render_full(&mut self, screen: &mut Screen) {
        if self.needs_full_redraw {
            screen.clear();
            self.needs_full_redraw = false;
        }

        match self.ui_state.mode {
            UiMode::Browse => {
                self.render_header(screen);
                self.render_categories(screen);
                self.render_list(screen);
                self.render_details(screen);
                self.render_footer(screen);
            }
            UiMode::Confirm => {
                self.render_header(screen);
                self.render_confirm_dialog(screen);
            }
            UiMode::Downloading => {
                self.render_header(screen);
                self.render_progress_only(screen);
            }
            UiMode::Result => {
                self.render_header(screen);
                self.render_result(screen);
            }
        }
    }

    /// Render only the list and details (for navigation - no clear needed)
    fn render_list_and_details(&self, screen: &mut Screen) {
        self.render_list(screen);
        self.render_details(screen);
    }

    fn render_header(&self, screen: &mut Screen) {
        let title = "=== DISTRO DOWNLOADER ===";
        let x = screen.center_x(title.len());
        screen.put_str_at(x, HEADER_Y, title, EFI_LIGHTGREEN, EFI_BLACK);

        let subtitle = "Download Linux distributions to ESP";
        let x = screen.center_x(subtitle.len());
        screen.put_str_at(x, HEADER_Y + 1, subtitle, EFI_DARKGREEN, EFI_BLACK);
    }

    fn render_categories(&self, screen: &mut Screen) {
        let x = 2;
        let y = CATEGORY_Y;
        let mut current_x = x;

        // Clear the category line
        screen.put_str_at(x, y, "                                                                              ", EFI_BLACK, EFI_BLACK);

        screen.put_str_at(x, y, "Category: ", EFI_GREEN, EFI_BLACK);
        current_x += 10;

        for (i, cat) in CATEGORIES.iter().enumerate() {
            let name = cat.name();
            let (fg, bg) = if i == self.ui_state.selected_category {
                (EFI_BLACK, EFI_LIGHTGREEN)
            } else {
                (EFI_GREEN, EFI_BLACK)
            };

            screen.put_str_at(current_x, y, "[", EFI_GREEN, EFI_BLACK);
            current_x += 1;
            screen.put_str_at(current_x, y, name, fg, bg);
            current_x += name.len();
            screen.put_str_at(current_x, y, "]", EFI_GREEN, EFI_BLACK);
            current_x += 2;
        }
    }

    fn render_list(&self, screen: &mut Screen) {
        let x = 2;
        let y = LIST_Y;

        // Column headers
        screen.put_str_at(x + 2, y, "Name              ", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 22, y, "Version   ", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 34, y, "Size         ", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 48, y, "Description                   ", EFI_DARKGREEN, EFI_BLACK);

        // Separator
        screen.put_str_at(x, y + 1, "--------------------------------------------------------------------------------", EFI_DARKGREEN, EFI_BLACK);

        // Clear list area
        for row in 0..VISIBLE_ITEMS {
            screen.put_str_at(x, y + 2 + row, "                                                                                ", EFI_BLACK, EFI_BLACK);
        }

        // Render visible items
        let scroll = self.ui_state.scroll_offset;
        let visible_end = (scroll + VISIBLE_ITEMS).min(self.current_distros.len());

        for (display_idx, list_idx) in (scroll..visible_end).enumerate() {
            let distro = self.current_distros[list_idx];
            let row_y = y + 2 + display_idx;
            let is_selected = list_idx == self.ui_state.selected_distro;

            let (fg, bg) = if is_selected {
                (EFI_BLACK, EFI_LIGHTGREEN)
            } else {
                (EFI_GREEN, EFI_BLACK)
            };

            // Selection indicator
            let marker = if is_selected { ">>" } else { "  " };
            screen.put_str_at(x, row_y, marker, EFI_LIGHTGREEN, EFI_BLACK);

            // Name (padded/truncated to 18 chars)
            let name = Self::pad_or_truncate(distro.name, 18);
            screen.put_str_at(x + 2, row_y, &name, fg, bg);

            // Version (padded/truncated to 10 chars)  
            let version = Self::pad_or_truncate(distro.version, 10);
            screen.put_str_at(x + 22, row_y, &version, fg, bg);

            // Size
            let size = Self::pad_or_truncate(distro.size_str(), 12);
            screen.put_str_at(x + 34, row_y, &size, fg, bg);

            // Description (truncated to 30 chars)
            let desc = Self::pad_or_truncate(distro.description, 30);
            screen.put_str_at(x + 48, row_y, &desc, fg, bg);
        }

        // Scroll indicators
        if scroll > 0 {
            screen.put_str_at(x + 78, y + 2, "^", EFI_LIGHTGREEN, EFI_BLACK);
        } else {
            screen.put_str_at(x + 78, y + 2, " ", EFI_BLACK, EFI_BLACK);
        }
        if visible_end < self.current_distros.len() {
            screen.put_str_at(x + 78, y + 1 + VISIBLE_ITEMS, "v", EFI_LIGHTGREEN, EFI_BLACK);
        } else {
            screen.put_str_at(x + 78, y + 1 + VISIBLE_ITEMS, " ", EFI_BLACK, EFI_BLACK);
        }
    }

    fn render_details(&self, screen: &mut Screen) {
        let x = 2;
        let y = DETAILS_Y;

        // Clear details area
        for row in 0..4 {
            screen.put_str_at(x, y + row, "                                                                                ", EFI_BLACK, EFI_BLACK);
        }

        if let Some(distro) = self.selected_distro() {
            // Box top
            screen.put_str_at(x, y, "+-[ Details ]", EFI_GREEN, EFI_BLACK);
            for i in 14..78 {
                screen.put_str_at(x + i, y, "-", EFI_GREEN, EFI_BLACK);
            }
            screen.put_str_at(x + 78, y, "+", EFI_GREEN, EFI_BLACK);

            // Content line 1
            screen.put_str_at(x, y + 1, "|", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x + 2, y + 1, "Name: ", EFI_DARKGREEN, EFI_BLACK);
            screen.put_str_at(x + 8, y + 1, distro.name, EFI_LIGHTGREEN, EFI_BLACK);
            screen.put_str_at(x + 30, y + 1, "Arch: ", EFI_DARKGREEN, EFI_BLACK);
            screen.put_str_at(x + 36, y + 1, distro.arch, EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x + 50, y + 1, "Live: ", EFI_DARKGREEN, EFI_BLACK);
            screen.put_str_at(x + 56, y + 1, if distro.is_live { "Yes" } else { "No " }, EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x + 78, y + 1, "|", EFI_GREEN, EFI_BLACK);

            // Content line 2 - URL
            screen.put_str_at(x, y + 2, "|", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x + 2, y + 2, "URL: ", EFI_DARKGREEN, EFI_BLACK);
            let url_display = if distro.url.len() > 70 { &distro.url[..70] } else { distro.url };
            screen.put_str_at(x + 7, y + 2, url_display, EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x + 78, y + 2, "|", EFI_GREEN, EFI_BLACK);

            // Box bottom
            screen.put_str_at(x, y + 3, "+", EFI_GREEN, EFI_BLACK);
            for i in 1..78 {
                screen.put_str_at(x + i, y + 3, "-", EFI_GREEN, EFI_BLACK);
            }
            screen.put_str_at(x + 78, y + 3, "+", EFI_GREEN, EFI_BLACK);
        }
    }

    fn render_footer(&self, screen: &mut Screen) {
        let x = 2;
        let y = FOOTER_Y;

        screen.put_str_at(x, y, "+-[ Controls ]", EFI_GREEN, EFI_BLACK);
        for i in 15..78 {
            screen.put_str_at(x + i, y, "-", EFI_GREEN, EFI_BLACK);
        }
        screen.put_str_at(x + 78, y, "+", EFI_GREEN, EFI_BLACK);

        screen.put_str_at(x, y + 1, "|", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 2, y + 1, "[UP/DOWN] Select", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 22, y + 1, "[LEFT/RIGHT] Category", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 48, y + 1, "[ENTER] Download", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 68, y + 1, "[ESC] Back", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 78, y + 1, "|", EFI_GREEN, EFI_BLACK);

        screen.put_str_at(x, y + 2, "+", EFI_GREEN, EFI_BLACK);
        for i in 1..78 {
            screen.put_str_at(x + i, y + 2, "-", EFI_GREEN, EFI_BLACK);
        }
        screen.put_str_at(x + 78, y + 2, "+", EFI_GREEN, EFI_BLACK);
    }

    fn render_confirm_dialog(&self, screen: &mut Screen) {
        if let Some(distro) = self.selected_distro() {
            let x = 10;
            let y = 8;

            // Dialog box using ASCII (more compatible than Unicode box chars)
            screen.put_str_at(x, y,     "+--------------------------------------------------------+", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x, y + 1, "|              CONFIRM DOWNLOAD                          |", EFI_LIGHTGREEN, EFI_BLACK);
            screen.put_str_at(x, y + 2, "+--------------------------------------------------------+", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x, y + 3, "|                                                        |", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x, y + 4, "|                                                        |", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x, y + 5, "|                                                        |", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x, y + 6, "+--------------------------------------------------------+", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x, y + 7, "|     Download to /isos/ on ESP?    [Y]es   [N]o         |", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x, y + 8, "+--------------------------------------------------------+", EFI_GREEN, EFI_BLACK);

            // Content
            screen.put_str_at(x + 3, y + 3, "Distro: ", EFI_DARKGREEN, EFI_BLACK);
            screen.put_str_at(x + 11, y + 3, distro.name, EFI_LIGHTGREEN, EFI_BLACK);

            screen.put_str_at(x + 3, y + 4, "Size:   ", EFI_DARKGREEN, EFI_BLACK);
            screen.put_str_at(x + 11, y + 4, distro.size_str(), EFI_GREEN, EFI_BLACK);

            screen.put_str_at(x + 3, y + 5, "File:   ", EFI_DARKGREEN, EFI_BLACK);
            let filename = if distro.filename.len() > 40 { &distro.filename[..40] } else { distro.filename };
            screen.put_str_at(x + 11, y + 5, filename, EFI_GREEN, EFI_BLACK);
        }
    }

    fn render_progress_only(&self, screen: &mut Screen) {
        if let Some(distro) = self.selected_distro() {
            let x = 10;
            let y = 8;

            screen.put_str_at(x, y, "Downloading: ", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x + 13, y, distro.name, EFI_LIGHTGREEN, EFI_BLACK);

            // Progress bar
            let bar_width = 50;
            let progress = self.download_state.progress_percent();
            let filled = (bar_width * progress) / 100;

            screen.put_str_at(x, y + 2, "[", EFI_GREEN, EFI_BLACK);
            for i in 0..bar_width {
                let ch = if i < filled { "=" } else if i == filled { ">" } else { " " };
                screen.put_str_at(x + 1 + i, y + 2, ch, EFI_LIGHTGREEN, EFI_BLACK);
            }
            screen.put_str_at(x + 1 + bar_width, y + 2, "]", EFI_GREEN, EFI_BLACK);

            // Status
            screen.put_str_at(x, y + 4, "Status: ", EFI_DARKGREEN, EFI_BLACK);
            screen.put_str_at(x + 8, y + 4, self.download_state.status.as_str(), EFI_GREEN, EFI_BLACK);
        }
    }

    fn render_result(&self, screen: &mut Screen) {
        let x = 10;
        let y = 10;

        if self.download_state.status == DownloadStatus::Complete {
            screen.put_str_at(x, y, "SUCCESS: ", EFI_LIGHTGREEN, EFI_BLACK);
            let msg = self.ui_state.status_message.unwrap_or("Download complete!");
            screen.put_str_at(x + 9, y, msg, EFI_LIGHTGREEN, EFI_BLACK);
        } else {
            screen.put_str_at(x, y, "FAILED: ", EFI_RED, EFI_BLACK);
            let msg = self.download_state.error_message.unwrap_or("Download failed");
            screen.put_str_at(x + 8, y, msg, EFI_RED, EFI_BLACK);
        }

        screen.put_str_at(x, y + 2, "Press any key to continue...", EFI_DARKGREEN, EFI_BLACK);
    }

    /// Helper: pad or truncate string to exact length
    fn pad_or_truncate(s: &str, len: usize) -> alloc::string::String {
        use alloc::string::String;
        let mut result = String::with_capacity(len);
        for (i, c) in s.chars().enumerate() {
            if i >= len {
                break;
            }
            result.push(c);
        }
        while result.len() < len {
            result.push(' ');
        }
        result
    }

    /// Main event loop - follows same pattern as main_menu/distro_launcher
    pub fn run(&mut self, screen: &mut Screen, keyboard: &mut Keyboard) {
        // Initial render with clear
        self.needs_full_redraw = true;
        self.render_full(screen);

        loop {
            // Render global rain if active
            crate::tui::rain::render_rain(screen);

            // Poll for input with frame delay (~60fps timing)
            if let Some(key) = keyboard.poll_key_with_delay() {
                // Global rain toggle
                if key.unicode_char == b'x' as u16 || key.unicode_char == b'X' as u16 {
                    crate::tui::rain::toggle_rain(screen);
                    self.needs_full_redraw = true;
                    self.render_full(screen);
                    continue;
                }

                // Handle mode-specific input
                if !self.handle_input(&key, screen) {
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
