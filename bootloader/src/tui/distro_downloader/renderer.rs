//! Distro Downloader Renderer
//!
//! Rendering utilities for the distro downloader TUI.

use super::catalog::{DistroCategory, DistroEntry, CATEGORIES};
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN, EFI_RED};

const HEADER_ART: &[&str] = &[
    " ____  _     _              ____                      _                 _           ",
    "|  _ \\(_)___| |_ _ __ ___  |  _ \\  _____      ___ __ | | ___   __ _  __| | ___ _ __ ",
    "| | | | / __| __| '__/ _ \\ | | | |/ _ \\ \\ /\\ / / '_ \\| |/ _ \\ / _` |/ _` |/ _ \\ '__|",
    "| |_| | \\__ \\ |_| | | (_) || |_| | (_) \\ V  V /| | | | | (_) | (_| | (_| |  __/ |   ",
    "|____/|_|___/\\__|_|  \\___/ |____/ \\___/ \\_/\\_/ |_| |_|_|\\___/ \\__,_|\\__,_|\\___|_|   ",
];

const BORDER_H: &str = "═";
const BORDER_V: &str = "║";
const CORNER_TL: &str = "╔";
const CORNER_TR: &str = "╗";
const CORNER_BL: &str = "╚";
const CORNER_BR: &str = "╝";
const TEE_L: &str = "╠";
const TEE_R: &str = "╣";

pub struct DownloaderRenderer;

impl DownloaderRenderer {
    /// Render the header with title
    pub fn render_header(screen: &mut Screen) {
        morpheus_core::logger::log("Renderer: render_header()");
        let width = screen.width();
        let header_width = HEADER_ART[0].len();
        let x = if width > header_width {
            (width - header_width) / 2
        } else {
            0
        };

        // Draw border top
        screen.put_str_at(x, 0, CORNER_TL, EFI_GREEN, EFI_BLACK);
        for i in 1..header_width + 1 {
            screen.put_str_at(x + i, 0, BORDER_H, EFI_GREEN, EFI_BLACK);
        }
        screen.put_str_at(x + header_width + 1, 0, CORNER_TR, EFI_GREEN, EFI_BLACK);

        // Draw header art
        for (i, line) in HEADER_ART.iter().enumerate() {
            let y = i + 1;
            screen.put_str_at(x, y, BORDER_V, EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x + 1, y, line, EFI_LIGHTGREEN, EFI_BLACK);
            screen.put_str_at(x + header_width + 1, y, BORDER_V, EFI_GREEN, EFI_BLACK);
        }

        // Draw separator
        let sep_y = HEADER_ART.len() + 1;
        screen.put_str_at(x, sep_y, TEE_L, EFI_GREEN, EFI_BLACK);
        for i in 1..header_width + 1 {
            screen.put_str_at(x + i, sep_y, BORDER_H, EFI_GREEN, EFI_BLACK);
        }
        screen.put_str_at(x + header_width + 1, sep_y, TEE_R, EFI_GREEN, EFI_BLACK);
    }

    /// Render category tabs
    pub fn render_categories(
        screen: &mut Screen,
        categories: &[DistroCategory],
        selected_category: usize,
        y: usize,
    ) {
        morpheus_core::logger::log("Renderer: render_categories()");
        let x = 2;
        let mut current_x = x;

        screen.put_str_at(x, y, "Categories: ", EFI_GREEN, EFI_BLACK);
        current_x += 12;

        for (i, cat) in categories.iter().enumerate() {
            let name = cat.name();
            let (fg, bg) = if i == selected_category {
                (EFI_BLACK, EFI_LIGHTGREEN)
            } else {
                (EFI_GREEN, EFI_BLACK)
            };

            // Draw tab with brackets
            screen.put_str_at(current_x, y, "[", EFI_GREEN, EFI_BLACK);
            current_x += 1;
            screen.put_str_at(current_x, y, name, fg, bg);
            current_x += name.len();
            screen.put_str_at(current_x, y, "]", EFI_GREEN, EFI_BLACK);
            current_x += 2; // bracket + space
        }
    }

    /// Render the distro list
    pub fn render_distro_list(
        screen: &mut Screen,
        distros: &[&DistroEntry],
        selected_index: usize,
        scroll_offset: usize,
        y_start: usize,
        max_items: usize,
    ) {
        morpheus_core::logger::log("Renderer: render_distro_list()");
        let x = 2;

        // Column headers
        screen.put_str_at(x + 2, y_start, "Name", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 22, y_start, "Version", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 34, y_start, "Size", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 48, y_start, "Description", EFI_DARKGREEN, EFI_BLACK);

        // Separator line
        let sep_y = y_start + 1;
        for i in 0..76 {
            screen.put_str_at(x + i, sep_y, "-", EFI_DARKGREEN, EFI_BLACK);
        }

        // List items
        let visible_end = (scroll_offset + max_items).min(distros.len());
        for (display_idx, list_idx) in (scroll_offset..visible_end).enumerate() {
            let distro = distros[list_idx];
            let y = y_start + 2 + display_idx;
            let is_selected = list_idx == selected_index;

            let (fg, bg) = if is_selected {
                (EFI_BLACK, EFI_LIGHTGREEN)
            } else {
                (EFI_GREEN, EFI_BLACK)
            };

            // Selection indicator
            if is_selected {
                screen.put_str_at(x, y, ">>", EFI_LIGHTGREEN, EFI_BLACK);
            } else {
                screen.put_str_at(x, y, "  ", EFI_GREEN, EFI_BLACK);
            }

            // Name (truncate if needed)
            let name = if distro.name.len() > 18 {
                &distro.name[..18]
            } else {
                distro.name
            };
            screen.put_str_at(x + 2, y, name, fg, bg);

            // Version
            let version = if distro.version.len() > 10 {
                &distro.version[..10]
            } else {
                distro.version
            };
            screen.put_str_at(x + 22, y, version, fg, bg);

            // Size
            screen.put_str_at(x + 34, y, distro.size_str(), fg, bg);

            // Description (truncate)
            let desc_max = 30;
            let desc = if distro.description.len() > desc_max {
                &distro.description[..desc_max]
            } else {
                distro.description
            };
            screen.put_str_at(x + 48, y, desc, fg, bg);
        }

        // Scroll indicators
        if scroll_offset > 0 {
            screen.put_str_at(x + 76, y_start + 2, "^", EFI_GREEN, EFI_BLACK);
        }
        if visible_end < distros.len() {
            screen.put_str_at(x + 76, y_start + 1 + max_items, "v", EFI_GREEN, EFI_BLACK);
        }
    }

    /// Render distro details panel
    pub fn render_details(screen: &mut Screen, distro: &DistroEntry, y: usize) {
        morpheus_core::logger::log("Renderer: render_details()");
        let x = 2;

        // Box top
        screen.put_str_at(x, y, "┌─ Details ", EFI_GREEN, EFI_BLACK);
        for i in 12..78 {
            screen.put_str_at(x + i, y, "─", EFI_GREEN, EFI_BLACK);
        }
        screen.put_str_at(x + 78, y, "┐", EFI_GREEN, EFI_BLACK);

        // Details content
        let content_y = y + 1;
        screen.put_str_at(x, content_y, "│", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 2, content_y, "Name: ", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 8, content_y, distro.name, EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(x + 78, content_y, "│", EFI_GREEN, EFI_BLACK);

        let content_y = y + 2;
        screen.put_str_at(x, content_y, "│", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 2, content_y, "URL: ", EFI_DARKGREEN, EFI_BLACK);
        // Truncate URL for display
        let url_display = if distro.url.len() > 68 {
            &distro.url[..68]
        } else {
            distro.url
        };
        screen.put_str_at(x + 7, content_y, url_display, EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 78, content_y, "│", EFI_GREEN, EFI_BLACK);

        let content_y = y + 3;
        screen.put_str_at(x, content_y, "│", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 2, content_y, "Arch: ", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 8, content_y, distro.arch, EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 20, content_y, "Live: ", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(
            x + 26,
            content_y,
            if distro.is_live { "Yes" } else { "No" },
            EFI_GREEN,
            EFI_BLACK,
        );
        screen.put_str_at(x + 78, content_y, "│", EFI_GREEN, EFI_BLACK);

        // Box bottom
        let bottom_y = y + 4;
        screen.put_str_at(x, bottom_y, "└", EFI_GREEN, EFI_BLACK);
        for i in 1..78 {
            screen.put_str_at(x + i, bottom_y, "─", EFI_GREEN, EFI_BLACK);
        }
        screen.put_str_at(x + 78, bottom_y, "┘", EFI_GREEN, EFI_BLACK);
    }

    /// Render download progress
    pub fn render_progress(
        screen: &mut Screen,
        distro: &DistroEntry,
        bytes_downloaded: usize,
        total_bytes: Option<usize>,
        status: &str,
        y: usize,
    ) {
        morpheus_core::logger::log("Renderer: render_progress()");
        let x = 2;

        // Clear area
        for i in 0..6 {
            screen.put_str_at(
                x,
                y + i,
                &"                                                                                ",
                EFI_BLACK,
                EFI_BLACK,
            );
        }

        // Status message
        screen.put_str_at(x, y, "Downloading: ", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 13, y, distro.name, EFI_LIGHTGREEN, EFI_BLACK);

        // Progress bar
        let bar_width = 60;
        let progress = if let Some(total) = total_bytes {
            if total > 0 {
                (bytes_downloaded * 100) / total
            } else {
                0
            }
        } else {
            0
        };
        let filled = (bar_width * progress) / 100;

        let bar_y = y + 2;
        screen.put_str_at(x, bar_y, "[", EFI_GREEN, EFI_BLACK);
        for i in 0..bar_width {
            let ch = if i < filled {
                "="
            } else if i == filled {
                ">"
            } else {
                " "
            };
            screen.put_str_at(x + 1 + i, bar_y, ch, EFI_LIGHTGREEN, EFI_BLACK);
        }
        screen.put_str_at(x + 1 + bar_width, bar_y, "]", EFI_GREEN, EFI_BLACK);

        // Progress text
        let progress_text = if let Some(total) = total_bytes {
            let mb_downloaded = bytes_downloaded / (1024 * 1024);
            let mb_total = total / (1024 * 1024);
            // Simple formatting without alloc
            if progress < 100 {
                "Downloading..."
            } else {
                "Complete!"
            }
        } else {
            "Downloading..."
        };
        screen.put_str_at(
            x + bar_width + 4,
            bar_y,
            progress_text,
            EFI_GREEN,
            EFI_BLACK,
        );

        // Status line
        let status_y = y + 4;
        screen.put_str_at(x, status_y, "Status: ", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 8, status_y, status, EFI_GREEN, EFI_BLACK);
    }

    /// Render error message
    pub fn render_error(screen: &mut Screen, message: &str, y: usize) {
        morpheus_core::logger::log("Renderer: render_error()");
        let x = 2;
        screen.put_str_at(x, y, "ERROR: ", EFI_RED, EFI_BLACK);
        screen.put_str_at(x + 7, y, message, EFI_RED, EFI_BLACK);
    }

    /// Render footer with keybindings
    pub fn render_footer(screen: &mut Screen, y: usize) {
        morpheus_core::logger::log("Renderer: render_footer()");
        let x = 2;
        screen.put_str_at(x, y, "┌─ Controls ", EFI_GREEN, EFI_BLACK);
        for i in 13..78 {
            screen.put_str_at(x + i, y, "─", EFI_GREEN, EFI_BLACK);
        }
        screen.put_str_at(x + 78, y, "┐", EFI_GREEN, EFI_BLACK);

        let y = y + 1;
        screen.put_str_at(x, y, "│", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 2, y, "[UP/DOWN] Navigate", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 22, y, "[LEFT/RIGHT] Category", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 46, y, "[ENTER] Download", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 66, y, "[ESC] Back", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 78, y, "│", EFI_GREEN, EFI_BLACK);

        let y = y + 1;
        screen.put_str_at(x, y, "└", EFI_GREEN, EFI_BLACK);
        for i in 1..78 {
            screen.put_str_at(x + i, y, "─", EFI_GREEN, EFI_BLACK);
        }
        screen.put_str_at(x + 78, y, "┘", EFI_GREEN, EFI_BLACK);
    }

    /// Render success message
    pub fn render_success(screen: &mut Screen, message: &str, y: usize) {
        morpheus_core::logger::log("Renderer: render_success()");
        let x = 2;
        screen.put_str_at(x, y, "SUCCESS: ", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(x + 9, y, message, EFI_LIGHTGREEN, EFI_BLACK);
    }
}
