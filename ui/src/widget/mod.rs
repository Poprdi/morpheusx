pub mod label;
pub mod button;
pub mod text_input;
pub mod text_area;
pub mod list;
pub mod panel;
pub mod progress;
pub mod divider;
pub mod checkbox;

pub use label::Label;
pub use button::Button;
pub use text_input::TextInput;
pub use text_area::TextArea;
pub use list::List;
pub use panel::Panel;
pub use progress::ProgressBar;
pub use divider::Divider;
pub use checkbox::Checkbox;

use crate::canvas::Canvas;
use crate::event::{Event, EventResult};
use crate::theme::Theme;

pub trait Widget {
    fn size_hint(&self) -> (u32, u32);
    fn render(&self, canvas: &mut dyn Canvas, theme: &Theme);
    fn handle_event(&mut self, event: &Event) -> EventResult;

    fn is_focusable(&self) -> bool {
        false
    }

    fn set_focused(&mut self, _focused: bool) {}
}
