use super::entry::BootEntry;
use super::renderer::EntryRenderer;
use super::scanner::EntryScanner;
use crate::boot::loader::BootError;
use crate::tui::input::Keyboard;
use crate::tui::renderer::Screen;
use alloc::vec::Vec;

pub struct DistroLauncher {
    entries: Vec<BootEntry>,
    selected_index: usize,
}

impl DistroLauncher {
    pub fn new(boot_services: *const crate::BootServices, image_handle: *mut ()) -> Self {
        morpheus_core::logger::log("DistroLauncher::new() - scanning for boot entries");

        let scanner = EntryScanner::new(boot_services, image_handle);
        let entries = scanner.scan_boot_entries();

        morpheus_core::logger::log(alloc::format!("Found {} boot entries", entries.len()).leak());

        Self {
            entries,
            selected_index: 0,
        }
    }
    fn select_next(&mut self) {
        if self.selected_index < self.entries.len() - 1 {
            self.selected_index += 1;
        }
    }

    fn select_prev(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    fn render(&self, screen: &mut Screen) {
        EntryRenderer::render_header(screen);
        EntryRenderer::render_entries(screen, &self.entries, self.selected_index);
        EntryRenderer::render_footer(screen);
    }
    pub fn run(
        &mut self,
        screen: &mut Screen,
        keyboard: &mut Keyboard,
        boot_services: &crate::BootServices,
        system_table: *mut (),
        image_handle: *mut (),
    ) {
        screen.clear();
        self.render(screen);

        loop {
            if let Some(key) = keyboard.poll_key_with_delay() {
                // ESC - return to main menu
                if key.scan_code == 0x17 {
                    return;
                }

                // Up arrow
                if key.scan_code == 0x01 {
                    self.select_prev();
                    self.render(screen);
                }

                // Down arrow
                if key.scan_code == 0x02 {
                    self.select_next();
                    self.render(screen);
                }

                // Enter - boot selected kernel
                if key.unicode_char == 0x0D {
                    morpheus_core::logger::log("enter pressed");
                    let entry = &self.entries[self.selected_index];
                    morpheus_core::logger::log("entry selected");
                    self.boot_entry(
                        screen,
                        keyboard,
                        boot_services,
                        system_table,
                        image_handle,
                        entry,
                    );
                    morpheus_core::logger::log("boot_entry returned");
                    // If we return here, boot failed
                    morpheus_core::logger::log("clearing screen");
                    screen.clear();
                    morpheus_core::logger::log("calling render");
                    self.render(screen);
                    morpheus_core::logger::log("render complete");
                }
            }
        }
    }
}
