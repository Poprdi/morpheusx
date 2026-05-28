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
fn _panic(_info: &core::panic::PanicInfo) -> ! {
    let msg = "User Process shit the bed!\n";
    let _ = unsafe {
        crate::raw::syscall3(
            crate::raw::SYS_WRITE,
            2,
            msg.as_ptr() as u64,
            msg.len() as u64,
        )
    };

    process::exit(101);
}
