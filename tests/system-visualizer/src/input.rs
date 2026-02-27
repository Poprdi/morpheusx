use libmorpheus::hw;
use libmorpheus::io;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    SelectNext,
    SelectPrev,
    KillSelected,
    Focus,
    Unfocus,
    TogglePause,
    ToggleHud,
    ToggleSlow,
    ResetView,
    TogglePin,
    SpeedUp,
    SpeedDown,
    SelectDigit1,
    SelectDigit2,
    SelectDigit3,
    SelectDigit4,
    SelectDigit5,
    SelectDigit6,
    SelectDigit7,
    SelectDigit8,
    SelectDigit9,
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
    pub held: u16,
}

pub const HELD_W: u16     = 1 << 0;
pub const HELD_A: u16     = 1 << 1;
pub const HELD_S: u16     = 1 << 2;
pub const HELD_D: u16     = 1 << 3;
pub const HELD_Z: u16     = 1 << 4;
pub const HELD_X: u16     = 1 << 5;

impl InputState {
    pub fn new() -> Self {
        Self {
            actions: [Action::None; MAX_ACTIONS],
            count: 0,
            mouse_dx: 0.0,
            mouse_dy: 0.0,
            mouse_left: false,
            mouse_right: false,
            held: 0,
        }
    }

    pub fn poll(&mut self) {
        self.count = 0;
        self.mouse_dx = 0.0;
        self.mouse_dy = 0.0;
        self.held = 0;

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
            match buf[i] {
                b'w' | b'W' => self.held |= HELD_W,
                b'a' | b'A' => self.held |= HELD_A,
                b's' | b'S' => self.held |= HELD_S,
                b'd' | b'D' => self.held |= HELD_D,
                b'z' | b'Z' | b'=' | b'+' => self.held |= HELD_Z,
                b'x' | b'X' | b'-' => self.held |= HELD_X,
                _ => {}
            }

            let action = match buf[i] {
                b'\t' | b'n' | b'N' => Action::SelectNext,
                b'p' | b'P' => Action::SelectPrev,
                b'k' | b'K' => Action::KillSelected,
                b'\r' | b'\n' => Action::Focus,
                0x1B => Action::Unfocus,
                b' ' => Action::TogglePause,
                b'h' | b'H' => Action::ToggleHud,
                b'o' | b'O' => Action::ToggleSlow,
                b'r' | b'R' => Action::ResetView,
                b'f' | b'F' => Action::TogglePin,
                0x18 => Action::SpeedDown,   // Ctrl+X
                0x19 => Action::SpeedUp,     // Ctrl+Y
                b'1' => Action::SelectDigit1,
                b'2' => Action::SelectDigit2,
                b'3' => Action::SelectDigit3,
                b'4' => Action::SelectDigit4,
                b'5' => Action::SelectDigit5,
                b'6' => Action::SelectDigit6,
                b'7' => Action::SelectDigit7,
                b'8' => Action::SelectDigit8,
                b'9' => Action::SelectDigit9,
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
}
