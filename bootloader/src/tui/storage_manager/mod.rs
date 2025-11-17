use crate::tui::input::Keyboard;
use crate::tui::rain::MatrixRain;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN};
use crate::BootServices;
use morpheus_core::disk::manager::DiskManager;
use morpheus_core::disk::partition::PartitionTable;

mod format;
mod gpt_ops_ui;
mod partition_ops;
mod render;
mod utils;

pub struct StorageManager {
    disk_manager: DiskManager,
    selected_disk: usize,
    view_mode: ViewMode,

    // Partition view state
    pub(self) partition_table: PartitionTable,
    pub(self) selected_partition: usize,
    pub(self) current_disk_index: usize,
    pub(self) free_space_mb: u64,

    // Rain effect
    rain: MatrixRain,
}

enum ViewMode {
    DiskList,
    PartitionView,
}

impl StorageManager {
    pub fn new(screen: &Screen) -> Self {
        Self {
            disk_manager: DiskManager::new(),
            selected_disk: 0,
            view_mode: ViewMode::DiskList,
            partition_table: PartitionTable::new(),
            selected_partition: 0,
            current_disk_index: 0,
            free_space_mb: 0,
            rain: MatrixRain::new(screen.width(), screen.height()),
        }
    }

    pub fn select_next(&mut self) {
        match self.view_mode {
            ViewMode::DiskList => {
                let disk_count = self.disk_manager.disk_count();
                if disk_count > 0 && self.selected_disk < disk_count - 1 {
                    self.selected_disk += 1;
                }
            }
            ViewMode::PartitionView => {
                let part_count = self.partition_table.count();
                if part_count > 0 && self.selected_partition < part_count - 1 {
                    self.selected_partition += 1;
                }
            }
        }
    }

    pub fn select_prev(&mut self) {
        match self.view_mode {
            ViewMode::DiskList => {
                if self.selected_disk > 0 {
                    self.selected_disk -= 1;
                }
            }
            ViewMode::PartitionView => {
                if self.selected_partition > 0 {
                    self.selected_partition -= 1;
                }
            }
        }
    }

    pub(self) fn format_number(num: u64, buf: &mut [u8]) -> usize {
        if num == 0 {
            buf[0] = b'0';
            return 1;
        }

        let mut n = num;
        let mut digits = [0u8; 20];
        let mut count = 0;

        while n > 0 {
            digits[count] = b'0' + (n % 10) as u8;
            n /= 10;
            count += 1;
        }

        for i in 0..count {
            if i < buf.len() {
                buf[i] = digits[count - 1 - i];
            }
        }

        count
    }

    pub fn run(&mut self, screen: &mut Screen, keyboard: &mut Keyboard, bs: &BootServices) -> bool {
        // Enumerate disks using UEFI helper
        if crate::uefi::disk::enumerate_disks(bs, &mut self.disk_manager).is_err() {
            screen.clear();
            screen.put_str_at(
                5,
                10,
                "ERROR: Failed to enumerate disks",
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );
            screen.put_str_at(5, 12, "Press any key to return...", EFI_GREEN, EFI_BLACK);
            keyboard.wait_for_key();
            return false;
        }

        // Initial render
        self.render(screen);

        loop {
            // Render global rain if active
            crate::tui::rain::render_rain(screen);

            // Check for input (non-blocking)
            if let Some(key) = keyboard.read_key() {
                // Global rain toggle
                if key.unicode_char == b'x' as u16 || key.unicode_char == b'X' as u16 {
                    crate::tui::rain::toggle_rain(screen);
                    screen.clear();
                    self.render(screen);
                    continue;
                }

                if self.handle_input(key, screen, keyboard, bs) {
                    return true; // Exit back to main menu
                }
            }
        }
    }

    fn handle_input(
        &mut self,
        key: crate::tui::input::InputKey,
        screen: &mut Screen,
        keyboard: &mut Keyboard,
        bs: &BootServices,
    ) -> bool {
        match self.view_mode {
            ViewMode::DiskList => {
                if key.scan_code == 0x01 {
                    self.select_prev();
                    self.render(screen);
                } else if key.scan_code == 0x02 {
                    self.select_next();
                    self.render(screen);
                } else if key.scan_code == 0 && key.unicode_char == 0x000D {
                    self.handle_disk_selection(screen, keyboard, bs);
                } else if key.scan_code == 0x17 {
                    return true; // ESC pressed - exit to main menu
                }
            }
            ViewMode::PartitionView => {
                if key.scan_code == 0x01 {
                    self.select_prev();
                    self.render(screen);
                } else if key.scan_code == 0x02 {
                    self.select_next();
                    self.render(screen);
                } else if key.unicode_char == b'c' as u16 || key.unicode_char == b'C' as u16 {
                    if !self.partition_table.has_gpt {
                        self.create_gpt_interactive(screen, keyboard, bs);
                        let _ = self.scan_disk(self.current_disk_index, bs);
                        self.render(screen);
                    }
                } else if key.unicode_char == b'n' as u16 || key.unicode_char == b'N' as u16 {
                    self.create_partition_ui(screen, keyboard, bs);
                    let _ = self.scan_disk(self.current_disk_index, bs);
                    self.render(screen);
                } else if key.unicode_char == b'd' as u16 || key.unicode_char == b'D' as u16 {
                    self.delete_partition_ui(screen, keyboard, bs);
                    let _ = self.scan_disk(self.current_disk_index, bs);
                    if self.selected_partition >= self.partition_table.count()
                        && self.selected_partition > 0
                    {
                        self.selected_partition -= 1;
                    }
                    self.render(screen);
                } else if key.unicode_char == b's' as u16 || key.unicode_char == b'S' as u16 {
                    self.shrink_partition_ui(screen, keyboard, bs);
                    let _ = self.scan_disk(self.current_disk_index, bs);
                    self.render(screen);
                } else if key.unicode_char == b'f' as u16 || key.unicode_char == b'F' as u16 {
                    self.format_partition_ui(screen, keyboard, bs);
                    self.render(screen);
                } else if key.scan_code == 0x17 {
                    self.view_mode = ViewMode::DiskList;
                    self.render(screen);
                }
            }
        }
        false // Continue running
    }

    fn handle_disk_selection(
        &mut self,
        screen: &mut Screen,
        keyboard: &mut Keyboard,
        bs: &BootServices,
    ) {
        let disk_count = self.disk_manager.disk_count();
        if disk_count > 0 && self.selected_disk < disk_count {
            self.current_disk_index = self.selected_disk;

            match self.scan_disk(self.selected_disk, bs) {
                Ok(()) => {
                    if !self.partition_table.has_gpt {
                        screen.clear();
                        screen.put_str_at(
                            5,
                            5,
                            "=== NO PARTITION TABLE FOUND ===",
                            EFI_LIGHTGREEN,
                            EFI_BLACK,
                        );
                        screen.put_str_at(
                            5,
                            7,
                            "This disk has no GPT partition table.",
                            EFI_GREEN,
                            EFI_BLACK,
                        );
                        screen.put_str_at(
                            5,
                            9,
                            "Would you like to create one?",
                            EFI_GREEN,
                            EFI_BLACK,
                        );
                        screen.put_str_at(
                            5,
                            11,
                            "[Y] Yes, create GPT    [N] No, go back",
                            EFI_DARKGREEN,
                            EFI_BLACK,
                        );

                        let response = keyboard.wait_for_key();
                        if response.unicode_char == b'y' as u16
                            || response.unicode_char == b'Y' as u16
                        {
                            self.create_gpt_interactive(screen, keyboard, bs);
                            let _ = self.scan_disk(self.current_disk_index, bs);

                            if self.partition_table.has_gpt {
                                self.view_mode = ViewMode::PartitionView;
                                self.selected_partition = 0;
                            }
                        }
                        self.render(screen);
                    } else {
                        self.view_mode = ViewMode::PartitionView;
                        self.selected_partition = 0;
                        self.render(screen);
                    }
                }
                Err(err_code) => {
                    screen.clear();
                    screen.put_str_at(
                        5,
                        5,
                        "=== ERROR SCANNING DISK ===",
                        EFI_LIGHTGREEN,
                        EFI_BLACK,
                    );

                    let mut err_buf = [0u8; 20];
                    let err_len = utils::format_number(err_code as u64, &mut err_buf);
                    let err_str = core::str::from_utf8(&err_buf[..err_len]).unwrap_or("?");

                    screen.put_str_at(5, 7, "Error code: ", EFI_GREEN, EFI_BLACK);
                    screen.put_str_at(17, 7, err_str, EFI_GREEN, EFI_BLACK);
                    screen.put_str_at(5, 9, "Failed to access disk", EFI_GREEN, EFI_BLACK);
                    screen.put_str_at(
                        5,
                        11,
                        "Press any key to continue...",
                        EFI_DARKGREEN,
                        EFI_BLACK,
                    );
                    keyboard.wait_for_key();
                    self.render(screen);
                }
            }
        }
    }

    fn render(&mut self, screen: &mut Screen) {
        screen.clear();

        match self.view_mode {
            ViewMode::DiskList => self.render_disk_list(screen),
            ViewMode::PartitionView => self.render_partition_view(screen),
        }
    }
}
