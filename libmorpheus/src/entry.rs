//! Userspace CRT0: `_start` and panic handler.

use crate::process;

/// Emits `_start` that calls `$main` and exits with its return code.
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
fn _panic(info: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write;
    // `PanicInfo`'s Display gives "panicked at <file>:<line>:<col>:\n<message>".
    // FdWriter buffers on the stack (no alloc) and flushes on drop.
    {
        let mut w = crate::io::FdWriter::new(2);
        let _ = write!(w, "User Process shit the bed! {info}");
    }

    process::exit(101);
}
