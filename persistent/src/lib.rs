//! Bootloader self-persistence: PE/COFF parsing + storage backends.
//!
//! Relocation table format is identical across arches; semantics differ
//! (x86_64 pointer fixups vs ARM64 ADRP/ADD instruction encoding). Arch
//! modules abstract the divergence.

#![no_std]
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::new_without_default)]
#![allow(clippy::doc_lazy_continuation)]

extern crate alloc;

pub mod capture;
pub mod feedback;
pub mod pe;
pub mod storage;

#[cfg(target_arch = "x86_64")]
pub mod arch {
    pub mod x86_64;
}

#[cfg(target_arch = "aarch64")]
pub mod arch {
    pub mod aarch64;
}
