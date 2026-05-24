//! Morpheus core: disk, FS, ISO, networking. `no_std`.

#![no_std]
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::new_without_default)]
#![allow(clippy::result_unit_err)]
#![allow(clippy::op_ref)]
#![allow(clippy::manual_div_ceil)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]

pub mod disk;
pub mod fs;
pub mod iso;
pub mod logger;
pub mod net;
pub mod uefi_alloc;
