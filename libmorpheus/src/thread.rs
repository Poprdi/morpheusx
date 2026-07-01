//! Threads share parent's page tables (shared CR3); each has its own user stack
//! (SYS_MMAP) and kernel stack. Default user stack: 64 KiB, page-aligned.

extern crate alloc;

use alloc::boxed::Box;
use core::marker::PhantomData;

use crate::raw::*;

const DEFAULT_STACK_PAGES: u64 = 16;
const DEFAULT_STACK_SIZE: u64 = DEFAULT_STACK_PAGES * 4096;

/// Set this thread's TLS base (x86 FS base) — the `arch_prctl(ARCH_SET_FS)`
/// primitive. `tp` must point at the thread's variant-II TCB (FS-relative TLS
/// data sits below it). Low-level; crt0/std own the TCB layout. Must be a
/// canonical user address (`< 0x0000_8000_0000_0000`) or the kernel returns
/// EINVAL. `tp == 0` clears TLS.
///
/// # Safety
/// `tp` must reference a valid, correctly laid-out TCB that outlives the thread,
/// or any subsequent `#[thread_local]` access is UB.
pub unsafe fn set_thread_pointer(tp: u64) -> Result<(), u64> {
    let ret = syscall1(SYS_SET_THREAD_POINTER, tp);
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Read this thread's TLS base (x86 FS base) via the variant-II self-pointer at
/// `%fs:0` (`install_thread_tls` stores `*(tp) = tp`). Pure userspace — no
/// syscall. Useful to save/restore the thread pointer around code that
/// temporarily reprograms it.
///
/// # Safety
/// TLS must already be installed for this thread (crt0 or `thread_trampoline`);
/// with `fs:base == 0` this dereferences address 0 and faults.
#[inline]
pub unsafe fn get_thread_pointer() -> u64 {
    let tp: u64;
    core::arch::asm!("mov {}, fs:0", out(reg) tp, options(nostack, readonly, preserves_flags));
    tp
}

/// Must be joined; otherwise the thread is detached on drop.
pub struct JoinHandle<T> {
    tid: u32,
    _marker: PhantomData<T>,
}

impl<T> JoinHandle<T> {
    /// Block until the thread exits; returns exit code. Consumes the handle so the
    /// detach-on-drop below does not also fire (join already reaps the tid).
    pub fn join(self) -> Result<u64, u64> {
        let tid = self.tid;
        core::mem::forget(self);
        let ret = unsafe { syscall1(SYS_THREAD_JOIN, tid as u64) };
        if crate::is_error(ret) {
            Err(ret)
        } else {
            Ok(ret)
        }
    }

    /// Same as PID in the kernel process table.
    pub fn tid(&self) -> u32 {
        self.tid
    }
}

/// A handle dropped without `join` detaches the thread so the kernel auto-reaps it
/// on exit — a dropped handle never leaks a table slot. Idempotent vs a finished
/// thread (returns ESRCH, ignored).
impl<T> Drop for JoinHandle<T> {
    fn drop(&mut self) {
        unsafe {
            syscall1(SYS_THREAD_DETACH, self.tid as u64);
        }
    }
}

/// Kernel jumps here with `rdi=arg`. `arg` is `Box<Box<dyn FnOnce()>>`.
/// extern "C" for stable ABI with the kernel's thread setup.
unsafe extern "C" fn thread_trampoline(arg: u64) -> ! {
    // Threads don't run crt0; each sets up its own variant-II TLS block before
    // touching any `#[thread_local]`.
    crate::entry::install_thread_tls();
    let closure: Box<Box<dyn FnOnce()>> = Box::from_raw(arg as *mut Box<dyn FnOnce()>);
    (*closure)();

    syscall1(SYS_THREAD_EXIT, 0);
    unsafe { core::hint::unreachable_unchecked() }
}

pub fn spawn<F>(f: F) -> Result<JoinHandle<()>, u64>
where
    F: FnOnce() + Send + 'static,
{
    let stack_base = crate::mem::mmap_raw(DEFAULT_STACK_PAGES);
    if crate::is_error(stack_base) {
        return Err(stack_base);
    }

    // Stack grows down; top must be 16-byte aligned per SysV ABI.
    let stack_top = (stack_base + DEFAULT_STACK_SIZE) & !0xF;

    // Double-box: inner for type erasure, outer for stable thin pointer.
    let closure: Box<Box<dyn FnOnce()>> = Box::new(Box::new(f));
    let arg = Box::into_raw(closure) as u64;

    let entry = thread_trampoline as *const () as u64;
    // tls_base=0: the trampoline installs its own variant-II TLS. ctid_ptr=0:
    // this handle joins via SYS_THREAD_JOIN (the ctid futex slot is for std).
    // flags=0: not detached at creation (drop-detach handles leak-free drop).
    let ret = unsafe { syscall6(SYS_THREAD_CREATE, entry, stack_top, arg, 0, 0, 0) };

    if crate::is_error(ret) {
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

    /// Rounded up to page boundary.
    pub fn stack_size(mut self, bytes: usize) -> Self {
        self.stack_pages = (bytes as u64).div_ceil(4096);
        if self.stack_pages == 0 {
            self.stack_pages = 1;
        }
        self
    }

    pub fn spawn<F>(self, f: F) -> Result<JoinHandle<()>, u64>
    where
        F: FnOnce() + Send + 'static,
    {
        spawn_with_stack(f, self.stack_pages)
    }
}

fn spawn_with_stack<F>(f: F, stack_pages: u64) -> Result<JoinHandle<()>, u64>
where
    F: FnOnce() + Send + 'static,
{
    let stack_size = stack_pages * 4096;

    let stack_base = crate::mem::mmap_raw(stack_pages);
    if crate::is_error(stack_base) {
        return Err(stack_base);
    }

    let stack_top = (stack_base + stack_size) & !0xF;

    let closure: Box<Box<dyn FnOnce()>> = Box::new(Box::new(f));
    let arg = Box::into_raw(closure) as u64;

    let entry = thread_trampoline as *const () as u64;
    let ret = unsafe { syscall6(SYS_THREAD_CREATE, entry, stack_top, arg, 0, 0, 0) };

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

pub fn current_tid() -> u32 {
    crate::process::getpid()
}

pub fn yield_now() {
    crate::process::yield_cpu();
}

pub fn sleep_ms(millis: u64) {
    crate::process::sleep(millis);
}
