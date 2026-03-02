use crate::canvas::Canvas;
use crate::event::Event;
use crate::theme::Theme;
use alloc::boxed::Box;
use alloc::vec::Vec;

pub enum AppResult {
    Continue,
    Close,
    Redraw,
}

pub trait App {
    fn title(&self) -> &str;
    fn default_size(&self) -> (u32, u32);
    fn init(&mut self, canvas: &mut dyn Canvas, theme: &Theme);
    fn render(&self, canvas: &mut dyn Canvas, theme: &Theme);
    fn handle_event(&mut self, event: &Event) -> AppResult;
}

pub struct AppEntry {
    pub name: &'static str,
    pub title: &'static str,
    pub default_size: (u32, u32),
    pub create: fn() -> Box<dyn App>,
}

pub struct AppRegistry {
    entries: Vec<AppEntry>,
}

impl AppRegistry {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn register(&mut self, entry: AppEntry) {
        self.entries.push(entry);
    }

    pub fn find(&self, name: &str) -> Option<&AppEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    pub fn list(&self) -> &[AppEntry] {
        &self.entries
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.entries.iter().map(|e| e.name).collect()
    }
}
