//! CRT0 for userspace. provides `_start` and a panic handler that
//! prints something vaguely useful before dying.

use crate::process;

/// Generates `_start`. your main() returns i32, we call exit(). simple.
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
