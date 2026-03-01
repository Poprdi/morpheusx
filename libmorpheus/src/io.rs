//! Console I/O — read from stdin, write to stdout.
//!
//! This module provides:
//! - Raw functions: [`print`], [`println`], [`write_fd`], [`read_fd`], etc.
//! - Traits: [`Read`], [`Write`], [`Seek`], [`BufRead`]
//! - Buffered adapters: [`BufReader`], [`BufWriter`]
//! - Typed handles: [`Stdin`], [`Stdout`], [`Stderr`]
//! - Formatting macros: `print!`, `println!`, `eprint!`, `eprintln!`

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::error::{self, Error, ErrorKind};
use crate::raw::*;

/// Write a string to the kernel console (serial port, fd 1 = stdout).
pub fn print(s: &str) {
    if s.is_empty() {
        return;
    }
    unsafe {
        syscall3(SYS_WRITE, 1, s.as_ptr() as u64, s.len() as u64);
    }
}

/// Write a string followed by a newline.
pub fn println(s: &str) {
    print(s);
    print("\n");
}

/// Write to a specific fd.
pub fn write_fd(fd: u32, data: &[u8]) -> Result<usize, u64> {
    if data.is_empty() {
        return Ok(0);
    }
    let ret = unsafe {
        syscall3(
            SYS_WRITE,
            fd as u64,
            data.as_ptr() as u64,
            data.len() as u64,
        )
    };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}

/// Read from a specific fd.
pub fn read_fd(fd: u32, buf: &mut [u8]) -> Result<usize, u64> {
    if buf.is_empty() {
        return Ok(0);
    }
    let ret = unsafe {
        syscall3(
            SYS_READ,
            fd as u64,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
        )
    };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}

/// Blocking read from stdin (fd 0).
///
/// Returns the number of bytes read.  Blocks until at least one byte
/// is available (keyboard input).
pub fn read_stdin(buf: &mut [u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }
    let ret = unsafe { syscall3(SYS_READ, 0, buf.as_mut_ptr() as u64, buf.len() as u64) };
    if crate::is_error(ret) {
        0
    } else {
        ret as usize
    }
}

/// Read a line from stdin into `buf`, echoing characters to stdout.
///
/// Returns the number of bytes read (excluding the trailing newline).
/// The newline is NOT included in the buffer.
pub fn read_line(buf: &mut [u8]) -> usize {
    let mut pos = 0;
    let mut ch = [0u8; 1];
    loop {
        let n = read_stdin(&mut ch);
        if n == 0 {
            crate::process::yield_cpu();
            continue;
        }
        match ch[0] {
            b'\r' | b'\n' => {
                print("\n");
                return pos;
            }
            // Backspace (0x08 or 0x7F)
            0x08 | 0x7F => {
                if pos > 0 {
                    pos -= 1;
                    print("\x08 \x08"); // erase character
                }
            }
            c => {
                if pos < buf.len() {
                    buf[pos] = c;
                    pos += 1;
                    // Echo the character.
                    let s = unsafe { core::str::from_utf8_unchecked(core::slice::from_ref(&c)) };
                    print(s);
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Formatting support — print!(), println!(), eprint!(), eprintln!()
// ═══════════════════════════════════════════════════════════════════════

/// Stdout handle for `core::fmt::Write`.  Zero-size — formatting goes
/// directly to serial via `SYS_WRITE(1, ...)`.
pub struct Stdout;

/// Stderr handle.  Goes to fd 2.
pub struct Stderr;

/// Stdin handle.
pub struct Stdin;

impl core::fmt::Write for Stdout {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        print(s);
        Ok(())
    }
}

impl core::fmt::Write for Stderr {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        if !s.is_empty() {
            unsafe {
                syscall3(SYS_WRITE, 2, s.as_ptr() as u64, s.len() as u64);
            }
        }
        Ok(())
    }
}

/// Internal: write formatted args to stdout.
#[doc(hidden)]
pub fn _print_fmt(args: core::fmt::Arguments<'_>) {
    let _ = core::fmt::Write::write_fmt(&mut Stdout, args);
}

/// Internal: write formatted args to stderr.
#[doc(hidden)]
pub fn _eprint_fmt(args: core::fmt::Arguments<'_>) {
    let _ = core::fmt::Write::write_fmt(&mut Stderr, args);
}

/// Print formatted text to stdout (serial console).
///
/// # Example
/// ```ignore
/// use libmorpheus::print;
/// print!("PID: {}, status: {}\n", pid, status);
/// ```
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::io::_print_fmt(format_args!($($arg)*))
    };
}

/// Print formatted text to stdout with a trailing newline.
///
/// # Example
/// ```ignore
/// use libmorpheus::println;
/// println!("Hello, {}!", name);
/// ```
#[macro_export]
macro_rules! println {
    () => { $crate::io::_print_fmt(format_args!("\n")) };
    ($($arg:tt)*) => {
        $crate::io::_print_fmt(format_args!("{}\n", format_args!($($arg)*)))
    };
}

/// Print formatted text to stderr.
#[macro_export]
macro_rules! eprint {
    ($($arg:tt)*) => {
        $crate::io::_eprint_fmt(format_args!($($arg)*))
    };
}

/// Print formatted text to stderr with a trailing newline.
#[macro_export]
macro_rules! eprintln {
    () => { $crate::io::_eprint_fmt(format_args!("\n")) };
    ($($arg:tt)*) => {
        $crate::io::_eprint_fmt(format_args!("{}\n", format_args!($($arg)*)))
    };
}

// ═══════════════════════════════════════════════════════════════════════
// I/O traits — Read, Write, Seek, BufRead
// ═══════════════════════════════════════════════════════════════════════

/// Seek reference point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekFrom {
    /// Absolute offset from file start.
    Start(u64),
    /// Relative offset from current position.
    Current(i64),
    /// Relative offset from file end.
    End(i64),
}

/// Read bytes from a source.
pub trait Read {
    /// Pull bytes into `buf`.  Returns number of bytes read (0 = EOF).
    fn read(&mut self, buf: &mut [u8]) -> error::Result<usize>;

    /// Read exactly `buf.len()` bytes, or error.
    fn read_exact(&mut self, mut buf: &mut [u8]) -> error::Result<()> {
        while !buf.is_empty() {
            match self.read(buf) {
                Ok(0) => return Err(Error::new(ErrorKind::UnexpectedEof)),
                Ok(n) => buf = &mut buf[n..],
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Read all remaining bytes into a Vec.
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> error::Result<usize> {
        let start_len = buf.len();
        let mut tmp = [0u8; 4096];
        loop {
            match self.read(&mut tmp) {
                Ok(0) => return Ok(buf.len() - start_len),
                Ok(n) => buf.extend_from_slice(&tmp[..n]),
                Err(e) => return Err(e),
            }
        }
    }

    /// Read all remaining bytes into a String.
    fn read_to_string(&mut self, buf: &mut String) -> error::Result<usize> {
        let mut bytes = Vec::new();
        let n = self.read_to_end(&mut bytes)?;
        match core::str::from_utf8(&bytes) {
            Ok(s) => {
                buf.push_str(s);
                Ok(n)
            }
            Err(_) => Err(Error::new(ErrorKind::InvalidInput)),
        }
    }
}

/// Write bytes to a sink.
pub trait Write {
    /// Write bytes from `buf`.  Returns number of bytes written.
    fn write(&mut self, buf: &[u8]) -> error::Result<usize>;

    /// Flush buffered data.  No-op for unbuffered writers.
    fn flush(&mut self) -> error::Result<()>;

    /// Write the entire buffer, looping if needed.
    fn write_all(&mut self, mut buf: &[u8]) -> error::Result<()> {
        while !buf.is_empty() {
            match self.write(buf) {
                Ok(0) => return Err(Error::new(ErrorKind::WriteZero)),
                Ok(n) => buf = &buf[n..],
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Write a formatted string.
    fn write_fmt(&mut self, fmt: core::fmt::Arguments<'_>) -> error::Result<()> {
        // Use an adapter to bridge core::fmt::Write → io::Write.
        struct Adapter<'a, W: ?Sized + Write> {
            inner: &'a mut W,
            error: Option<Error>,
        }
        impl<W: ?Sized + Write> core::fmt::Write for Adapter<'_, W> {
            fn write_str(&mut self, s: &str) -> core::fmt::Result {
                match self.inner.write_all(s.as_bytes()) {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        self.error = Some(e);
                        Err(core::fmt::Error)
                    }
                }
            }
        }
        let mut adapter = Adapter {
            inner: self,
            error: None,
        };
        match core::fmt::Write::write_fmt(&mut adapter, fmt) {
            Ok(()) => Ok(()),
            Err(_) => Err(adapter.error.unwrap_or(Error::new(ErrorKind::Io))),
        }
    }
}

/// Seek to an offset within a stream.
pub trait Seek {
    /// Seek to a position.  Returns the new absolute offset.
    fn seek(&mut self, pos: SeekFrom) -> error::Result<u64>;

    /// Return the current stream position.
    fn stream_position(&mut self) -> error::Result<u64> {
        self.seek(SeekFrom::Current(0))
    }
}

/// Read with an internal buffer.
pub trait BufRead: Read {
    /// Return the internal buffer contents (may be empty).
    fn fill_buf(&mut self) -> error::Result<&[u8]>;

    /// Mark `amt` bytes as consumed.
    fn consume(&mut self, amt: usize);

    /// Read until `byte` is found or EOF.  The delimiter is included.
    fn read_until(&mut self, byte: u8, buf: &mut Vec<u8>) -> error::Result<usize> {
        let start = buf.len();
        loop {
            let available = match self.fill_buf() {
                Ok(b) => b,
                Err(e) => return Err(e),
            };
            if available.is_empty() {
                return Ok(buf.len() - start);
            }
            let used = match available.iter().position(|&b| b == byte) {
                Some(i) => {
                    buf.extend_from_slice(&available[..=i]);
                    i + 1
                }
                None => {
                    buf.extend_from_slice(available);
                    available.len()
                }
            };
            self.consume(used);
            if buf.last() == Some(&byte) {
                return Ok(buf.len() - start);
            }
        }
    }

    /// Read a line (including `\n`) into `buf`.
    fn read_line(&mut self, buf: &mut String) -> error::Result<usize> {
        let mut bytes = Vec::new();
        let n = self.read_until(b'\n', &mut bytes)?;
        match core::str::from_utf8(&bytes) {
            Ok(s) => {
                buf.push_str(s);
                Ok(n)
            }
            Err(_) => Err(Error::new(ErrorKind::InvalidInput)),
        }
    }

    /// Return an iterator over the lines of this reader.
    fn lines(self) -> Lines<Self>
    where
        Self: Sized,
    {
        Lines { inner: self }
    }
}

/// Iterator over lines from a [`BufRead`].
pub struct Lines<B> {
    inner: B,
}

impl<B: BufRead> Iterator for Lines<B> {
    type Item = error::Result<String>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut buf = String::new();
        match self.inner.read_line(&mut buf) {
            Ok(0) => None,
            Ok(_) => {
                // Strip trailing newline.
                if buf.ends_with('\n') {
                    buf.pop();
                    if buf.ends_with('\r') {
                        buf.pop();
                    }
                }
                Some(Ok(buf))
            }
            Err(e) => Some(Err(e)),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Trait impls for Stdin / Stdout / Stderr
// ═══════════════════════════════════════════════════════════════════════

impl Read for Stdin {
    fn read(&mut self, buf: &mut [u8]) -> error::Result<usize> {
        read_fd(0, buf).map_err(Error::from_raw)
    }
}

impl Write for Stdout {
    fn write(&mut self, buf: &[u8]) -> error::Result<usize> {
        write_fd(1, buf).map_err(Error::from_raw)
    }
    fn flush(&mut self) -> error::Result<()> {
        Ok(())
    }
}

impl Write for Stderr {
    fn write(&mut self, buf: &[u8]) -> error::Result<usize> {
        write_fd(2, buf).map_err(Error::from_raw)
    }
    fn flush(&mut self) -> error::Result<()> {
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════
// BufReader<R>
// ═══════════════════════════════════════════════════════════════════════

const DEFAULT_BUF_SIZE: usize = 4096;

/// Wraps a [`Read`] with an internal buffer for efficient small reads.
pub struct BufReader<R> {
    inner: R,
    buf: Vec<u8>,
    pos: usize,
    filled: usize,
}

impl<R: Read> BufReader<R> {
    /// Create with default 4 KiB buffer.
    pub fn new(inner: R) -> Self {
        Self::with_capacity(DEFAULT_BUF_SIZE, inner)
    }

    /// Create with custom buffer size.
    pub fn with_capacity(cap: usize, inner: R) -> Self {
        Self {
            inner,
            buf: vec![0u8; cap],
            pos: 0,
            filled: 0,
        }
    }

    /// Get a reference to the underlying reader.
    pub fn get_ref(&self) -> &R {
        &self.inner
    }

    /// Get a mutable reference to the underlying reader.
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    /// Consume self and return the underlying reader.
    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<R: Read> Read for BufReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> error::Result<usize> {
        // If buf is larger than internal buffer and nothing is buffered,
        // bypass the internal buffer entirely.
        if self.pos == self.filled && buf.len() >= self.buf.len() {
            return self.inner.read(buf);
        }
        let available = self.fill_buf()?;
        let n = available.len().min(buf.len());
        buf[..n].copy_from_slice(&available[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl<R: Read> BufRead for BufReader<R> {
    fn fill_buf(&mut self) -> error::Result<&[u8]> {
        if self.pos >= self.filled {
            self.pos = 0;
            self.filled = self.inner.read(&mut self.buf)?;
        }
        Ok(&self.buf[self.pos..self.filled])
    }

    fn consume(&mut self, amt: usize) {
        self.pos = (self.pos + amt).min(self.filled);
    }
}

// ═══════════════════════════════════════════════════════════════════════
// BufWriter<W>
// ═══════════════════════════════════════════════════════════════════════

/// Wraps a [`Write`] with an internal buffer to batch small writes.
pub struct BufWriter<W: Write> {
    inner: W,
    buf: Vec<u8>,
}

impl<W: Write> BufWriter<W> {
    /// Create with default 4 KiB buffer.
    pub fn new(inner: W) -> Self {
        Self::with_capacity(DEFAULT_BUF_SIZE, inner)
    }

    /// Create with custom buffer size.
    pub fn with_capacity(cap: usize, inner: W) -> Self {
        Self {
            inner,
            buf: Vec::with_capacity(cap),
        }
    }

    /// Get a reference to the underlying writer.
    pub fn get_ref(&self) -> &W {
        &self.inner
    }

    /// Get a mutable reference to the underlying writer.
    pub fn get_mut(&mut self) -> &mut W {
        &mut self.inner
    }

    /// Flush the buffer and return the underlying writer.
    pub fn into_inner(mut self) -> error::Result<W> {
        self.flush_buf()?;
        // SAFETY: we flush first, then take inner out before Drop runs.
        let inner = unsafe { core::ptr::read(&self.inner) };
        core::mem::forget(self);
        Ok(inner)
    }

    fn flush_buf(&mut self) -> error::Result<()> {
        let mut written = 0;
        while written < self.buf.len() {
            match self.inner.write(&self.buf[written..]) {
                Ok(0) => return Err(Error::new(ErrorKind::WriteZero)),
                Ok(n) => written += n,
                Err(e) => {
                    // Shift remaining data to front.
                    self.buf.drain(..written);
                    return Err(e);
                }
            }
        }
        self.buf.clear();
        Ok(())
    }
}

impl<W: Write> crate::io::Write for BufWriter<W> {
    fn write(&mut self, buf: &[u8]) -> error::Result<usize> {
        if self.buf.len() + buf.len() > self.buf.capacity() {
            self.flush_buf()?;
        }
        if buf.len() >= self.buf.capacity() {
            // Skip buffer for large writes.
            self.inner.write(buf)
        } else {
            self.buf.extend_from_slice(buf);
            Ok(buf.len())
        }
    }

    fn flush(&mut self) -> error::Result<()> {
        self.flush_buf()?;
        self.inner.flush()
    }
}

impl<W: Write> Drop for BufWriter<W> {
    fn drop(&mut self) {
        let _ = self.flush_buf();
    }
}

// ═══════════════════════════════════════════════════════════════════════
// copy() utility
// ═══════════════════════════════════════════════════════════════════════

/// Copy all bytes from `reader` to `writer`.  Returns total bytes copied.
pub fn copy(reader: &mut dyn Read, writer: &mut dyn Write) -> error::Result<u64> {
    let mut buf = [0u8; 4096];
    let mut total = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            return Ok(total);
        }
        writer.write_all(&buf[..n])?;
        total += n as u64;
    }
}

// ═══════════════════════════════════════════════════════════════════════
// ioctl wrappers
// ═══════════════════════════════════════════════════════════════════════

const IOCTL_FIONREAD: u64 = 0x541B;
const IOCTL_TIOCGWINSZ: u64 = 0x5413;

/// Raw ioctl syscall.
pub fn ioctl(fd: u32, cmd: u64, arg: u64) -> Result<u64, u64> {
    let ret = unsafe { syscall3(SYS_IOCTL, fd as u64, cmd, arg) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

/// Terminal dimensions: (rows, cols, xpixel, ypixel).
///
/// Derived from the framebuffer resolution and 8×16 font size.
/// Falls back to (25, 80, 0, 0) if no framebuffer is registered.
pub fn terminal_size() -> (u16, u16, u16, u16) {
    let mut buf = [0u16; 4];
    let _ = ioctl(0, IOCTL_TIOCGWINSZ, buf.as_mut_ptr() as u64);
    (buf[0], buf[1], buf[2], buf[3])
}

/// Number of bytes available on stdin without blocking.
pub fn stdin_available() -> usize {
    let mut avail = 0u32;
    let _ = ioctl(0, IOCTL_FIONREAD, &mut avail as *mut u32 as u64);
    avail as usize
}
