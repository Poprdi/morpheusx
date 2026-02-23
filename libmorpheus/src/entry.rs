//! C runtime for MorpheusX user processes.
//!
//! Provides `_start` (the ELF entry point) and a minimal `#[panic_handler]`.
//! User binaries define `fn main() -> i32` (or `fn main()`) which this
//! runtime calls after zeroing BSS.
//!
//! # Usage
//!
//! In your binary crate:
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//!
//! use libmorpheus::entry;
//!
//! entry!(main);
//!
//! fn main() -> i32 {
//!     libmorpheus::io::println("Hello from userspace!");
//!     0
//! }
//! ```

use crate::process;

/// The entry-point macro.  Generates `_start` that calls your `main`.
///
/// `main` must be `fn() -> i32` or `fn()`.
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        #[no_mangle]
        pub extern "C" fn _start() -> ! {
            let code: i32 = $main();
            $crate::process::exit(code);
        }
    };
}

/// Minimal panic handler — prints the message to serial and exits.
#[cfg(not(test))]
#[panic_handler]
fn _panic(_info: &core::panic::PanicInfo) -> ! {
    // Write a terse panic message to stderr (fd 2) via SYS_WRITE.
    let msg = "User Process shit the bed!\n";
    let _ = unsafe {
        crate::raw::syscall3(
            crate::raw::SYS_WRITE,
            2, // stderr
            msg.as_ptr() as u64,
            msg.len() as u64,
        )
    };

    process::exit(101);
}
