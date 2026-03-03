//! Thread API — spawn, join, and JoinHandle.
//!
//! Threads share the parent's address space (same page tables).  Each thread
//! gets its own stack (allocated via SYS_MMAP) and its own kernel stack.
//! The kernel treats threads as lightweight processes with shared CR3.
//!
//! Stack size: 64 KiB default.  Aligned to page boundary.

extern crate alloc;

use alloc::boxed::Box;
use core::marker::PhantomData;

use crate::raw::*;

/// Default thread stack: 64 KiB = 16 pages.
const DEFAULT_STACK_PAGES: u64 = 16;
const DEFAULT_STACK_SIZE: u64 = DEFAULT_STACK_PAGES * 4096;

/// Handle returned by `spawn`.  Must be joined or the thread is detached.
pub struct JoinHandle<T> {
    tid: u32,
    _marker: PhantomData<T>,
}

impl<T> JoinHandle<T> {
    /// Wait for the thread to finish.  Returns the exit code.
    pub fn join(self) -> Result<u64, u64> {
        let ret = unsafe { syscall1(SYS_THREAD_JOIN, self.tid as u64) };
        if crate::is_error(ret) {
            Err(ret)
        } else {
            Ok(ret)
        }
    }

    /// The thread's TID (same as a PID in the kernel's process table).
    pub fn tid(&self) -> u32 {
        self.tid
    }
}

/// Trampoline that the kernel jumps to.  `arg` is a raw pointer to a
/// heap-allocated `Box<dyn FnOnce()>` which we call, then exit the thread.
///
/// extern "C" because the kernel sets rdi=arg and we need a stable ABI.
unsafe extern "C" fn thread_trampoline(arg: u64) -> ! {
    // Reconstruct the boxed closure and call it.
    let closure: Box<Box<dyn FnOnce()>> = Box::from_raw(arg as *mut Box<dyn FnOnce()>);
    (*closure)();

    // Thread done — exit with code 0.
    syscall1(SYS_THREAD_EXIT, 0);

    // Should never reach here, but the compiler needs a diverging type.
    unsafe { core::hint::unreachable_unchecked() }
}

/// Spawn a new thread that runs `f`.
///
/// Allocates a user stack via SYS_MMAP, boxes the closure on the heap,
/// and calls SYS_THREAD_CREATE with the trampoline as the entry point.
///
/// # Example
/// ```ignore
/// use libmorpheus::thread;
/// let h = thread::spawn(|| {
///     libmorpheus::io::println("hello from thread!");
/// });
/// h.join().unwrap();
/// ```
pub fn spawn<F>(f: F) -> Result<JoinHandle<()>, u64>
where
    F: FnOnce() + Send + 'static,
{
    // Allocate a stack for the new thread.
    let stack_base = unsafe { syscall1(SYS_MMAP, DEFAULT_STACK_PAGES) };
    if crate::is_error(stack_base) {
        return Err(stack_base);
    }

    // Stack grows down — top is base + size, aligned to 16 bytes.
    let stack_top = (stack_base + DEFAULT_STACK_SIZE) & !0xF;

    // Box the closure so it lives on the heap (shared address space).
    // Double-box: inner Box<dyn FnOnce()> for type erasure, outer Box for stable pointer.
    let closure: Box<Box<dyn FnOnce()>> = Box::new(Box::new(f));
    let arg = Box::into_raw(closure) as u64;

    let entry = thread_trampoline as *const () as u64;
    let ret = unsafe { syscall3(SYS_THREAD_CREATE, entry, stack_top, arg) };

    if crate::is_error(ret) {
        // Clean up: free the closure and the stack.
        unsafe {
            let _ = Box::from_raw(arg as *mut Box<dyn FnOnce()>);
            syscall2(SYS_MUNMAP, stack_base, DEFAULT_STACK_PAGES);
        }
        return Err(ret);
    }

    Ok(JoinHandle {
        tid: ret as u32,
        _marker: PhantomData,
    })
}

// ═══════════════════════════════════════════════════════════════════════
// Builder — configure stack size before spawning
// ═══════════════════════════════════════════════════════════════════════

/// Thread builder.  Allows setting stack size before spawning.
///
/// # Example
/// ```ignore
/// use libmorpheus::thread;
/// let h = thread::Builder::new()
///     .stack_size(128 * 1024) // 128 KiB
///     .spawn(|| { /* work */ })?;
/// h.join()?;
/// ```
pub struct Builder {
    stack_pages: u64,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder {
    pub fn new() -> Self {
        Self {
            stack_pages: DEFAULT_STACK_PAGES,
        }
    }

    /// Set the stack size in bytes.  Rounded up to page boundary.
    pub fn stack_size(mut self, bytes: usize) -> Self {
        self.stack_pages = (bytes as u64).div_ceil(4096);
        if self.stack_pages == 0 {
            self.stack_pages = 1;
        }
        self
    }

    /// Spawn a thread with these settings.
    pub fn spawn<F>(self, f: F) -> Result<JoinHandle<()>, u64>
    where
        F: FnOnce() + Send + 'static,
    {
        spawn_with_stack(f, self.stack_pages)
    }
}

/// Spawn with a custom stack size (in pages).
fn spawn_with_stack<F>(f: F, stack_pages: u64) -> Result<JoinHandle<()>, u64>
where
    F: FnOnce() + Send + 'static,
{
    let stack_size = stack_pages * 4096;

    let stack_base = unsafe { syscall1(SYS_MMAP, stack_pages) };
    if crate::is_error(stack_base) {
        return Err(stack_base);
    }

    let stack_top = (stack_base + stack_size) & !0xF;

    let closure: Box<Box<dyn FnOnce()>> = Box::new(Box::new(f));
    let arg = Box::into_raw(closure) as u64;

    let entry = thread_trampoline as *const () as u64;
    let ret = unsafe { syscall3(SYS_THREAD_CREATE, entry, stack_top, arg) };

    if crate::is_error(ret) {
        unsafe {
            let _ = Box::from_raw(arg as *mut Box<dyn FnOnce()>);
            syscall2(SYS_MUNMAP, stack_base, stack_pages);
        }
        return Err(ret);
    }

    Ok(JoinHandle {
        tid: ret as u32,
        _marker: PhantomData,
    })
}

/// Get the current thread's TID.
pub fn current_tid() -> u32 {
    crate::process::getpid()
}

/// Yield the current thread's time slice.
pub fn yield_now() {
    crate::process::yield_cpu();
}

/// Sleep the current thread for `millis` milliseconds.
pub fn sleep_ms(millis: u64) {
    crate::process::sleep(millis);
}
