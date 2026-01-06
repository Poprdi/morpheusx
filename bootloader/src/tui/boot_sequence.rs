use crate::tui::renderer::{Screen, EFI_BLACK, EFI_GREEN, EFI_LIGHTGREEN};
use morpheus_core::logger;

pub struct BootSequence {
    last_rendered_count: usize,
    last_total_count: usize,
    completed: bool,
}

impl BootSequence {
    pub fn new() -> Self {
        Self {
            last_rendered_count: 0,
            last_total_count: 0,
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
        
        // Only re-render if logs have changed
        if total_count != self.last_total_count {
            // Collect logs into a buffer to compare
            let mut line_idx = 0;
            for log in logger::get_last_n_logs(logs_to_show) {
                let line_y = y + line_idx;
                if line_y < screen.height() {
                    // Clear just this line before writing
                    screen.put_str_at(x, line_y, "                                                                                ", EFI_BLACK, EFI_BLACK);
                    screen.put_str_at(x, line_y, "[  OK  ] ", EFI_GREEN, EFI_BLACK);
                    screen.put_str_at(x + 9, line_y, log, EFI_LIGHTGREEN, EFI_BLACK);
                }
                line_idx += 1;
            }
            
            // Clear any remaining lines if we have fewer logs now
            let current_log_count = logger::log_count().min(logs_to_show);
            for i in current_log_count..logs_to_show {
                let line_y = y + i;
                if line_y < screen.height() {
                    screen.put_str_at(x, line_y, "                                                                                ", EFI_BLACK, EFI_BLACK);
                }
            }
            
            self.last_total_count = total_count;
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
