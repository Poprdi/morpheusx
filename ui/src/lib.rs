#![no_std]
#![allow(dead_code)]

extern crate alloc;

pub mod color;
pub mod rect;
pub mod canvas;
pub mod buffer;
pub mod clip;
pub mod draw;
pub mod font;
pub mod event;
pub mod theme;
pub mod widget;
pub mod window;
pub mod compositor;
pub mod wm;
pub mod shell;
pub mod app;

pub use color::{Color, PixelFormat};
pub use rect::Rect;
pub use canvas::Canvas;
pub use buffer::OffscreenBuffer;
pub use event::{Event, EventResult, Key, KeyEvent, Modifiers, MouseButton};
pub use theme::Theme;
pub use widget::Widget;
pub use window::Window;
pub use wm::WindowManager;
pub use app::{App, AppResult};
