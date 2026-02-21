#[derive(Debug, Clone)]
pub enum Event {
    KeyPress(KeyEvent),
    KeyRelease(KeyEvent),
    MouseMove { x: i32, y: i32 },
    MousePress { button: MouseButton, x: i32, y: i32 },
    MouseRelease { button: MouseButton, x: i32, y: i32 },
    FocusGained,
    FocusLost,
    WindowResize { width: u32, height: u32 },
    WindowClose,
    Tick,
}

#[derive(Debug, Clone)]
pub struct KeyEvent {
    pub key: Key,
    pub modifiers: Modifiers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Escape,
    Backspace,
    Delete,
    Tab,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventResult {
    Consumed,
    Ignored,
}
