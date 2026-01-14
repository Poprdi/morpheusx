// Bootloader installer menu UI

mod esp_creation;
mod esp_scan;
mod installation;

use crate::installer::EspInfo;
use crate::tui::input::Keyboard;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN};
use crate::BootServices;
use alloc::string::ToString;
use alloc::vec::Vec;

// Box constants for centered UI
const BOX_WIDTH: usize = 77;
const EMPTY_LINE: &str =
    "|                                                                           |";
const TOP_BORDER: &str =
    "+===========================================================================+";
const BOTTOM_BORDER: &str =
    "+===========================================================================+";
const DIVIDER: &str =
    "+---------------------------------------------------------------------------+";

// Header art
const HEADER_ART: &[&str] = &[
    " ___           _        _ _           ",
    "|_ _|_ __  ___| |_ __ _| | | ___ _ __ ",
    " | || '_ \\/ __| __/ _` | | |/ _ \\ '__|",
    " | || | | \\__ \\ || (_| | | |  __/ |   ",
    "|___|_| |_|___/\\__\\__,_|_|_|\\___|_|   ",
];

pub struct InstallerMenu {
    esp_list: Vec<EspInfo>,
    selected_esp: usize,
    scan_complete: bool,
    image_handle: *mut (),
}

impl InstallerMenu {
    pub fn new(image_handle: *mut ()) -> Self {
        Self {
            esp_list: Vec::new(),
            selected_esp: 0,
            scan_complete: false,
            image_handle,
        }
    }

    pub fn run(&mut self, screen: &mut Screen, keyboard: &mut Keyboard, bs: &BootServices) {
        loop {
            self.render(screen, bs);

            // Render global rain and check for input
            loop {
                crate::tui::rain::render_rain(screen);

                if let Some(key) = keyboard.read_key() {
                    // Global rain toggle
                    if key.unicode_char == b'x' as u16 || key.unicode_char == b'X' as u16 {
                        crate::tui::rain::toggle_rain(screen);
                        screen.clear();
                        self.render(screen, bs);
                        continue;
                    }

                    match key.scan_code {
                        0x01 => {
                            // Up arrow
                            if self.selected_esp > 0 {
                                self.selected_esp -= 1;
                            }
                        }
                        0x02 => {
                            // Down arrow
                            if self.selected_esp + 1 < self.esp_list.len() {
                                self.selected_esp += 1;
                            }
                        }
                        0x17 => {
                            // ESC
                            return;
                        }
                        _ => {
                            if key.unicode_char == b'\r' as u16 || key.unicode_char == b'\n' as u16
                            {
                                // Enter key - install to selected ESP
                                if !self.esp_list.is_empty()
                                    && self.selected_esp < self.esp_list.len()
                                {
                                    let esp = &self.esp_list[self.selected_esp];
                                    installation::install_to_selected(
                                        esp,
                                        screen,
                                        keyboard,
                                        bs,
                                        self.image_handle,
                                    );
                                }
                            } else if key.unicode_char == b'r' as u16
                                || key.unicode_char == b'R' as u16
                            {
                                // Rescan
                                self.scan_complete = false;
                            } else if key.unicode_char == b'c' as u16
                                || key.unicode_char == b'C' as u16
                            {
                                // Show help or create ESP
                                if self.esp_list.is_empty() {
                                    if let Some(new_esp) =
                                        esp_creation::create_new_esp(screen, keyboard, bs)
                                    {
                                        self.esp_list.push(new_esp);
                                        self.selected_esp = self.esp_list.len() - 1;
                                        self.scan_complete = false;
                                    }
                                } else {
                                    esp_creation::show_create_esp_help(screen, keyboard);
                                }
                            }
                        }
                    }
                    break;
                }
            }
        }
    }

    fn render(&mut self, screen: &mut Screen, bs: &BootServices) {
        screen.clear();

        // Calculate total height for centering
        let esp_count = if self.scan_complete {
            self.esp_list.len().max(1)
        } else {
            1
        };
        let total_height =
            1 + 1 + HEADER_ART.len() + 1 + 1 + 1 + 1 + 1 + 1 + esp_count + 6 + 1 + 1 + 1;

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

        // Empty line
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Divider
        screen.put_str_at(x, current_y, DIVIDER, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Empty line
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Scan for ESPs if not done yet
        if !self.scan_complete {
            screen.put_str_at(x, current_y, "|", EFI_GREEN, EFI_BLACK);
            let msg = "Scanning for EFI Partitions...";
            let padding = (75 - msg.len()) / 2;
            screen.put_str_at(x + 1 + padding, current_y, msg, EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x + 76, current_y, "|", EFI_GREEN, EFI_BLACK);
            current_y += 1;

            self.esp_list = esp_scan::scan_for_esps(bs);
            self.scan_complete = true;
        }

        if self.esp_list.is_empty() {
            self.render_no_esp_centered(screen, x, &mut current_y);
        } else {
            self.render_esp_list_centered(screen, x, &mut current_y);
        }

        // Empty line
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Bottom border
        screen.put_str_at(x, current_y, BOTTOM_BORDER, EFI_GREEN, EFI_BLACK);
    }

    fn render_no_esp_centered(&self, screen: &mut Screen, x: usize, current_y: &mut usize) {
        // No ESP message
        screen.put_str_at(x, *current_y, "|", EFI_GREEN, EFI_BLACK);
        let msg = "No EFI Partition found";
        let padding = (75 - msg.len()) / 2;
        screen.put_str_at(x + 1 + padding, *current_y, msg, EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(x + 76, *current_y, "|", EFI_GREEN, EFI_BLACK);
        *current_y += 1;

        // Empty line
        screen.put_str_at(x, *current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        *current_y += 1;

        // Options
        let options = [
            "[C] How to create new ESP partition",
            "[R] Rescan for ESPs",
            "[ESC] Back to Main Menu",
        ];

        for opt in options.iter() {
            screen.put_str_at(x, *current_y, "|", EFI_GREEN, EFI_BLACK);
            let padding = (75 - opt.len()) / 2;
            screen.put_str_at(x + 1 + padding, *current_y, opt, EFI_DARKGREEN, EFI_BLACK);
            screen.put_str_at(x + 76, *current_y, "|", EFI_GREEN, EFI_BLACK);
            *current_y += 1;
        }
    }

    fn render_esp_list_centered(&self, screen: &mut Screen, x: usize, current_y: &mut usize) {
        // Title
        screen.put_str_at(x, *current_y, "|", EFI_GREEN, EFI_BLACK);
        let title = "Found EFI Partitions:";
        let padding = (75 - title.len()) / 2;
        screen.put_str_at(
            x + 1 + padding,
            *current_y,
            title,
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );
        screen.put_str_at(x + 76, *current_y, "|", EFI_GREEN, EFI_BLACK);
        *current_y += 1;

        // Empty line
        screen.put_str_at(x, *current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        *current_y += 1;

        // Table header
        screen.put_str_at(x, *current_y, "|", EFI_GREEN, EFI_BLACK);
        let header = "DISK    PART    SIZE (MB)    STATUS";
        let padding = (75 - header.len()) / 2;
        screen.put_str_at(x + 1 + padding, *current_y, header, EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 76, *current_y, "|", EFI_GREEN, EFI_BLACK);
        *current_y += 1;

        // ESP entries
        for (idx, esp) in self.esp_list.iter().enumerate() {
            screen.put_str_at(x, *current_y, "|", EFI_GREEN, EFI_BLACK);

            let marker = if idx == self.selected_esp {
                ">> "
            } else {
                "   "
            };
            let entry = alloc::format!(
                "{}{}       {}       {}         Ready",
                marker,
                esp.disk_index,
                esp.partition_index,
                esp.size_mb
            );
            let padding = (75 - entry.len()) / 2;
            let color = if idx == self.selected_esp {
                EFI_LIGHTGREEN
            } else {
                EFI_GREEN
            };
            screen.put_str_at(x + 1 + padding, *current_y, &entry, color, EFI_BLACK);
            screen.put_str_at(x + 76, *current_y, "|", EFI_GREEN, EFI_BLACK);
            *current_y += 1;
        }

        // Empty line
        screen.put_str_at(x, *current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        *current_y += 1;

        // Divider
        screen.put_str_at(x, *current_y, DIVIDER, EFI_GREEN, EFI_BLACK);
        *current_y += 1;

        // Empty line
        screen.put_str_at(x, *current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        *current_y += 1;

        // Instructions
        screen.put_str_at(x, *current_y, "|", EFI_GREEN, EFI_BLACK);
        let instr = "[UP/DOWN] Select  |  [ENTER] Install  |  [R] Rescan  |  [ESC] Back";
        let padding = (75 - instr.len()) / 2;
        screen.put_str_at(x + 1 + padding, *current_y, instr, EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 76, *current_y, "|", EFI_GREEN, EFI_BLACK);
        *current_y += 1;
    }
}
