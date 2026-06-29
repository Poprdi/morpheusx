//! Userspace CRT0: `_start`, per-thread TLS setup, and the panic handler.

#[cfg(all(not(test), feature = "panic-handler"))]
use crate::process;

// Bounds of the thread-local template, defined by libmorpheus/linker.ld. The
// span is padded to 64 bytes there, so `__tls_end - __tls_start` already equals
// the linker's `alignTo(PT_TLS.p_memsz, p_align)` tpoff block size for any
// thread-local alignment <= 64 — which lets crt0 stay align-agnostic (it can't
// read PT_TLS at runtime: the phdrs aren't in a mapped PT_LOAD).
extern "C" {
    static __tls_start: u8;
    static __tls_end: u8;
}

/// Minimal variant-II TCB. Only the self-pointer at `fs:0` is mandated; the
/// extra room keeps probes of low TCB slots (e.g. a stack canary at `fs:0x28`)
/// landing in mapped zero memory.
const TCB_SIZE: usize = 64;

/// Set up this thread's variant-II TLS block (local-exec model) and point the
/// FS base at it. Layout: `[base, base+block)` holds the TLS image copied
/// verbatim from the linker template (`.tbss` is folded into PROGBITS `.tdata`,
/// so the whole span is initialized and needs no separate zeroing); `tp =
/// base + block` is the TCB, with `*(tp) = tp` as the variant-II self-pointer
/// that `%fs:0`-relative accesses load. No-op when the binary has no
/// thread-locals (`block == 0`).
///
/// # Safety
/// Call once per thread, before any `#[thread_local]` access. The block is
/// intentionally leaked for the thread's lifetime — TLS must outlive every
/// thread-local access.
pub unsafe fn install_thread_tls() {
    let start = core::ptr::addr_of!(__tls_start) as usize;
    let end = core::ptr::addr_of!(__tls_end) as usize;
    let block = end - start;
    if block == 0 {
        return; // binary has no thread-locals
    }
    let pages = (block + TCB_SIZE).div_ceil(4096) as u64;
    let base = crate::mem::mmap_raw(pages);
    if crate::is_error(base) {
        return; // OOM; a subsequent #[thread_local] access would fault
    }
    let base = base as usize; // page-aligned, zeroed, USER_RW
    let tp = base + block;
    core::ptr::copy_nonoverlapping(start as *const u8, base as *mut u8, block);
    *(tp as *mut u64) = tp as u64; // variant-II self-pointer (read via %fs:0)
    let _ = crate::thread::set_thread_pointer(tp as u64);
}

/// Emits `_start`, which installs the main thread's TLS, calls `$main`, and
/// exits with its return code.
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        #[no_mangle]
        pub extern "C" fn _start() -> ! {
            // SAFETY: process entry — runs once before any `#[thread_local]` use.
            unsafe {
                $crate::entry::install_thread_tls();
            }
            let code: i32 = $main();
            $crate::process::exit(code);
        }
    };
}

#[cfg(all(not(test), feature = "panic-handler"))]
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
