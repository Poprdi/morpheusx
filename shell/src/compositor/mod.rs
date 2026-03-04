extern crate alloc;

use libmorpheus::compositor as compsys;
use libmorpheus::{hw, io, mem, process};

use crate::fb::Framebuffer;
use crate::font;

mod event_loop;
mod input;
mod render;
mod state;
mod surfaces;

pub use event_loop::compositor_loop;
pub use state::Compositor;

pub(super) use state::*;
