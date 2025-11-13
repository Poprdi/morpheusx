use crate::tui::renderer::{Screen, EFI_GREEN, EFI_LIGHTGREEN, EFI_BLACK, EFI_DARKGREEN};
use crate::tui::input::{Keyboard, InputKey};
use crate::tui::rain::MatrixRain;

const HEADER_ART: &[&str] = &[
    "  _____ _____ _____ _____ _   _ _____ _   _ _____ ",
    " |     |     | __  |  _  |   | |   __| | | |   __|",
    " | | | |  |  |    -|   __|     |   __| |_| |__   |",
    " |_|_|_|_____|__|__|__|  |__|__|_____|\\___|_____|",
    "      C O N T R O L   C E N T E R   v 0 . 1       ",
];

const SIDE_BORDER_LEFT: &str = "|";
const SIDE_BORDER_RIGHT: &str = "|";
const TOP_BORDER: &str = "+===========================================================================+";
const BOTTOM_BORDER: &str = "+===========================================================================+";
const DIVIDER: &str = "+===========================================================================+";

pub struct MainMenu {
    selected_index: usize,
    menu_items: [MenuItem; 6],
    rain: MatrixRain,
}

pub struct MenuItem {
    pub label: &'static str,
    pub description: &'static str,
    pub icon: &'static str,
}

impl MainMenu {
    pub fn new(screen: &Screen) -> Self {
        Self {
            selected_index: 0,
            rain: MatrixRain::new(screen.width(), screen.height()),
            menu_items: [
                MenuItem {
                    label: "Distro Launcher",
                    description: "Boot into ephemeral Linux distribution",
                    icon: "[>>]",
                },
                MenuItem {
                    label: "Distro Downloader",
                    description: "Download and manage distro templates",
                    icon: "[DN]",
                },
                MenuItem {
                    label: "Storage Manager",
                    description: "Manage partitions and overlay filesystems",
                    icon: "[FS]",
                },
                MenuItem {
                    label: "System Settings",
                    description: "Configure bootloader and system preferences",
                    icon: "[CFG]",
                },
                MenuItem {
                    label: "Admin Functions",
                    description: "Advanced operations and diagnostics",
                    icon: "[ADM]",
                },
                MenuItem {
                    label: "Exit to Firmware",
                    description: "Return to UEFI boot menu",
                    icon: "[EXT]",
                },
            ],
        }
    }

    pub fn select_next(&mut self) {
        if self.selected_index < self.menu_items.len() - 1 {
            self.selected_index += 1;
        }
    }

    pub fn select_prev(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn render(&mut self, screen: &mut Screen) {
        // Calculate centered position using screen helpers
        let border_width = 77;
        let x = screen.center_x(border_width);
        let y = 0;
        
        // Draw top border
        // Calculate centered X position based on border width (77 chars)
        let border_width = 77;
        let x = if screen.width() > border_width {
            (screen.width() - border_width) / 2
        } else {
            0
        };
        let y = 0;
        
        // Draw top border
        screen.put_str_at(x, y, TOP_BORDER, EFI_GREEN, EFI_BLACK);
        
        // Draw header ASCII art centered within border
        for (i, line) in HEADER_ART.iter().enumerate() {
            let art_y = y + 1 + i;
            let padding = (76 - line.len()) / 2;
            
            screen.put_str_at(x, art_y, SIDE_BORDER_LEFT, EFI_GREEN, EFI_BLACK);
            
            // Center the art
            let art_x = x + 1 + padding;
            screen.put_str_at(art_x, art_y, line, EFI_LIGHTGREEN, EFI_BLACK);
            
            screen.put_str_at(x + 75, art_y, SIDE_BORDER_RIGHT, EFI_GREEN, EFI_BLACK);
        }
        
        // Divider after header
        let divider_y = y + 1 + HEADER_ART.len();
        screen.put_str_at(x, divider_y, DIVIDER, EFI_GREEN, EFI_BLACK);
        
        // Instructions
        let instr_y = divider_y + 1;
        screen.put_str_at(x, instr_y, SIDE_BORDER_LEFT, EFI_GREEN, EFI_BLACK);
        let instr = "  [UP/DOWN] Navigate  |  [ENTER] Select  |  [ESC] Exit";
        let instr_x = x + (76 - instr.len()) / 2;
        screen.put_str_at(instr_x, instr_y, instr, EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 75, instr_y, SIDE_BORDER_RIGHT, EFI_GREEN, EFI_BLACK);
        
        // Another divider
        let menu_divider_y = instr_y + 1;
        screen.put_str_at(x, menu_divider_y, DIVIDER, EFI_GREEN, EFI_BLACK);
        
        // Menu items - each takes 2 lines
        let menu_start_y = menu_divider_y + 2;
        for (i, item) in self.menu_items.iter().enumerate() {
            let item_y = menu_start_y + i * 2;
            
            // Left border
            screen.put_str_at(x, item_y, SIDE_BORDER_LEFT, EFI_GREEN, EFI_BLACK);
            
            if i == self.selected_index {
                // Selection bracket and icon
                screen.put_str_at(x + 3, item_y, ">>", EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(x + 6, item_y, item.icon, EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(x + 12, item_y, item.label, EFI_LIGHTGREEN, EFI_BLACK);
                screen.put_str_at(x + 32, item_y, "-", EFI_GREEN, EFI_BLACK);
                screen.put_str_at(x + 34, item_y, item.description, EFI_GREEN, EFI_BLACK);
            } else {
                // Unselected item
                screen.put_str_at(x + 6, item_y, item.icon, EFI_DARKGREEN, EFI_BLACK);
                screen.put_str_at(x + 12, item_y, item.label, EFI_GREEN, EFI_BLACK);
            }
            
            // Right border
            screen.put_str_at(x + 75, item_y, SIDE_BORDER_RIGHT, EFI_GREEN, EFI_BLACK);
        }
        
        // Bottom divider
        let bottom_div_y = menu_start_y + (self.menu_items.len() * 2) + 1;
        screen.put_str_at(x, bottom_div_y, DIVIDER, EFI_GREEN, EFI_BLACK);
        
        // Status bar
        let status_y = bottom_div_y + 1;
        screen.put_str_at(x, status_y, SIDE_BORDER_LEFT, EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 3, status_y, "Status: READY", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(x + 20, status_y, "|", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 22, status_y, "System: Operational", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 75, status_y, SIDE_BORDER_RIGHT, EFI_GREEN, EFI_BLACK);
        
        // Bottom border
        screen.put_str_at(x, status_y + 1, BOTTOM_BORDER, EFI_GREEN, EFI_BLACK);
    }

    pub fn handle_input(&mut self, key: &InputKey) -> MenuAction {
        // Arrow up
        if key.scan_code == 0x01 {
            self.select_prev();
            return MenuAction::Navigate;
        }
        
        // Arrow down
        if key.scan_code == 0x02 {
            self.select_next();
            return MenuAction::Navigate;
        }
        
        // Enter key
        if key.unicode_char == 0x0D {
            return match self.selected_index {
                0 => MenuAction::DistroLauncher,
                1 => MenuAction::DistroDownloader,
                2 => MenuAction::StorageManager,
                3 => MenuAction::SystemSettings,
                4 => MenuAction::AdminFunctions,
                5 => MenuAction::ExitToFirmware,
                _ => MenuAction::Navigate,
            };
        }
        
        // ESC key
        if key.scan_code == 0x17 {
            return MenuAction::ExitToFirmware;
        }
        
        MenuAction::Navigate
    }

    pub fn run(&mut self, screen: &mut Screen, keyboard: &mut Keyboard) -> MenuAction {
        // Initial render
        screen.clear();
        self.render(screen);
        
        loop {
            // Animate rain in background
            self.rain.render_frame(screen);
            
            // Check for input (non-blocking)
            if let Some(key) = keyboard.read_key() {
                let action = self.handle_input(&key);
                if !matches!(action, MenuAction::Navigate) {
                    return action;
                }
                
                // Re-render UI after navigation
                screen.clear();
                self.render(screen);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MenuAction {
    Navigate,
    DistroLauncher,
    DistroDownloader,
    StorageManager,
    SystemSettings,
    AdminFunctions,
    ExitToFirmware,
}
