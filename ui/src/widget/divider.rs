use crate::canvas::Canvas;
use crate::draw::shapes::{hline, vline};
use crate::event::{Event, EventResult};
use crate::theme::Theme;
use super::Widget;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

pub struct Divider {
    orientation: Orientation,
    length: u32,
}

impl Divider {
    pub fn horizontal(length: u32) -> Self {
        Self {
            orientation: Orientation::Horizontal,
            length,
        }
    }

    pub fn vertical(length: u32) -> Self {
        Self {
            orientation: Orientation::Vertical,
            length,
        }
    }
}

impl Widget for Divider {
    fn size_hint(&self) -> (u32, u32) {
        match self.orientation {
            Orientation::Horizontal => (self.length, 1),
            Orientation::Vertical => (1, self.length),
        }
    }

    fn render(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        let w = canvas.width();
        let h = canvas.height();
        match self.orientation {
            Orientation::Horizontal => {
                hline(canvas, 0, 0, w, theme.border);
            }
            Orientation::Vertical => {
                vline(canvas, 0, 0, h, theme.border);
            }
        }
    }

    fn handle_event(&mut self, _event: &Event) -> EventResult {
        EventResult::Ignored
    }
}
