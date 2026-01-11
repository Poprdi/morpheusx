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

        let border_width = 80;
        let start_x = screen.center_x(border_width);
        let start_y = 2;

        // Title
        screen.put_str_at(
            start_x,
            start_y,
            "=== Persistance Installer ===",
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );

        // Scan for ESPs if not done yet
        if !self.scan_complete {
            screen.put_str_at(
                start_x,
                start_y + 2,
                "Scanning for EFI Partitions...",
                EFI_GREEN,
                EFI_BLACK,
            );
            self.esp_list = esp_scan::scan_for_esps(bs);
            self.scan_complete = true;
        }

        let mut y = start_y + 2;

        if self.esp_list.is_empty() {
            self.render_no_esp(screen, start_x, y);
        } else {
            self.render_esp_list(screen, start_x, &mut y);
        }

        y += 2;
        screen.put_str_at(
            start_x,
            y,
            "[ESC] Back to Main Menu",
            EFI_DARKGREEN,
            EFI_BLACK,
        );
    }

    fn render_no_esp(&self, screen: &mut Screen, start_x: usize, mut y: usize) {
        screen.put_str_at(
            start_x,
            y,
            "No EFI Partition found",
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );
        y += 2;
        screen.put_str_at(start_x, y, "You can:", EFI_GREEN, EFI_BLACK);
        y += 1;
        screen.put_str_at(
            start_x,
            y,
            "  [C] How to create new ESP partition",
            EFI_DARKGREEN,
            EFI_BLACK,
        );
        y += 1;
        screen.put_str_at(
            start_x,
            y,
            "  [R] Rescan for ESPs",
            EFI_DARKGREEN,
            EFI_BLACK,
        );
    }

    fn render_esp_list(&self, screen: &mut Screen, start_x: usize, y: &mut usize) {
        screen.put_str_at(
            start_x,
            *y,
            "Found EFI Partitions:",
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );
        *y += 2;

        // Table header
        screen.put_str_at(
            start_x,
            *y,
            "   DISK    PART    SIZE (MB)    STATUS",
            EFI_GREEN,
            EFI_BLACK,
        );
        *y += 1;
        screen.put_str_at(
            start_x,
            *y,
            "========================================",
            EFI_GREEN,
            EFI_BLACK,
        );
        *y += 1;

        // ESP entries
        for (idx, esp) in self.esp_list.iter().enumerate() {
            self.render_esp_entry(screen, start_x, y, idx, esp);
        }

        *y += 1;
        screen.put_str_at(start_x, *y, "Options:", EFI_GREEN, EFI_BLACK);
        *y += 1;
        screen.put_str_at(
            start_x,
            *y,
            "  [UP/DOWN] Select ESP",
            EFI_DARKGREEN,
            EFI_BLACK,
        );
        *y += 1;
        screen.put_str_at(
            start_x,
            *y,
            "  [ENTER] Install to selected ESP",
            EFI_DARKGREEN,
            EFI_BLACK,
        );
        *y += 1;
        screen.put_str_at(
            start_x,
            *y,
            "  [C] How to create new ESP partition",
            EFI_DARKGREEN,
            EFI_BLACK,
        );
        *y += 1;
        screen.put_str_at(
            start_x,
            *y,
            "  [R] Rescan for ESPs",
            EFI_DARKGREEN,
            EFI_BLACK,
        );
    }

    fn render_esp_entry(
        &self,
        screen: &mut Screen,
        start_x: usize,
        y: &mut usize,
        idx: usize,
        esp: &EspInfo,
    ) {
        let marker = if idx == self.selected_esp { "> " } else { "  " };

        let disk_str = esp.disk_index.to_string();
        let part_str = esp.partition_index.to_string();
        let size_str = esp.size_mb.to_string();

        let mut line = marker.to_string();
        line.push_str(&disk_str);

        // Pad to column 2 (PART)
        while line.len() < 9 {
            line.push(' ');
        }
        line.push_str(&part_str);

        // Pad to column 3 (SIZE)
        while line.len() < 17 {
            line.push(' ');
        }
        line.push_str(&size_str);

        // Pad to column 4 (STATUS)
        while line.len() < 30 {
            line.push(' ');
        }
        line.push_str("Ready");

        let fg = if idx == self.selected_esp {
            EFI_LIGHTGREEN
        } else {
            EFI_GREEN
        };
        screen.put_str_at(start_x, *y, &line, fg, EFI_BLACK);
        *y += 1;
    }
}
