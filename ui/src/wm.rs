use alloc::vec::Vec;
use crate::canvas::Canvas;
use crate::color::PixelFormat;
use crate::compositor::Compositor;
use crate::event::{Event, EventResult, Key, KeyEvent};
use crate::theme::Theme;
use crate::window::Window;

pub struct WindowManager {
    windows: Vec<Window>,
    next_id: u32,
    focused_id: Option<u32>,
    compositor: Compositor,
    format: PixelFormat,
    screen_w: u32,
    screen_h: u32,
}

impl WindowManager {
    pub fn new(screen_w: u32, screen_h: u32, format: PixelFormat, theme: &Theme) -> Self {
        Self {
            windows: Vec::new(),
            next_id: 1,
            focused_id: None,
            compositor: Compositor::new(theme.bg),
            format,
            screen_w,
            screen_h,
        }
    }

    pub fn create_window(
        &mut self,
        title: &str,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> u32 {
        let id = self.next_id;
        self.next_id += 1;

        let mut win = Window::new(id, title, x, y, width, height, self.format);
        win.dirty = true;
        win.visible = true;

        self.compositor.damage_rect(win.outer_rect());
        self.windows.push(win);
        self.focus_window(id);
        id
    }

    pub fn close_window(&mut self, id: u32) {
        if let Some(pos) = self.windows.iter().position(|w| w.id == id) {
            let win = &self.windows[pos];
            self.compositor.damage_rect(win.outer_rect());

            self.windows.remove(pos);

            if self.focused_id == Some(id) {
                self.focused_id = self.windows.last().map(|w| w.id);
                if let Some(fid) = self.focused_id {
                    if let Some(w) = self.window_mut(fid) {
                        w.focused = true;
                        w.dirty = true;
                    }
                }
            }
        }
    }

    pub fn focus_window(&mut self, id: u32) {
        if self.focused_id == Some(id) {
            return;
        }

        if let Some(old) = self.focused_id {
            if let Some(w) = self.window_mut(old) {
                w.focused = false;
                w.dirty = true;
                let r = w.outer_rect();
                self.compositor.damage_rect(r);
            }
        }

        self.focused_id = Some(id);

        if let Some(pos) = self.windows.iter().position(|w| w.id == id) {
            let len = self.windows.len();
            if pos < len - 1 {
                let win = self.windows.remove(pos);
                self.windows.push(win);
            }
            if let Some(w) = self.windows.last_mut() {
                w.focused = true;
                w.dirty = true;
                let r = w.outer_rect();
                self.compositor.damage_rect(r);
            }
        }
    }

    pub fn window(&self, id: u32) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    pub fn window_mut(&mut self, id: u32) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    pub fn focused_window(&self) -> Option<&Window> {
        self.focused_id.and_then(|id| self.window(id))
    }

    pub fn focused_window_mut(&mut self) -> Option<&mut Window> {
        let id = self.focused_id?;
        self.window_mut(id)
    }

    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    pub fn window_ids(&self) -> Vec<u32> {
        self.windows.iter().map(|w| w.id).collect()
    }

    pub fn dispatch_event(&mut self, event: &Event) -> EventResult {
        if let Event::KeyPress(KeyEvent { key: Key::Tab, modifiers }) = event {
            if modifiers.alt {
                self.cycle_focus();
                return EventResult::Consumed;
            }
        }

        EventResult::Ignored
    }

    fn cycle_focus(&mut self) {
        if self.windows.len() < 2 {
            return;
        }

        let current_pos = self.focused_id
            .and_then(|id| self.windows.iter().position(|w| w.id == id));

        if let Some(pos) = current_pos {
            let next = if pos == 0 {
                self.windows.len() - 1
            } else {
                pos - 1
            };
            let next_id = self.windows[next].id;
            self.focus_window(next_id);
        }
    }

    pub fn mark_dirty(&mut self, id: u32) {
        if let Some(w) = self.window_mut(id) {
            w.dirty = true;
            let r = w.outer_rect();
            self.compositor.damage_rect(r);
        }
    }

    pub fn compose(&mut self, canvas: &mut dyn Canvas, theme: &Theme) {
        for win in &self.windows {
            if win.dirty {
                self.compositor.damage_rect(win.outer_rect());
            }
        }

        self.compositor.compose(canvas, &self.windows, theme);

        for win in &self.windows {
            if win.decorations && win.visible {
                win.render_decorations(canvas, theme);
            }
        }

        for win in &mut self.windows {
            win.dirty = false;
        }
    }

    pub fn damage_all(&mut self) {
        self.compositor.damage_full(self.screen_w, self.screen_h);
    }
}
