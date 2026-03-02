#![no_std]
#![allow(dead_code)]
#![allow(clippy::new_without_default)]

extern crate alloc;

pub mod app;
pub mod buffer;
pub mod canvas;
pub mod clip;
pub mod color;
pub mod compositor;
pub mod draw;
pub mod event;
pub mod font;
pub mod rect;
pub mod shell;
pub mod theme;
pub mod widget;
pub mod window;
pub mod wm;

pub use app::{App, AppResult};
pub use buffer::OffscreenBuffer;
pub use canvas::Canvas;
pub use color::{Color, PixelFormat};
pub use event::{Event, EventResult, Key, KeyEvent, Modifiers, MouseButton};
pub use rect::Rect;
pub use theme::Theme;
pub use widget::Widget;
pub use window::Window;
pub use wm::WindowManager;
