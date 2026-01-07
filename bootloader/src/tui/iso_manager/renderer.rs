//! ISO Manager Renderer
//!
//! Renders the ISO manager TUI components.

use super::state::{IsoManagerState, ViewMode};
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_GREEN, EFI_LIGHTGREEN, EFI_DARKGRAY, EFI_WHITE, EFI_RED, EFI_YELLOW};

/// Box drawing characters (ASCII fallback for UEFI)
const BOX_H: char = '-';
const BOX_V: char = '|';
const BOX_TL: char = '+';
const BOX_TR: char = '+';
const BOX_BL: char = '+';
const BOX_BR: char = '+';

/// Render the ISO manager UI
pub fn render(screen: &mut Screen, state: &IsoManagerState) {
    render_header(screen);
    
    match state.mode {
        ViewMode::List => render_list(screen, state),
        ViewMode::Details => render_details(screen, state),
        ViewMode::ConfirmDelete => {
            render_list(screen, state);
            render_confirm_dialog(screen, "Delete ISO?", state.selected_name());
        }
        ViewMode::ConfirmBoot => {
            render_list(screen, state);
            render_confirm_dialog(screen, "Boot ISO?", state.selected_name());
        }
    }

    render_footer(screen, state);
}

fn render_header(screen: &mut Screen) {
    let width = screen.width();
    
    // Title bar
    screen.set_cursor(0, 0);
    screen.set_colors(EFI_BLACK, EFI_GREEN);
    
    let title = " ISO MANAGER ";
    let padding = (width - title.len()) / 2;
    
    for _ in 0..padding {
        screen.print_char(' ');
    }
    screen.print(title);
    for _ in 0..(width - padding - title.len()) {
        screen.print_char(' ');
    }

    // Subtitle
    screen.set_cursor(0, 1);
    screen.set_colors(EFI_LIGHTGREEN, EFI_BLACK);
    screen.print("  Manage downloaded ISO images");
    
    // Separator
    screen.set_cursor(0, 2);
    screen.set_colors(EFI_DARKGRAY, EFI_BLACK);
    for _ in 0..width {
        screen.print_char(BOX_H);
    }
}

fn render_list(screen: &mut Screen, state: &IsoManagerState) {
    let start_row = 4;
    
    if state.count == 0 {
        screen.set_cursor(2, start_row);
        screen.set_colors(EFI_DARKGRAY, EFI_BLACK);
        screen.print("No ISOs stored. Use Distro Downloader to download ISOs.");
        return;
    }

    // Column headers
    screen.set_cursor(2, start_row);
    screen.set_colors(EFI_GREEN, EFI_BLACK);
    screen.print("  NAME                                    SIZE      CHUNKS  STATUS");
    
    screen.set_cursor(2, start_row + 1);
    screen.set_colors(EFI_DARKGRAY, EFI_BLACK);
    for _ in 0..70 {
        screen.print_char(BOX_H);
    }

    // List ISOs
    for i in 0..state.count {
        let row = start_row + 2 + i;
        screen.set_cursor(2, row);

        // Selection indicator
        if i == state.selected {
            screen.set_colors(EFI_BLACK, EFI_GREEN);
            screen.print("> ");
        } else {
            screen.set_colors(EFI_LIGHTGREEN, EFI_BLACK);
            screen.print("  ");
        }

        // Name (max 40 chars)
        let name = core::str::from_utf8(&state.names[i][..state.name_lens[i].min(40)])
            .unwrap_or("???");
        screen.print(name);
        
        // Padding after name
        for _ in name.len()..40 {
            screen.print_char(' ');
        }

        // Size
        screen.print("  ");
        print_size_mb(screen, state.sizes_mb[i]);
        
        // Chunks
        screen.print("   ");
        let chunks = state.chunk_counts[i];
        if chunks < 10 {
            screen.print_char(' ');
        }
        print_number(screen, chunks as u64);
        screen.print("     ");

        // Status
        if state.complete[i] {
            screen.set_colors(EFI_GREEN, EFI_BLACK);
            screen.print("Ready");
        } else {
            screen.set_colors(EFI_YELLOW, EFI_BLACK);
            screen.print("Incomplete");
        }

        // Reset selection highlighting
        if i == state.selected {
            screen.set_colors(EFI_LIGHTGREEN, EFI_BLACK);
        }
    }
}

fn render_details(screen: &mut Screen, state: &IsoManagerState) {
    let start_row = 4;
    
    if state.count == 0 {
        return;
    }

    // Title
    screen.set_cursor(2, start_row);
    screen.set_colors(EFI_GREEN, EFI_BLACK);
    screen.print("ISO Details");

    // Box around details
    let box_left = 2;
    let box_width = 60;
    let box_top = start_row + 1;
    
    // Top border
    screen.set_cursor(box_left, box_top);
    screen.set_colors(EFI_DARKGRAY, EFI_BLACK);
    screen.print_char(BOX_TL);
    for _ in 0..box_width - 2 {
        screen.print_char(BOX_H);
    }
    screen.print_char(BOX_TR);

    // Name row
    screen.set_cursor(box_left, box_top + 1);
    screen.print_char(BOX_V);
    screen.set_colors(EFI_LIGHTGREEN, EFI_BLACK);
    screen.print(" Name: ");
    screen.set_colors(EFI_WHITE, EFI_BLACK);
    let name = state.selected_name();
    screen.print(name);
    for _ in (7 + name.len())..box_width - 1 {
        screen.print_char(' ');
    }
    screen.set_colors(EFI_DARKGRAY, EFI_BLACK);
    screen.print_char(BOX_V);

    // Size row
    screen.set_cursor(box_left, box_top + 2);
    screen.print_char(BOX_V);
    screen.set_colors(EFI_LIGHTGREEN, EFI_BLACK);
    screen.print(" Size: ");
    screen.set_colors(EFI_WHITE, EFI_BLACK);
    print_size_mb(screen, state.selected_size_mb());
    screen.print(" MB");
    for _ in 0..35 {
        screen.print_char(' ');
    }
    screen.set_colors(EFI_DARKGRAY, EFI_BLACK);
    screen.print_char(BOX_V);

    // Chunks row
    screen.set_cursor(box_left, box_top + 3);
    screen.print_char(BOX_V);
    screen.set_colors(EFI_LIGHTGREEN, EFI_BLACK);
    screen.print(" Chunks: ");
    screen.set_colors(EFI_WHITE, EFI_BLACK);
    print_number(screen, state.selected_chunks() as u64);
    screen.print(" partitions");
    for _ in 0..34 {
        screen.print_char(' ');
    }
    screen.set_colors(EFI_DARKGRAY, EFI_BLACK);
    screen.print_char(BOX_V);

    // Status row
    screen.set_cursor(box_left, box_top + 4);
    screen.print_char(BOX_V);
    screen.set_colors(EFI_LIGHTGREEN, EFI_BLACK);
    screen.print(" Status: ");
    if state.selected_complete() {
        screen.set_colors(EFI_GREEN, EFI_BLACK);
        screen.print("Ready to boot");
    } else {
        screen.set_colors(EFI_YELLOW, EFI_BLACK);
        screen.print("Download incomplete");
    }
    for _ in 0..30 {
        screen.print_char(' ');
    }
    screen.set_colors(EFI_DARKGRAY, EFI_BLACK);
    screen.print_char(BOX_V);

    // Bottom border
    screen.set_cursor(box_left, box_top + 5);
    screen.print_char(BOX_BL);
    for _ in 0..box_width - 2 {
        screen.print_char(BOX_H);
    }
    screen.print_char(BOX_BR);

    // Actions hint
    screen.set_cursor(box_left, box_top + 7);
    screen.set_colors(EFI_DARKGRAY, EFI_BLACK);
    if state.selected_complete() {
        screen.print("[B] Boot   [D] Delete   [ESC] Back");
    } else {
        screen.print("[D] Delete   [ESC] Back");
    }
}

fn render_confirm_dialog(screen: &mut Screen, title: &str, item_name: &str) {
    let width = screen.width();
    let height = screen.height();
    
    let dialog_width = 50;
    let dialog_height = 7;
    let left = (width - dialog_width) / 2;
    let top = (height - dialog_height) / 2;

    // Shadow
    screen.set_colors(EFI_BLACK, EFI_DARKGRAY);
    for row in 1..=dialog_height {
        screen.set_cursor(left + dialog_width, top + row);
        screen.print_char(' ');
    }
    screen.set_cursor(left + 1, top + dialog_height);
    for _ in 0..dialog_width {
        screen.print_char(' ');
    }

    // Dialog background
    screen.set_colors(EFI_WHITE, EFI_RED);
    for row in 0..dialog_height {
        screen.set_cursor(left, top + row);
        for _ in 0..dialog_width {
            screen.print_char(' ');
        }
    }

    // Title
    screen.set_cursor(left + 2, top + 1);
    screen.set_colors(EFI_WHITE, EFI_RED);
    screen.print(title);

    // Item name
    screen.set_cursor(left + 2, top + 3);
    let max_name = item_name.len().min(dialog_width - 4);
    screen.print(&item_name[..max_name]);

    // Buttons
    screen.set_cursor(left + 2, top + 5);
    screen.set_colors(EFI_BLACK, EFI_WHITE);
    screen.print(" [Y]es ");
    screen.set_colors(EFI_WHITE, EFI_RED);
    screen.print("  ");
    screen.set_colors(EFI_BLACK, EFI_WHITE);
    screen.print(" [N]o ");
}

fn render_footer(screen: &mut Screen, state: &IsoManagerState) {
    let height = screen.height();
    let width = screen.width();

    // Error message (if any)
    if let Some(msg) = state.error_msg {
        screen.set_cursor(2, height - 3);
        screen.set_colors(EFI_RED, EFI_BLACK);
        screen.print("Error: ");
        screen.print(msg);
    }

    // Separator
    screen.set_cursor(0, height - 2);
    screen.set_colors(EFI_DARKGRAY, EFI_BLACK);
    for _ in 0..width {
        screen.print_char(BOX_H);
    }

    // Help text
    screen.set_cursor(2, height - 1);
    screen.set_colors(EFI_DARKGRAY, EFI_BLACK);
    
    match state.mode {
        ViewMode::List => {
            if state.count > 0 {
                screen.print("[UP/DOWN] Select  [ENTER] Details  [B] Boot  [D] Delete  [R] Refresh  [ESC] Back");
            } else {
                screen.print("[ESC] Back to main menu");
            }
        }
        ViewMode::Details => {
            screen.print("[B] Boot  [D] Delete  [ESC] Back to list");
        }
        _ => {}
    }
}

/// Print a number (no heap allocation)
fn print_number(screen: &mut Screen, n: u64) {
    if n == 0 {
        screen.print_char('0');
        return;
    }

    let mut buf = [0u8; 20];
    let mut i = 0;
    let mut val = n;
    
    while val > 0 {
        buf[i] = b'0' + (val % 10) as u8;
        val /= 10;
        i += 1;
    }

    while i > 0 {
        i -= 1;
        screen.print_char(buf[i] as char);
    }
}

/// Print size in MB with formatting
fn print_size_mb(screen: &mut Screen, mb: u64) {
    if mb >= 1024 {
        // Show in GB
        let gb = mb / 1024;
        let frac = (mb % 1024) / 100; // One decimal place
        print_number(screen, gb);
        screen.print_char('.');
        print_number(screen, frac);
        screen.print(" GB");
    } else {
        print_number(screen, mb);
        screen.print(" MB");
    }
}
