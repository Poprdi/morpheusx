//! Morpheus Core Library
//!
//! Low-level operations for disk, filesystem, and distro management.
//! Designed to be no_std compatible.

#![no_std]
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::new_without_default)]
#![allow(clippy::result_unit_err)]
#![allow(clippy::op_ref)]
#![allow(clippy::manual_div_ceil)]

pub mod disk;
pub mod fs;
pub mod iso;
pub mod logger;
