use super::entry::BootEntry;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN};

// Box constants
const BOX_WIDTH: usize = 77;
const EMPTY_LINE: &str = "|                                                                           |";
const TOP_BORDER: &str = "+===========================================================================+";
const BOTTOM_BORDER: &str = "+===========================================================================+";
const DIVIDER: &str = "+---------------------------------------------------------------------------+";

// Header art for distro launcher
const HEADER_ART: &[&str] = &[
    " ____  _     _               _                            _               ",
    "|  _ \\(_)___| |_ _ __ ___   | |    __ _ _   _ _ __   ___| |__   ___ _ __ ",
    "| | | | / __| __| '__/ _ \\  | |   / _` | | | | '_ \\ / __| '_ \\ / _ \\ '__|",
    "| |_| | \\__ \\ |_| | | (_) | | |__| (_| | |_| | | | | (__| | | |  __/ |   ",
    "|____/|_|___/\\__|_|  \\___/  |_____\\__,_|\\__,_|_| |_|\\___|_| |_|\\___|_|   ",
];

pub struct EntryRenderer;

impl EntryRenderer {
    pub fn render_header(screen: &mut Screen) {
        // Calculate total height for centering will be done in full render
    }

    pub fn render_entries(screen: &mut Screen, entries: &[BootEntry], selected: usize) {
        // Full centered render
        let total_height = 1 + 1 + HEADER_ART.len() + 1 + 1 + 1 + 1 + 1 + 1 + 
                          entries.len() + 1 + 1 + 1 + 1;
        
        let x = screen.center_x(BOX_WIDTH);
        let y = screen.center_y(total_height);
        
        let mut current_y = y;

        // Top border
        screen.put_str_at(x, current_y, TOP_BORDER, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Empty line
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Header art
        for line in HEADER_ART.iter() {
            screen.put_str_at(x, current_y, "|", EFI_GREEN, EFI_BLACK);
            let padding = (75 - line.len()) / 2;
            screen.put_str_at(x + 1 + padding, current_y, line, EFI_LIGHTGREEN, EFI_BLACK);
            screen.put_str_at(x + 76, current_y, "|", EFI_GREEN, EFI_BLACK);
            current_y += 1;
        }

        // Empty line after header
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Divider
        screen.put_str_at(x, current_y, DIVIDER, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Empty line
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Instructions
        screen.put_str_at(x, current_y, "|", EFI_GREEN, EFI_BLACK);
        let instr = "[UP/DOWN] Navigate  |  [ENTER] Boot  |  [ESC] Back";
        let instr_padding = (75 - instr.len()) / 2;
        screen.put_str_at(x + 1 + instr_padding, current_y, instr, EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 76, current_y, "|", EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Empty line
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Divider
        screen.put_str_at(x, current_y, DIVIDER, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Entries
        if entries.is_empty() {
            screen.put_str_at(x, current_y, "|", EFI_GREEN, EFI_BLACK);
            let msg = "No bootable entries found";
            let msg_padding = (75 - msg.len()) / 2;
            screen.put_str_at(x + 1 + msg_padding, current_y, msg, EFI_DARKGREEN, EFI_BLACK);
            screen.put_str_at(x + 76, current_y, "|", EFI_GREEN, EFI_BLACK);
            current_y += 1;
        } else {
            for (i, entry) in entries.iter().enumerate() {
                screen.put_str_at(x, current_y, "|", EFI_GREEN, EFI_BLACK);
                
                let marker = if i == selected { ">> " } else { "   " };
                let entry_text = alloc::format!("{}{}", marker, entry.name);
                let entry_padding = (75 - entry_text.len()) / 2;
                
                let color = if i == selected { EFI_LIGHTGREEN } else { EFI_GREEN };
                screen.put_str_at(x + 1 + entry_padding, current_y, &entry_text, color, EFI_BLACK);
                screen.put_str_at(x + 76, current_y, "|", EFI_GREEN, EFI_BLACK);
                current_y += 1;
            }
        }

        // Divider
        screen.put_str_at(x, current_y, DIVIDER, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Empty line
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Footer
        screen.put_str_at(x, current_y, "|", EFI_GREEN, EFI_BLACK);
        let footer = "Entries auto-discovered from ESP partition";
        let footer_padding = (75 - footer.len()) / 2;
        screen.put_str_at(x + 1 + footer_padding, current_y, footer, EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 76, current_y, "|", EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Empty line
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Bottom border
        screen.put_str_at(x, current_y, BOTTOM_BORDER, EFI_GREEN, EFI_BLACK);
    }

    pub fn render_footer(screen: &mut Screen) {
        // Footer is now rendered in render_entries for proper centering
    }
}
