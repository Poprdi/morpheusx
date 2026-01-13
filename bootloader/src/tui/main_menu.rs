use crate::tui::debug::DebugOverlay;
use crate::tui::input::{InputKey, Keyboard};
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_CYAN, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN};

// Smaller header that fits in the box
const HEADER_ART: &[&str] = &[
    " __  __  ___  ____  ____  _   _ _____ _   _ ______  __",
    "|  \\/  |/ _ \\|  _ \\|  _ \\| | | | ____| | | / ___\\ \\/ /",
    "| |\\/| | | | | |_) | |_) | |_| |  _| | | | \\___ \\\\  / ",
    "| |  | | |_| |  _ <|  __/|  _  | |___| |_| |___) /  \\ ",
    "|_|  |_|\\___/|_| \\_\\_|   |_| |_|_____|\\___/|____/_/\\_\\",
];

// Box width (inner content is 75 chars)
const BOX_WIDTH: usize = 77;
const EMPTY_LINE: &str = "|                                                                           |";
const TOP_BORDER: &str = "+===========================================================================+";
const BOTTOM_BORDER: &str = "+===========================================================================+";
const DIVIDER: &str = "+---------------------------------------------------------------------------+";

pub struct MainMenu {
    selected_index: usize,
    menu_items: [MenuItem; 5],
    debug: DebugOverlay,
}

pub struct MenuItem {
    pub label: &'static str,
    pub description: &'static str,
    pub icon: &'static str,
}

impl MainMenu {
    pub fn new(_screen: &Screen) -> Self {
        Self {
            selected_index: 0,
            debug: DebugOverlay::new(),
            menu_items:[
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
                    label: "Installation",
                    description: "Persists the bootloader to disk",
                    icon: "[INS]",
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
        // Calculate total menu height
        // Top border (1) + empty (1) + header art (5) + empty (1) + divider (1) + 
        // empty (1) + instructions (1) + empty (1) + divider (1) +
        // menu items (6 * 2 = 12) + empty lines between (5) + 
        // divider (1) + empty (1) + status (1) + empty (1) + bottom border (1) = ~34
        let total_height = 1 + 1 + HEADER_ART.len() + 1 + 1 + 1 + 1 + 1 + 1 + 
                          (self.menu_items.len() * 2 - 1) + 1 + 1 + 1 + 1 + 1;
        
        // Center horizontally and vertically
        let x = screen.center_x(BOX_WIDTH);
        let y = screen.center_y(total_height);

        let mut current_y = y;

        // Draw top border
        screen.put_str_at(x, current_y, TOP_BORDER, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Empty line after top border
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Draw header ASCII art centered within border
        for line in HEADER_ART.iter() {
            // Draw left border
            screen.put_str_at(x, current_y, "|", EFI_GREEN, EFI_BLACK);
            
            // Center the art within the box (75 inner width)
            let padding = (75 - line.len()) / 2;
            let art_x = x + 1 + padding;
            screen.put_str_at(art_x, current_y, line, EFI_DARKGREEN, EFI_BLACK);
            
            // Draw right border
            screen.put_str_at(x + 76, current_y, "|", EFI_GREEN, EFI_BLACK);
            current_y += 1;
        }

        // Empty line after header
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Divider after header
        screen.put_str_at(x, current_y, DIVIDER, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Empty line before instructions
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Instructions line
        screen.put_str_at(x, current_y, "|", EFI_GREEN, EFI_BLACK);
        let instr = "[UP/DOWN] Navigate  |  [ENTER] Select  |  [ESC] Exit";
        let instr_padding = (75 - instr.len()) / 2;
        screen.put_str_at(x + 1 + instr_padding, current_y, instr, EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 76, current_y, "|", EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Empty line after instructions
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Divider before menu
        screen.put_str_at(x, current_y, DIVIDER, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Menu items
        for (i, item) in self.menu_items.iter().enumerate() {
            // Draw item line with borders
            screen.put_str_at(x, current_y, "|", EFI_GREEN, EFI_BLACK);

            // Build the menu item string: ">> [ICN] Label" or "   [ICN] Label"
            let item_text = if i == self.selected_index {
                alloc::format!(">> {} {}", item.icon, item.label)
            } else {
                alloc::format!("   {} {}", item.icon, item.label)
            };
            
            // Center the item text within the box (75 inner width)
            let item_padding = (75 - item_text.len()) / 2;
            let item_x = x + 1 + item_padding;
            
            if i == self.selected_index {
                screen.put_str_at(item_x, current_y, &item_text, EFI_LIGHTGREEN, EFI_BLACK);
            } else {
                screen.put_str_at(item_x, current_y, &item_text, EFI_GREEN, EFI_BLACK);
            }

            screen.put_str_at(x + 76, current_y, "|", EFI_GREEN, EFI_BLACK);
            current_y += 1;

            // Empty line between menu items (except after last)
            if i < self.menu_items.len() - 1 {
                screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
                current_y += 1;
            }
        }

        // Divider after menu
        screen.put_str_at(x, current_y, DIVIDER, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Empty line before status
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Status bar
        screen.put_str_at(x, current_y, "|", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 3, current_y, "Status: READY", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(x + 20, current_y, "|", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 22, current_y, "System: Operational", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 76, current_y, "|", EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Empty line after status
        screen.put_str_at(x, current_y, EMPTY_LINE, EFI_GREEN, EFI_BLACK);
        current_y += 1;

        // Bottom border
        screen.put_str_at(x, current_y, BOTTOM_BORDER, EFI_GREEN, EFI_BLACK);
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
        self.debug.render(screen);

        loop {
            // Render global rain if active
            crate::tui::rain::render_rain(screen);

            // Always render debug overlay on top
            self.debug.render(screen);

            // Check for input with frame limiting (~60 FPS)
            if let Some(key) = keyboard.poll_key_with_delay() {
                // Debug overlay toggle
                if key.unicode_char == b'd' as u16 || key.unicode_char == b'D' as u16 {
                    self.debug.toggle();
                    screen.clear();
                    self.render(screen);
                    self.debug.render(screen);
                    continue;
                }

                // Global rain toggle
                if key.unicode_char == b'x' as u16 || key.unicode_char == b'X' as u16 {
                    crate::tui::rain::toggle_rain(screen);
                    screen.clear();
                    self.render(screen);
                    self.debug.render(screen);
                    continue;
                }

                let action = self.handle_input(&key);
                if !matches!(action, MenuAction::Navigate) {
                    return action;
                }

                // Re-render UI after navigation (without clearing)
                self.render(screen);
                self.debug.render(screen);
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
