use crate::tui::renderer::{Screen, EFI_BLACK, EFI_GREEN, EFI_LIGHTGREEN};
use morpheus_core::logger;

pub struct BootSequence {
    last_rendered_count: usize,
    completed: bool,
}

impl BootSequence {
    pub fn new() -> Self {
        Self {
            last_rendered_count: 0,
            completed: false,
        }
    }

    pub fn mark_complete(&mut self) {
        self.completed = true;
    }

    pub fn render(&mut self, screen: &mut Screen, x: usize, y: usize) {
        let total_count = logger::total_log_count();
        
        // Only show last 20 logs to fit on screen
        let logs_to_show = 20;
        
        // Clear the log area first to prevent text overlap
        for i in 0..logs_to_show {
            let line_y = y + i;
            if line_y < screen.height() {
                screen.put_str_at(x, line_y, "                                                                                ", EFI_BLACK, EFI_BLACK);
            }
        }
        
        // Now render the actual logs
        for (i, log) in logger::get_last_n_logs(logs_to_show).enumerate() {
            let line_y = y + i;
            if line_y < screen.height() {
                screen.put_str_at(x, line_y, "[  OK  ] ", EFI_GREEN, EFI_BLACK);
                screen.put_str_at(x + 9, line_y, log, EFI_LIGHTGREEN, EFI_BLACK);
            }
        }

        if self.completed {
            let displayed_logs = logs_to_show.min(logger::log_count());
            let final_y = y + displayed_logs;
            if final_y < screen.height() - 1 {
                screen.put_str_at(
                    x,
                    final_y + 1,
                    "> System initialized. Press any key...",
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
            }
        }

        self.last_rendered_count = total_count;
    }
}
