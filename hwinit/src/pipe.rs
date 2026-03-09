//! Kernel pipe infrastructure — fixed ring buffers for IPC.
//!
//! Each pipe is a 4 KiB ring buffer with reader/writer reference counts.
//! `SYS_PIPE` allocates a pipe and returns two fds (read end, write end).
//! Reading from an empty pipe with live writers blocks; reading with no
//! writers returns 0 (EOF).  Writing to a pipe with no readers returns EPIPE.

/// Maximum number of simultaneous pipes.
pub const MAX_PIPES: usize = 16;

/// Per-pipe buffer capacity (must be power of two).
const PIPE_BUF_SIZE: usize = 4096;
const PIPE_BUF_MASK: usize = PIPE_BUF_SIZE - 1;

/// A kernel pipe — SPSC ring buffer with refcounts.
pub struct Pipe {
    buf: [u8; PIPE_BUF_SIZE],
    head: usize,
    tail: usize,
    /// Number of open read-end file descriptors.
    pub readers: u8,
    /// Number of open write-end file descriptors.
    pub writers: u8,
    /// True if this pipe slot is allocated.
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

    /// Bytes available to read.
    #[inline]
    pub fn available_read(&self) -> usize {
        (self.head.wrapping_sub(self.tail)) & PIPE_BUF_MASK
    }

    /// Free space available for writing.
    #[inline]
    pub fn available_write(&self) -> usize {
        PIPE_BUF_SIZE - 1 - self.available_read()
    }

    /// Write data into the pipe.  Returns the number of bytes written.
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

    /// Read data from the pipe.  Returns the number of bytes read.
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

/// Global pipe table.
static mut PIPE_TABLE: [Pipe; MAX_PIPES] = [const { Pipe::empty() }; MAX_PIPES];

// smp: without this, two cores writing to the same pipe corrupt head/tail.
static PIPE_LOCK: crate::sync::RawSpinLock = crate::sync::RawSpinLock::new();

/// Allocate a new pipe.  Returns the pipe index, or None if full.
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

/// Write to a pipe.
///
/// # Safety
/// `idx` must be a valid pipe index.
pub unsafe fn pipe_write(idx: u8, data: &[u8]) -> usize {
    PIPE_LOCK.lock();
    let n = if let Some(pipe) = PIPE_TABLE.get_mut(idx as usize) {
        if !pipe.active {
            0
        } else {
            pipe.write(data)
        }
    } else {
        0
    };
    PIPE_LOCK.unlock();
    n
}

/// Read from a pipe.
///
/// # Safety
/// `idx` must be a valid pipe index.
pub unsafe fn pipe_read(idx: u8, buf: &mut [u8]) -> usize {
    PIPE_LOCK.lock();
    let n = if let Some(pipe) = PIPE_TABLE.get_mut(idx as usize) {
        if !pipe.active {
            0
        } else {
            pipe.read(buf)
        }
    } else {
        0
    };
    PIPE_LOCK.unlock();
    n
}

/// Get the number of open writers for a pipe.
pub unsafe fn pipe_writers(idx: u8) -> u8 {
    PIPE_LOCK.lock();
    let n = PIPE_TABLE.get(idx as usize).map(|p| p.writers).unwrap_or(0);
    PIPE_LOCK.unlock();
    n
}

/// Get the number of open readers for a pipe.
pub unsafe fn pipe_readers(idx: u8) -> u8 {
    PIPE_LOCK.lock();
    let n = PIPE_TABLE.get(idx as usize).map(|p| p.readers).unwrap_or(0);
    PIPE_LOCK.unlock();
    n
}

/// Bytes available to read from a pipe (non-blocking check).
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

/// Close the read end of a pipe.  Frees the pipe if both ends are closed.
pub unsafe fn pipe_close_reader(idx: u8) {
    PIPE_LOCK.lock();
    if let Some(pipe) = PIPE_TABLE.get_mut(idx as usize) {
        if pipe.readers > 0 {
            pipe.readers -= 1;
        }
        if pipe.readers == 0 && pipe.writers == 0 {
            pipe.active = false;
        }
    }
    PIPE_LOCK.unlock();
}

/// Close the write end of a pipe.  Frees the pipe if both ends are closed.
pub unsafe fn pipe_close_writer(idx: u8) {
    PIPE_LOCK.lock();
    if let Some(pipe) = PIPE_TABLE.get_mut(idx as usize) {
        if pipe.writers > 0 {
            pipe.writers -= 1;
        }
        if pipe.readers == 0 && pipe.writers == 0 {
            pipe.active = false;
        }
    }
    PIPE_LOCK.unlock();
}

/// Increment the reader refcount (used by fd inheritance / dup).
pub unsafe fn pipe_add_reader(idx: u8) {
    PIPE_LOCK.lock();
    if let Some(pipe) = PIPE_TABLE.get_mut(idx as usize) {
        if pipe.active {
            pipe.readers = pipe.readers.saturating_add(1);
        }
    }
    PIPE_LOCK.unlock();
}

/// Increment the writer refcount (used by fd inheritance / dup).
pub unsafe fn pipe_add_writer(idx: u8) {
    PIPE_LOCK.lock();
    if let Some(pipe) = PIPE_TABLE.get_mut(idx as usize) {
        if pipe.active {
            pipe.writers = pipe.writers.saturating_add(1);
        }
    }
    PIPE_LOCK.unlock();
}
