use libmorpheus::hw;
use libmorpheus::io;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    RotateLeft,
    RotateRight,
    RotateUp,
    RotateDown,
    ZoomIn,
    ZoomOut,
    SelectNext,
    SelectPrev,
    KillSelected,
    Focus,
    Unfocus,
    TogglePause,
    ToggleHud,
    Quit,
}

const MAX_ACTIONS: usize = 8;

pub struct InputState {
    actions: [Action; MAX_ACTIONS],
    count: usize,
    pub mouse_dx: f32,
    pub mouse_dy: f32,
    pub mouse_left: bool,
    pub mouse_right: bool,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            actions: [Action::None; MAX_ACTIONS],
            count: 0,
            mouse_dx: 0.0,
            mouse_dy: 0.0,
            mouse_left: false,
            mouse_right: false,
        }
    }

    pub fn poll(&mut self) {
        self.count = 0;
        self.mouse_dx = 0.0;
        self.mouse_dy = 0.0;

        let mouse = hw::mouse_read();
        self.mouse_dx = mouse.dx as f32;
        self.mouse_dy = mouse.dy as f32;
        self.mouse_left = (mouse.buttons & 1) != 0;
        self.mouse_right = (mouse.buttons & 2) != 0;

        let avail = io::stdin_available();
        if avail == 0 {
            return;
        }

        let mut buf = [0u8; 16];
        let to_read = avail.min(16);
        let n = io::read_stdin(&mut buf[..to_read]);

        for i in 0..n {
            let action = match buf[i] {
                b'a' | b'A' => Action::RotateLeft,
                b'd' | b'D' => Action::RotateRight,
                b'w' | b'W' => Action::RotateUp,
                b's' | b'S' => Action::RotateDown,
                b'z' | b'Z' | b'=' | b'+' => Action::ZoomIn,
                b'x' | b'X' | b'-' => Action::ZoomOut,
                b'\t' | b'n' | b'N' => Action::SelectNext,
                b'p' | b'P' => Action::SelectPrev,
                b'k' | b'K' => Action::KillSelected,
                b'\r' | b'\n' => Action::Focus,
                0x1B => Action::Unfocus,
                b' ' => Action::TogglePause,
                b'h' | b'H' => Action::ToggleHud,
                b'q' | b'Q' => Action::Quit,
                _ => Action::None,
            };
            if action != Action::None && self.count < MAX_ACTIONS {
                self.actions[self.count] = action;
                self.count += 1;
            }
        }
    }

    pub fn has(&self, action: Action) -> bool {
        self.actions[..self.count].iter().any(|&a| a == action)
    }

    pub fn iter_actions(&self) -> &[Action] {
        &self.actions[..self.count]
    }
}
