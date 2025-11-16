use super::button::Button;
use crate::tui::input::{InputKey, Keyboard, KEY_ENTER, SCAN_DOWN, SCAN_UP};
use crate::tui::renderer::Screen;

pub struct Menu {
    pub buttons: [Button; 5],
    pub selected_index: usize,
    pub button_count: usize,
}

impl Menu {
    pub fn new(start_x: usize, start_y: usize) -> Self {
        let spacing = 2; // Changed from 4 since buttons are now single line
        Self {
            buttons: [
                Button::new(start_x, start_y, "Boot Arch"),
                Button::new(start_x, start_y + spacing, "Boot Fedora"),
                Button::new(start_x, start_y + spacing * 2, "Boot Ubuntu"),
                Button::new(start_x, start_y + spacing * 3, "Configure"),
                Button::new(start_x, start_y + spacing * 4, "Shell"),
            ],
            selected_index: 0,
            button_count: 5,
        }
    }

    pub fn select(&mut self, index: usize) {
        if index < self.button_count {
            // Deselect all
            for btn in &mut self.buttons {
                btn.selected = false;
            }
            // Select target
            self.buttons[index].selected = true;
            self.selected_index = index;
        }
    }

    pub fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.select(self.selected_index - 1);
        }
    }

    pub fn move_down(&mut self) {
        if self.selected_index < self.button_count - 1 {
            self.select(self.selected_index + 1);
        }
    }

    pub fn render(&self, screen: &mut Screen) {
        for i in 0..self.button_count {
            self.buttons[i].render(screen);
        }
    }

    // Returns the index of selected button when user presses enter
    pub fn handle_input(&mut self, key: &InputKey) -> Option<usize> {
        if key.scan_code == SCAN_UP {
            self.move_up();
        } else if key.scan_code == SCAN_DOWN {
            self.move_down();
        } else if key.unicode_char == KEY_ENTER {
            return Some(self.selected_index);
        }
        None
    }

    pub fn run(&mut self, screen: &mut Screen, keyboard: &mut Keyboard) -> usize {
        self.select(0);

        loop {
            self.render(screen);

            let key = keyboard.wait_for_key();
            if let Some(choice) = self.handle_input(&key) {
                return choice;
            }
        }
    }
}
