//! Morpheus Core Library
//!
//! Low-level operations for disk, filesystem, networking, and distro management.
//! Designed to be no_std compatible.
//!
//! # Modules
//!
//! - [`disk`] - GPT disk operations and partition management
//! - [`fs`] - FAT32 filesystem operations
//! - [`iso`] - ISO storage and chunk management
//! - [`net`] - Network initialization orchestration
//! - [`logger`] - Logging infrastructure

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
pub mod net;
pub mod uefi_alloc;
