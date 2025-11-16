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
        let logs = logger::get_logs();
        let log_count = logs.len();

        // Only show last 20 logs to fit on screen
        let start_idx = log_count.saturating_sub(20);

        for (i, log_opt) in logs[start_idx..log_count].iter().enumerate() {
            if let Some(log) = log_opt {
                let line_y = y + i;
                if line_y < 25 {
                    screen.put_str_at(x, line_y, "[  OK  ] ", EFI_GREEN, EFI_BLACK);
                    screen.put_str_at(x + 9, line_y, log, EFI_LIGHTGREEN, EFI_BLACK);
                }
            }
        }

        if self.completed {
            let final_y = y + (log_count - start_idx).min(20);
            if final_y < 24 {
                screen.put_str_at(
                    x,
                    final_y + 1,
                    "> System initialized. Press any key...",
                    EFI_LIGHTGREEN,
                    EFI_BLACK,
                );
            }
        }

        self.last_rendered_count = log_count;
    }
}
