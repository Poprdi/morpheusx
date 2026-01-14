//! Rendering methods for the Distro Downloader UI.
//!
//! All screen rendering logic is centralized here for maintainability.

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;

use super::helpers::{
    format_size_mb, pad_or_truncate, CATEGORY_Y, DETAILS_Y, FOOTER_Y, HEADER_Y, LIST_Y,
    VISIBLE_ITEMS,
};
use crate::tui::distro_downloader::catalog::{DistroEntry, CATEGORIES};
use crate::tui::distro_downloader::state::{DownloadStatus, UiMode, UiState};
use crate::tui::renderer::{
    Screen, EFI_BLACK, EFI_DARKGRAY, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN, EFI_RED, EFI_WHITE,
    EFI_YELLOW,
};
use morpheus_core::iso::MAX_ISOS;

/// Rendering context for the distro downloader UI.
///
/// This struct holds references to all state needed for rendering,
/// allowing the render methods to be pure functions.
pub struct RenderContext<'a> {
    pub ui_state: &'a UiState,
    pub download_status: DownloadStatus,
    pub download_progress: usize,
    pub error_message: Option<&'static str>,
    pub current_distros: &'a [&'static DistroEntry],
    pub iso_names: &'a [[u8; 64]; MAX_ISOS],
    pub iso_name_lens: &'a [usize; MAX_ISOS],
    pub iso_sizes_mb: &'a [u64; MAX_ISOS],
    pub iso_complete: &'a [bool; MAX_ISOS],
}

impl<'a> RenderContext<'a> {
    /// Get the currently selected distro
    pub fn selected_distro(&self) -> Option<&'static DistroEntry> {
        self.current_distros
            .get(self.ui_state.selected_distro)
            .copied()
    }
}

/// Full render - clears screen if needed and draws everything
pub fn render_full(ctx: &RenderContext, screen: &mut Screen, needs_clear: bool) {
    if needs_clear {
        screen.clear();
    }

    match ctx.ui_state.mode {
        UiMode::Browse => {
            render_header(screen);
            render_categories(ctx, screen);
            render_list(ctx, screen);
            render_details(ctx, screen);
            render_footer(screen);
        }
        UiMode::Confirm => {
            render_header(screen);
            render_confirm_dialog(ctx, screen);
        }
        UiMode::Downloading => {
            render_header(screen);
            render_progress_only(ctx, screen);
        }
        UiMode::Result => {
            render_header(screen);
            render_result(ctx, screen);
        }
        UiMode::Manage => {
            render_manage_header(screen);
            render_iso_list(ctx, screen);
            render_manage_footer(screen);
        }
        UiMode::ConfirmDelete => {
            render_manage_header(screen);
            render_iso_list(ctx, screen);
            render_manage_confirm_dialog(ctx, screen, "Delete this ISO?");
        }
    }
}

/// Render only the list and details (for navigation - no clear needed)
pub fn render_list_and_details(ctx: &RenderContext, screen: &mut Screen) {
    render_list(ctx, screen);
    render_details(ctx, screen);
}

fn render_header(screen: &mut Screen) {
    let title = "=== DISTRO DOWNLOADER ===";
    let x = screen.center_x(title.len());
    screen.put_str_at(x, HEADER_Y, title, EFI_LIGHTGREEN, EFI_BLACK);

    let subtitle = "Download Linux distributions to ESP";
    let x = screen.center_x(subtitle.len());
    screen.put_str_at(x, HEADER_Y + 1, subtitle, EFI_DARKGREEN, EFI_BLACK);
}

fn render_categories(ctx: &RenderContext, screen: &mut Screen) {
    let x = 2;
    let y = CATEGORY_Y;
    let mut current_x = x;

    // Clear the category line
    screen.put_str_at(
        x,
        y,
        "                                                                              ",
        EFI_BLACK,
        EFI_BLACK,
    );

    screen.put_str_at(x, y, "Category: ", EFI_GREEN, EFI_BLACK);
    current_x += 10;

    for (i, cat) in CATEGORIES.iter().enumerate() {
        let name = cat.name();
        let (fg, bg) = if i == ctx.ui_state.selected_category {
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

fn render_list(ctx: &RenderContext, screen: &mut Screen) {
    let x = 2;
    let y = LIST_Y;

    // Column headers
    screen.put_str_at(x + 2, y, "Name              ", EFI_DARKGREEN, EFI_BLACK);
    screen.put_str_at(x + 22, y, "Version   ", EFI_DARKGREEN, EFI_BLACK);
    screen.put_str_at(x + 34, y, "Size         ", EFI_DARKGREEN, EFI_BLACK);
    screen.put_str_at(
        x + 48,
        y,
        "Description                   ",
        EFI_DARKGREEN,
        EFI_BLACK,
    );

    // Separator
    screen.put_str_at(
        x,
        y + 1,
        "--------------------------------------------------------------------------------",
        EFI_DARKGREEN,
        EFI_BLACK,
    );

    // Clear list area
    for row in 0..VISIBLE_ITEMS {
        screen.put_str_at(
            x,
            y + 2 + row,
            "                                                                                ",
            EFI_BLACK,
            EFI_BLACK,
        );
    }

    // Render visible items
    let scroll = ctx.ui_state.scroll_offset;
    let visible_end = (scroll + VISIBLE_ITEMS).min(ctx.current_distros.len());

    for (display_idx, list_idx) in (scroll..visible_end).enumerate() {
        let distro = ctx.current_distros[list_idx];
        let row_y = y + 2 + display_idx;
        let is_selected = list_idx == ctx.ui_state.selected_distro;

        let (fg, bg) = if is_selected {
            (EFI_BLACK, EFI_LIGHTGREEN)
        } else {
            (EFI_GREEN, EFI_BLACK)
        };

        // Selection indicator
        let marker = if is_selected { ">>" } else { "  " };
        screen.put_str_at(x, row_y, marker, EFI_LIGHTGREEN, EFI_BLACK);

        // Name (padded/truncated to 18 chars)
        let name = pad_or_truncate(distro.name, 18);
        screen.put_str_at(x + 2, row_y, &name, fg, bg);

        // Version (padded/truncated to 10 chars)
        let version = pad_or_truncate(distro.version, 10);
        screen.put_str_at(x + 22, row_y, &version, fg, bg);

        // Size
        let size = pad_or_truncate(distro.size_str(), 12);
        screen.put_str_at(x + 34, row_y, &size, fg, bg);

        // Description (truncated to 30 chars)
        let desc = pad_or_truncate(distro.description, 30);
        screen.put_str_at(x + 48, row_y, &desc, fg, bg);
    }

    // Scroll indicators
    if scroll > 0 {
        screen.put_str_at(x + 78, y + 2, "^", EFI_LIGHTGREEN, EFI_BLACK);
    } else {
        screen.put_str_at(x + 78, y + 2, " ", EFI_BLACK, EFI_BLACK);
    }
    if visible_end < ctx.current_distros.len() {
        screen.put_str_at(
            x + 78,
            y + 1 + VISIBLE_ITEMS,
            "v",
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );
    } else {
        screen.put_str_at(x + 78, y + 1 + VISIBLE_ITEMS, " ", EFI_BLACK, EFI_BLACK);
    }
}

fn render_details(ctx: &RenderContext, screen: &mut Screen) {
    let x = 2;
    let y = DETAILS_Y;

    // Clear details area
    for row in 0..4 {
        screen.put_str_at(
            x,
            y + row,
            "                                                                                ",
            EFI_BLACK,
            EFI_BLACK,
        );
    }

    if let Some(distro) = ctx.selected_distro() {
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
        screen.put_str_at(
            x + 56,
            y + 1,
            if distro.is_live { "Yes" } else { "No " },
            EFI_GREEN,
            EFI_BLACK,
        );
        screen.put_str_at(x + 78, y + 1, "|", EFI_GREEN, EFI_BLACK);

        // Content line 2 - URL
        screen.put_str_at(x, y + 2, "|", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 2, y + 2, "URL: ", EFI_DARKGREEN, EFI_BLACK);
        let url_display = if distro.url.len() > 70 {
            &distro.url[..70]
        } else {
            distro.url
        };
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

fn render_footer(screen: &mut Screen) {
    let x = 2;
    let y = FOOTER_Y;

    screen.put_str_at(x, y, "+-[ Controls ]", EFI_GREEN, EFI_BLACK);
    for i in 15..78 {
        screen.put_str_at(x + i, y, "-", EFI_GREEN, EFI_BLACK);
    }
    screen.put_str_at(x + 78, y, "+", EFI_GREEN, EFI_BLACK);

    screen.put_str_at(x, y + 1, "|", EFI_GREEN, EFI_BLACK);
    screen.put_str_at(x + 2, y + 1, "[Arrows] Nav", EFI_DARKGREEN, EFI_BLACK);
    screen.put_str_at(x + 17, y + 1, "[ENTER] Download", EFI_DARKGREEN, EFI_BLACK);
    screen.put_str_at(x + 37, y + 1, "[M] Manage ISOs", EFI_DARKGREEN, EFI_BLACK);
    screen.put_str_at(x + 56, y + 1, "[ESC] Back", EFI_DARKGREEN, EFI_BLACK);
    screen.put_str_at(x + 78, y + 1, "|", EFI_GREEN, EFI_BLACK);

    screen.put_str_at(x, y + 2, "+", EFI_GREEN, EFI_BLACK);
    for i in 1..78 {
        screen.put_str_at(x + i, y + 2, "-", EFI_GREEN, EFI_BLACK);
    }
    screen.put_str_at(x + 78, y + 2, "+", EFI_GREEN, EFI_BLACK);
}

fn render_confirm_dialog(ctx: &RenderContext, screen: &mut Screen) {
    if let Some(distro) = ctx.selected_distro() {
        let x = 10;
        let y = 8;

        // Dialog box using ASCII (more compatible than Unicode box chars)
        screen.put_str_at(
            x,
            y,
            "+--------------------------------------------------------+",
            EFI_GREEN,
            EFI_BLACK,
        );
        screen.put_str_at(
            x,
            y + 1,
            "|              CONFIRM DOWNLOAD                          |",
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );
        screen.put_str_at(
            x,
            y + 2,
            "+--------------------------------------------------------+",
            EFI_GREEN,
            EFI_BLACK,
        );
        screen.put_str_at(
            x,
            y + 3,
            "|                                                        |",
            EFI_GREEN,
            EFI_BLACK,
        );
        screen.put_str_at(
            x,
            y + 4,
            "|                                                        |",
            EFI_GREEN,
            EFI_BLACK,
        );
        screen.put_str_at(
            x,
            y + 5,
            "|                                                        |",
            EFI_GREEN,
            EFI_BLACK,
        );
        screen.put_str_at(
            x,
            y + 6,
            "+--------------------------------------------------------+",
            EFI_GREEN,
            EFI_BLACK,
        );
        screen.put_str_at(
            x,
            y + 7,
            "|     Download to /isos/ on ESP?    [Y]es   [N]o         |",
            EFI_GREEN,
            EFI_BLACK,
        );
        screen.put_str_at(
            x,
            y + 8,
            "+--------------------------------------------------------+",
            EFI_GREEN,
            EFI_BLACK,
        );

        // Content
        screen.put_str_at(x + 3, y + 3, "Distro: ", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 11, y + 3, distro.name, EFI_LIGHTGREEN, EFI_BLACK);

        screen.put_str_at(x + 3, y + 4, "Size:   ", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 11, y + 4, distro.size_str(), EFI_GREEN, EFI_BLACK);

        screen.put_str_at(x + 3, y + 5, "File:   ", EFI_DARKGREEN, EFI_BLACK);
        let filename = if distro.filename.len() > 40 {
            &distro.filename[..40]
        } else {
            distro.filename
        };
        screen.put_str_at(x + 11, y + 5, filename, EFI_GREEN, EFI_BLACK);
    }
}

fn render_progress_only(ctx: &RenderContext, screen: &mut Screen) {
    if let Some(distro) = ctx.selected_distro() {
        let x = 10;
        let y = 8;

        screen.put_str_at(x, y, "Downloading: ", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 13, y, distro.name, EFI_LIGHTGREEN, EFI_BLACK);

        // Progress bar
        let bar_width = 50;
        let progress = ctx.download_progress;
        let filled = (bar_width * progress) / 100;

        screen.put_str_at(x, y + 2, "[", EFI_GREEN, EFI_BLACK);
        for i in 0..bar_width {
            let ch = if i < filled {
                "="
            } else if i == filled {
                ">"
            } else {
                " "
            };
            screen.put_str_at(x + 1 + i, y + 2, ch, EFI_LIGHTGREEN, EFI_BLACK);
        }
        screen.put_str_at(x + 1 + bar_width, y + 2, "]", EFI_GREEN, EFI_BLACK);

        // Status
        screen.put_str_at(x, y + 4, "Status: ", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(
            x + 8,
            y + 4,
            ctx.download_status.as_str(),
            EFI_GREEN,
            EFI_BLACK,
        );
    }
}

fn render_result(ctx: &RenderContext, screen: &mut Screen) {
    let x = 10;
    let y = 10;

    if ctx.download_status == DownloadStatus::Complete {
        screen.put_str_at(x, y, "SUCCESS: ", EFI_LIGHTGREEN, EFI_BLACK);
        let msg = ctx.ui_state.status_message.unwrap_or("Download complete!");
        screen.put_str_at(x + 9, y, msg, EFI_LIGHTGREEN, EFI_BLACK);
    } else {
        screen.put_str_at(x, y, "FAILED: ", EFI_RED, EFI_BLACK);
        let msg = ctx.error_message.unwrap_or("Download failed");
        screen.put_str_at(x + 8, y, msg, EFI_RED, EFI_BLACK);
    }

    screen.put_str_at(
        x,
        y + 2,
        "Press any key to continue...",
        EFI_DARKGREEN,
        EFI_BLACK,
    );
}

// =========================================================================
// ISO Manager Rendering
// =========================================================================

fn render_manage_header(screen: &mut Screen) {
    let title = "=== ISO MANAGER ===";
    let x = screen.center_x(title.len());
    screen.put_str_at(x, HEADER_Y, title, EFI_LIGHTGREEN, EFI_BLACK);

    let subtitle = "Manage downloaded ISO images  |  Press [ESC] to return";
    let x = screen.center_x(subtitle.len());
    screen.put_str_at(x, HEADER_Y + 1, subtitle, EFI_DARKGREEN, EFI_BLACK);
}

fn render_iso_list(ctx: &RenderContext, screen: &mut Screen) {
    let x = 2;
    let y = 4;

    if ctx.ui_state.iso_count == 0 {
        screen.put_str_at(x, y, "No ISOs stored.", EFI_DARKGRAY, EFI_BLACK);
        screen.put_str_at(
            x,
            y + 1,
            "Download distros from the Browse view to see them here.",
            EFI_DARKGRAY,
            EFI_BLACK,
        );
        return;
    }

    // Column headers
    screen.put_str_at(
        x + 2,
        y,
        "NAME                                    ",
        EFI_DARKGREEN,
        EFI_BLACK,
    );
    screen.put_str_at(x + 44, y, "SIZE (MB)", EFI_DARKGREEN, EFI_BLACK);
    screen.put_str_at(x + 58, y, "STATUS", EFI_DARKGREEN, EFI_BLACK);

    // Separator
    screen.put_str_at(
        x,
        y + 1,
        "------------------------------------------------------------------------",
        EFI_DARKGREEN,
        EFI_BLACK,
    );

    // List ISOs
    for i in 0..ctx.ui_state.iso_count {
        let row_y = y + 2 + i;

        // Selection indicator
        if i == ctx.ui_state.selected_iso {
            screen.put_str_at(x, row_y, ">>", EFI_LIGHTGREEN, EFI_BLACK);
        } else {
            screen.put_str_at(x, row_y, "  ", EFI_BLACK, EFI_BLACK);
        }

        // Name (max 40 chars)
        let name = core::str::from_utf8(&ctx.iso_names[i][..ctx.iso_name_lens[i].min(40)])
            .unwrap_or("???");

        let (fg, bg) = if i == ctx.ui_state.selected_iso {
            (EFI_BLACK, EFI_GREEN)
        } else {
            (EFI_LIGHTGREEN, EFI_BLACK)
        };

        let name_padded = pad_or_truncate(name, 40);
        screen.put_str_at(x + 2, row_y, &name_padded, fg, bg);

        // Size
        let size_str = format_size_mb(ctx.iso_sizes_mb[i]);
        screen.put_str_at(x + 44, row_y, &size_str, EFI_GREEN, EFI_BLACK);

        // Status
        if ctx.iso_complete[i] {
            screen.put_str_at(x + 58, row_y, "Ready   ", EFI_GREEN, EFI_BLACK);
        } else {
            screen.put_str_at(x + 58, row_y, "Partial ", EFI_YELLOW, EFI_BLACK);
        }
    }
}

fn render_manage_footer(screen: &mut Screen) {
    let x = 2;
    let y = FOOTER_Y;

    screen.put_str_at(x, y, "+-[ Controls ]", EFI_GREEN, EFI_BLACK);
    for i in 15..70 {
        screen.put_str_at(x + i, y, "-", EFI_GREEN, EFI_BLACK);
    }
    screen.put_str_at(x + 70, y, "+", EFI_GREEN, EFI_BLACK);

    screen.put_str_at(x, y + 1, "|", EFI_GREEN, EFI_BLACK);
    screen.put_str_at(x + 2, y + 1, "[UP/DOWN] Select", EFI_DARKGREEN, EFI_BLACK);
    screen.put_str_at(x + 22, y + 1, "[D] Delete", EFI_DARKGREEN, EFI_BLACK);
    screen.put_str_at(x + 38, y + 1, "[R] Refresh", EFI_DARKGREEN, EFI_BLACK);
    screen.put_str_at(x + 54, y + 1, "[ESC] Back", EFI_DARKGREEN, EFI_BLACK);
    screen.put_str_at(x + 70, y + 1, "|", EFI_GREEN, EFI_BLACK);

    screen.put_str_at(x, y + 2, "+", EFI_GREEN, EFI_BLACK);
    for i in 1..70 {
        screen.put_str_at(x + i, y + 2, "-", EFI_GREEN, EFI_BLACK);
    }
    screen.put_str_at(x + 70, y + 2, "+", EFI_GREEN, EFI_BLACK);
}

fn render_manage_confirm_dialog(ctx: &RenderContext, screen: &mut Screen, message: &str) {
    let x = 15;
    let y = 10;

    // Get selected ISO name
    let name = if ctx.ui_state.selected_iso < ctx.ui_state.iso_count {
        core::str::from_utf8(
            &ctx.iso_names[ctx.ui_state.selected_iso]
                [..ctx.iso_name_lens[ctx.ui_state.selected_iso].min(40)],
        )
        .unwrap_or("???")
    } else {
        "???"
    };

    screen.put_str_at(
        x,
        y,
        "+--------------------------------------------------+",
        EFI_GREEN,
        EFI_BLACK,
    );
    screen.put_str_at(
        x,
        y + 1,
        "|                    CONFIRM                       |",
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );
    screen.put_str_at(
        x,
        y + 2,
        "+--------------------------------------------------+",
        EFI_GREEN,
        EFI_BLACK,
    );
    screen.put_str_at(
        x,
        y + 3,
        "|                                                  |",
        EFI_GREEN,
        EFI_BLACK,
    );
    screen.put_str_at(
        x,
        y + 4,
        "|                                                  |",
        EFI_GREEN,
        EFI_BLACK,
    );
    screen.put_str_at(
        x,
        y + 5,
        "+--------------------------------------------------+",
        EFI_GREEN,
        EFI_BLACK,
    );
    screen.put_str_at(
        x,
        y + 6,
        "|               [Y]es       [N]o                   |",
        EFI_GREEN,
        EFI_BLACK,
    );
    screen.put_str_at(
        x,
        y + 7,
        "+--------------------------------------------------+",
        EFI_GREEN,
        EFI_BLACK,
    );

    screen.put_str_at(x + 3, y + 3, message, EFI_WHITE, EFI_BLACK);
    screen.put_str_at(x + 3, y + 4, name, EFI_LIGHTGREEN, EFI_BLACK);
}
