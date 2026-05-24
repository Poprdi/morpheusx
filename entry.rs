//! Userspace C runtime: `_start` and panic handler. User defines `fn main() -> i32`.

use crate::process;

/// Emit `_start` that calls `$main` and exits with its return code.
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

#[cfg(not(test))]
#[panic_handler]
fn _panic(_info: &core::panic::PanicInfo) -> ! {
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
