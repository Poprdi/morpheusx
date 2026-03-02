use crate::tui::input::{
    InputKey, KEY_BACKSPACE, KEY_ENTER, KEY_TAB, SCAN_DELETE, SCAN_DOWN, SCAN_END, SCAN_ESC,
    SCAN_F1, SCAN_F12, SCAN_HOME, SCAN_LEFT, SCAN_PGDN, SCAN_PGUP, SCAN_RIGHT, SCAN_UP,
};
use morpheus_ui::event::{Event, Key, KeyEvent, Modifiers};

pub fn translate_key(input: &InputKey, kb: &crate::tui::input::Keyboard) -> Option<Event> {
    let modifiers = Modifiers {
        shift: kb.is_shift(),
        ctrl: kb.is_ctrl(),
        alt: kb.is_alt(),
    };

    let key = if input.scan_code != 0 {
        match input.scan_code {
            SCAN_UP => Key::Up,
            SCAN_DOWN => Key::Down,
            SCAN_LEFT => Key::Left,
            SCAN_RIGHT => Key::Right,
            SCAN_HOME => Key::Home,
            SCAN_END => Key::End,
            SCAN_PGUP => Key::PageUp,
            SCAN_PGDN => Key::PageDown,
            SCAN_DELETE => Key::Delete,
            SCAN_ESC => Key::Escape,
            sc if (SCAN_F1..=SCAN_F12).contains(&sc) => Key::F((sc - SCAN_F1 + 1) as u8),
            _ => return None,
        }
    } else if input.unicode_char != 0 {
        match input.unicode_char {
            KEY_ENTER => Key::Enter,
            KEY_BACKSPACE => Key::Backspace,
            KEY_TAB => Key::Tab,
            ch => {
                if let Some(c) = char::from_u32(ch as u32) {
                    Key::Char(c)
                } else {
                    return None;
                }
            }
        }
    } else {
        return None;
    };

    Some(Event::KeyPress(KeyEvent { key, modifiers }))
}
