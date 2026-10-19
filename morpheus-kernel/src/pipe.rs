//! Kernel pipe infrastructure — fixed ring buffers for IPC.
//!
//! Each pipe is a 4 KiB ring buffer with reader/writer reference counts.
//! `SYS_PIPE` allocates a pipe and returns two fds (read end, write end).
//! Reading from an empty pipe with live writers blocks; reading with no
//! writers returns 0 (EOF). Writing to a pipe with no readers returns EPIPE.

use core::sync::atomic::{AtomicU32, Ordering};

/// Maximum number of simultaneous pipes. Bounded by the 32-bit deferred-close
/// bitmaps below.
pub const MAX_PIPES: usize = 16;
const _: () = assert!(MAX_PIPES <= 32);

/// Per-pipe buffer capacity (must be power of two).
const PIPE_BUF_SIZE: usize = 4096;
const PIPE_BUF_MASK: usize = PIPE_BUF_SIZE - 1;

/// SPSC ring buffer with reader/writer refcounts.
pub struct Pipe {
    buf: [u8; PIPE_BUF_SIZE],
    head: usize,
    tail: usize,
    pub readers: u8,
    pub writers: u8,
    pub active: bool,
}

impl Pipe {
    pub const fn empty() -> Self {
        Self {
            buf: [0; PIPE_BUF_SIZE],
            head: 0,
            tail: 0,
            readers: 0,
            writers: 0,
            active: false,
        }
    }

    #[inline]
    pub fn available_read(&self) -> usize {
        (self.head.wrapping_sub(self.tail)) & PIPE_BUF_MASK
    }

    #[inline]
    pub fn available_write(&self) -> usize {
        PIPE_BUF_SIZE - 1 - self.available_read()
    }

    /// Returns bytes written.
    pub fn write(&mut self, data: &[u8]) -> usize {
        let mut written = 0;
        for &byte in data {
            let next = (self.head + 1) & PIPE_BUF_MASK;
            if next == self.tail {
                break; // full
            }
            self.buf[self.head] = byte;
            self.head = next;
            written += 1;
        }
        written
    }

    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        let mut count = 0;
        while count < buf.len() {
            if self.tail == self.head {
                break; // empty
            }
            buf[count] = self.buf[self.tail];
            self.tail = (self.tail + 1) & PIPE_BUF_MASK;
            count += 1;
        }
        count
    }
}

static mut PIPE_TABLE: [Pipe; MAX_PIPES] = [const { Pipe::empty() }; MAX_PIPES];

// smp: without this, two cores writing to the same pipe corrupt head/tail.
static PIPE_LOCK: crate::sync::RawSpinLock = crate::sync::RawSpinLock::new();

/// Returns the pipe index, or None if the table is full.
///
/// # Safety
/// Must be called with interrupts disabled (syscall context).
pub unsafe fn pipe_alloc() -> Option<u8> {
    PIPE_LOCK.lock();
    for (i, pipe) in PIPE_TABLE.iter_mut().enumerate() {
        if !pipe.active {
            *pipe = Pipe::empty();
            pipe.active = true;
            pipe.readers = 1;
            pipe.writers = 1;
            PIPE_LOCK.unlock();
            return Some(i as u8);
        }
    }
    PIPE_LOCK.unlock();
    None
}

/// Re-publish a pipe's level-triggered epoll readiness from its live fill level.
/// Called OUTSIDE `PIPE_LOCK`: `set_ready` takes `PROCESS_TABLE_LOCK` to wake
/// parked waiters, and nesting that under `PIPE_LOCK` would invert lock order.
unsafe fn publish_readiness(idx: u8, avail_read: usize, avail_write: usize) {
    use crate::io::readiness::{clear_ready, pipe_token, set_ready};
    use morpheus_foundation::flags::{EPOLLIN, EPOLLOUT};
    let token = pipe_token(idx);
    if avail_read > 0 {
        set_ready(token, EPOLLIN);
    } else {
        clear_ready(token, EPOLLIN);
    }
    if avail_write > 0 {
        set_ready(token, EPOLLOUT);
    } else {
        clear_ready(token, EPOLLOUT);
    }
}

/// # Safety
/// Caller must hold no lock that would invert order with `PIPE_LOCK`; the
/// static-mut `PIPE_TABLE` is only ever touched under `PIPE_LOCK`, taken and
/// released within this fn, so concurrent callers on other cores serialize here.
pub unsafe fn pipe_write(idx: u8, data: &[u8]) -> usize {
    PIPE_LOCK.lock();
    let mut levels = None;
    let n = if let Some(pipe) = PIPE_TABLE.get_mut(idx as usize) {
        if !pipe.active {
            0
        } else {
            let n = pipe.write(data);
            levels = Some((pipe.available_read(), pipe.available_write()));
            n
        }
    } else {
        0
    };
    PIPE_LOCK.unlock();
    if let Some((ar, aw)) = levels {
        publish_readiness(idx, ar, aw);
    }
    n
}

/// # Safety
/// Caller must hold no lock that would invert order with `PIPE_LOCK`; the
/// static-mut `PIPE_TABLE` is only ever touched under `PIPE_LOCK`, taken and
/// released within this fn, so concurrent callers on other cores serialize here.
pub unsafe fn pipe_read(idx: u8, buf: &mut [u8]) -> usize {
    PIPE_LOCK.lock();
    let mut levels = None;
    let n = if let Some(pipe) = PIPE_TABLE.get_mut(idx as usize) {
        if !pipe.active {
            0
        } else {
            let n = pipe.read(buf);
            levels = Some((pipe.available_read(), pipe.available_write()));
            n
        }
    } else {
        0
    };
    PIPE_LOCK.unlock();
    if let Some((ar, aw)) = levels {
        publish_readiness(idx, ar, aw);
    }
    n
}

// SAFETY: static-mut `PIPE_TABLE` is only read here under `PIPE_LOCK`, held
// for the duration of the access below.
pub unsafe fn pipe_writers(idx: u8) -> u8 {
    PIPE_LOCK.lock();
    let n = PIPE_TABLE.get(idx as usize).map(|p| p.writers).unwrap_or(0);
    PIPE_LOCK.unlock();
    n
}

// SAFETY: static-mut `PIPE_TABLE` is only read here under `PIPE_LOCK`, held
// for the duration of the access below.
pub unsafe fn pipe_readers(idx: u8) -> u8 {
    PIPE_LOCK.lock();
    let n = PIPE_TABLE.get(idx as usize).map(|p| p.readers).unwrap_or(0);
    PIPE_LOCK.unlock();
    n
}

/// Bytes available to read from a pipe (non-blocking check).
// SAFETY: static-mut `PIPE_TABLE` is only read here under `PIPE_LOCK`, held
// for the duration of the access below.
pub unsafe fn pipe_available(idx: u8) -> usize {
    PIPE_LOCK.lock();
    let n = PIPE_TABLE
        .get(idx as usize)
        .filter(|p| p.active)
        .map(|p| p.available_read())
        .unwrap_or(0);
    PIPE_LOCK.unlock();
    n
}

/// Drop one endpoint refcount; frees the pipe when both sides are gone. Returns
/// true when `idx` just lost its LAST reader/writer (the peer must be told).
// SAFETY: static-mut `PIPE_TABLE` is only mutated here under `PIPE_LOCK`, held
// for the duration of the access below.
unsafe fn close_end(idx: u8, writer: bool) -> bool {
    PIPE_LOCK.lock();
    let mut last = false;
    if let Some(pipe) = PIPE_TABLE.get_mut(idx as usize) {
        let n = if writer {
            &mut pipe.writers
        } else {
            &mut pipe.readers
        };
        if *n > 0 {
            *n -= 1;
        }
        last = *n == 0;
        if pipe.readers == 0 && pipe.writers == 0 {
            pipe.active = false;
        }
    }
    PIPE_LOCK.unlock();
    last
}

/// Readiness a peer must see when the last reader (EPIPE) / last writer (EOF)
/// goes away.
const READER_GONE_MASK: u32 =
    morpheus_foundation::flags::EPOLLERR | morpheus_foundation::flags::EPOLLOUT;
const WRITER_GONE_MASK: u32 =
    morpheus_foundation::flags::EPOLLHUP | morpheus_foundation::flags::EPOLLIN;

/// Close events whose readiness publication was deferred by a `*_quiet` close
/// (one bit per pipe; `MAX_PIPES` <= 32). `set_ready` takes `PROCESS_TABLE_LOCK`
/// to wake parked waiters, so a close that runs UNDER that lock (process exit,
/// spawn file_actions replay, in-ISR signal kill) must not publish inline: the
/// nested acquire of a non-recursive spinlock hangs every core. The closer parks
/// the event here and the caller drains it after unlock; `scheduler_tick` also
/// drains as a catch-all so an event parked from ISR context is never lost.
static DEFER_READER_GONE: AtomicU32 = AtomicU32::new(0);
static DEFER_WRITER_GONE: AtomicU32 = AtomicU32::new(0);

/// # Safety
/// Caller must hold no lock that would invert order with `PIPE_LOCK`, and must
/// NOT hold `PROCESS_TABLE_LOCK` (the readiness publish takes it).
pub unsafe fn pipe_close_reader(idx: u8) {
    if close_end(idx, false) {
        crate::io::readiness::set_ready(crate::io::readiness::pipe_token(idx), READER_GONE_MASK);
    }
}

/// # Safety
/// Caller must hold no lock that would invert order with `PIPE_LOCK`, and must
/// NOT hold `PROCESS_TABLE_LOCK` (the readiness publish takes it).
pub unsafe fn pipe_close_writer(idx: u8) {
    if close_end(idx, true) {
        crate::io::readiness::set_ready(crate::io::readiness::pipe_token(idx), WRITER_GONE_MASK);
    }
}

/// `pipe_close_reader` for callers holding `PROCESS_TABLE_LOCK`: the EPIPE
/// readiness is parked and published by `publish_pending_close_events`.
///
/// # Safety
/// Caller must hold no lock that would invert order with `PIPE_LOCK`.
pub unsafe fn pipe_close_reader_quiet(idx: u8) {
    if close_end(idx, false) {
        DEFER_READER_GONE.fetch_or(1 << idx, Ordering::AcqRel);
    }
}

/// `pipe_close_writer` for callers holding `PROCESS_TABLE_LOCK`: the EOF
/// readiness is parked and published by `publish_pending_close_events`.
///
/// # Safety
/// Caller must hold no lock that would invert order with `PIPE_LOCK`.
pub unsafe fn pipe_close_writer_quiet(idx: u8) {
    if close_end(idx, true) {
        DEFER_WRITER_GONE.fetch_or(1 << idx, Ordering::AcqRel);
    }
}

/// Publish every close event parked by a `*_quiet` close. Must be called with
/// `PROCESS_TABLE_LOCK` released. Idempotent and cheap when nothing is pending,
/// so the tick calls it unconditionally.
pub fn publish_pending_close_events() {
    use crate::io::readiness::{pipe_token, set_ready};
    let mut readers = DEFER_READER_GONE.swap(0, Ordering::AcqRel);
    let mut writers = DEFER_WRITER_GONE.swap(0, Ordering::AcqRel);
    while readers != 0 {
        let idx = readers.trailing_zeros() as u8;
        readers &= readers - 1;
        set_ready(pipe_token(idx), READER_GONE_MASK);
    }
    while writers != 0 {
        let idx = writers.trailing_zeros() as u8;
        writers &= writers - 1;
        set_ready(pipe_token(idx), WRITER_GONE_MASK);
    }
}

/// Bump reader refcount (fd inheritance / dup).
// SAFETY: static-mut `PIPE_TABLE` is only mutated here under `PIPE_LOCK`,
// held for the duration of the access below.
pub unsafe fn pipe_add_reader(idx: u8) {
    PIPE_LOCK.lock();
    if let Some(pipe) = PIPE_TABLE.get_mut(idx as usize) {
        if pipe.active {
            pipe.readers = pipe.readers.saturating_add(1);
        }
    }
    PIPE_LOCK.unlock();
}

/// Bump writer refcount (fd inheritance / dup).
// SAFETY: static-mut `PIPE_TABLE` is only mutated here under `PIPE_LOCK`,
// held for the duration of the access below.
pub unsafe fn pipe_add_writer(idx: u8) {
    PIPE_LOCK.lock();
    if let Some(pipe) = PIPE_TABLE.get_mut(idx as usize) {
        if pipe.active {
            pipe.writers = pipe.writers.saturating_add(1);
        }
    }
    PIPE_LOCK.unlock();
}
