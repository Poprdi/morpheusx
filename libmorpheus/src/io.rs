//! Console I/O: stdin/stdout, `Read`/`Write`/`Seek`/`BufRead`, `BufReader`/`BufWriter`.

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::error::{self, Error, ErrorKind};
use crate::raw::*;

/// Write `s` to fd 1.
pub fn print(s: &str) {
    if s.is_empty() {
        return;
    }
    unsafe {
        syscall3(SYS_WRITE, 1, s.as_ptr() as u64, s.len() as u64);
    }
}

pub fn println(s: &str) {
    let mut w = FdWriter::new(1);
    use core::fmt::Write;
    let _ = w.write_str(s);
    let _ = w.write_str("\n");
}

/// Stack-buffered writer that coalesces a whole formatted message into a single
/// `SYS_WRITE`. `format_args!` calls `write_str` once per literal/argument, so
/// without this a `println!("[{}] {}", a, b)` becomes several syscalls — each
/// its own atomic line on the serial console — and interleaves with other
/// cores. Buffering makes one call = one line. Lines longer than the buffer
/// flush in chunks (still far fewer writes than per fragment). Flushes on drop.
pub(crate) struct FdWriter {
    fd: u32,
    len: usize,
    buf: [u8; 512],
}

impl FdWriter {
    #[inline]
    pub(crate) fn new(fd: u32) -> Self {
        Self {
            fd,
            len: 0,
            buf: [0u8; 512],
        }
    }

    #[inline]
    fn flush(&mut self) {
        if self.len > 0 {
            let _ = write_fd(self.fd, &self.buf[..self.len]);
            self.len = 0;
        }
    }
}

impl core::fmt::Write for FdWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let mut rest = s.as_bytes();
        while !rest.is_empty() {
            if self.len == self.buf.len() {
                self.flush();
            }
            let n = (self.buf.len() - self.len).min(rest.len());
            self.buf[self.len..self.len + n].copy_from_slice(&rest[..n]);
            self.len += n;
            rest = &rest[n..];
        }
        Ok(())
    }
}

impl Drop for FdWriter {
    fn drop(&mut self) {
        self.flush();
    }
}

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

/// Non-blocking drain of the kernel keyboard event ring — raw PS/2 Set 1
/// bytes (break encoded as `|0x80`, `0xE0` prefix as its own byte). Returns
/// the number of bytes read (0 if empty). The compositor reads input through
/// this rather than the stdin byte stream.
pub fn read_keyboard(buf: &mut [u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }
    let ret = unsafe { syscall2(SYS_KEYBOARD_READ, buf.as_mut_ptr() as u64, buf.len() as u64) };
    if crate::is_error(ret) {
        0
    } else {
        ret as usize
    }
}

/// Read a line with local echo; newline consumed and not stored.
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
            },
            0x08 | 0x7F => {
                if pos > 0 {
                    pos -= 1;
                    print("\x08 \x08");
                }
            },
            c => {
                if pos < buf.len() {
                    buf[pos] = c;
                    pos += 1;
                    let s = unsafe { core::str::from_utf8_unchecked(core::slice::from_ref(&c)) };
                    print(s);
                }
            },
        }
    }
}

pub struct Stdout;
pub struct Stderr;
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

#[doc(hidden)]
pub fn _print_fmt(args: core::fmt::Arguments<'_>) {
    // One buffered writer -> one SYS_WRITE for the whole line (see FdWriter).
    let mut w = FdWriter::new(1);
    let _ = core::fmt::Write::write_fmt(&mut w, args);
}

#[doc(hidden)]
pub fn _eprint_fmt(args: core::fmt::Arguments<'_>) {
    let mut w = FdWriter::new(2);
    let _ = core::fmt::Write::write_fmt(&mut w, args);
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::io::_print_fmt(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! println {
    () => { $crate::io::_print_fmt(format_args!("\n")) };
    ($($arg:tt)*) => {
        $crate::io::_print_fmt(format_args!("{}\n", format_args!($($arg)*)))
    };
}

#[macro_export]
macro_rules! eprint {
    ($($arg:tt)*) => {
        $crate::io::_eprint_fmt(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! eprintln {
    () => { $crate::io::_eprint_fmt(format_args!("\n")) };
    ($($arg:tt)*) => {
        $crate::io::_eprint_fmt(format_args!("{}\n", format_args!($($arg)*)))
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekFrom {
    Start(u64),
    Current(i64),
    End(i64),
}

pub trait Read {
    /// Read into `buf`; returns 0 on EOF.
    fn read(&mut self, buf: &mut [u8]) -> error::Result<usize>;

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

    fn read_to_string(&mut self, buf: &mut String) -> error::Result<usize> {
        let mut bytes = Vec::new();
        let n = self.read_to_end(&mut bytes)?;
        match core::str::from_utf8(&bytes) {
            Ok(s) => {
                buf.push_str(s);
                Ok(n)
            },
            Err(_) => Err(Error::new(ErrorKind::InvalidInput)),
        }
    }
}

pub trait Write {
    fn write(&mut self, buf: &[u8]) -> error::Result<usize>;
    fn flush(&mut self) -> error::Result<()>;

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

    fn write_fmt(&mut self, fmt: core::fmt::Arguments<'_>) -> error::Result<()> {
        // Bridge core::fmt::Write → io::Write; capture first error.
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
                    },
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

pub trait Seek {
    fn seek(&mut self, pos: SeekFrom) -> error::Result<u64>;

    fn stream_position(&mut self) -> error::Result<u64> {
        self.seek(SeekFrom::Current(0))
    }
}

pub trait BufRead: Read {
    fn fill_buf(&mut self) -> error::Result<&[u8]>;
    fn consume(&mut self, amt: usize);

    /// Reads through `byte` inclusive (or EOF).
    fn read_until(&mut self, byte: u8, buf: &mut Vec<u8>) -> error::Result<usize> {
        let start = buf.len();
        loop {
            let available = self.fill_buf()?;
            if available.is_empty() {
                return Ok(buf.len() - start);
            }
            let used = match available.iter().position(|&b| b == byte) {
                Some(i) => {
                    buf.extend_from_slice(&available[..=i]);
                    i + 1
                },
                None => {
                    buf.extend_from_slice(available);
                    available.len()
                },
            };
            self.consume(used);
            if buf.last() == Some(&byte) {
                return Ok(buf.len() - start);
            }
        }
    }

    /// Reads through `\n` inclusive.
    fn read_line(&mut self, buf: &mut String) -> error::Result<usize> {
        let mut bytes = Vec::new();
        let n = self.read_until(b'\n', &mut bytes)?;
        match core::str::from_utf8(&bytes) {
            Ok(s) => {
                buf.push_str(s);
                Ok(n)
            },
            Err(_) => Err(Error::new(ErrorKind::InvalidInput)),
        }
    }

    fn lines(self) -> Lines<Self>
    where
        Self: Sized,
    {
        Lines { inner: self }
    }
}

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
                if buf.ends_with('\n') {
                    buf.pop();
                    if buf.ends_with('\r') {
                        buf.pop();
                    }
                }
                Some(Ok(buf))
            },
            Err(e) => Some(Err(e)),
        }
    }
}

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

const DEFAULT_BUF_SIZE: usize = 4096;

pub struct BufReader<R> {
    inner: R,
    buf: Vec<u8>,
    pos: usize,
    filled: usize,
}

impl<R: Read> BufReader<R> {
    pub fn new(inner: R) -> Self {
        Self::with_capacity(DEFAULT_BUF_SIZE, inner)
    }

    pub fn with_capacity(cap: usize, inner: R) -> Self {
        Self {
            inner,
            buf: vec![0u8; cap],
            pos: 0,
            filled: 0,
        }
    }

    pub fn get_ref(&self) -> &R {
        &self.inner
    }

    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<R: Read> Read for BufReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> error::Result<usize> {
        // Bypass internal buffer for large reads when buffer is empty.
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

/// Batches small writes through an internal buffer.
pub struct BufWriter<W: Write> {
    inner: W,
    buf: Vec<u8>,
}

impl<W: Write> BufWriter<W> {
    pub fn new(inner: W) -> Self {
        Self::with_capacity(DEFAULT_BUF_SIZE, inner)
    }

    pub fn with_capacity(cap: usize, inner: W) -> Self {
        Self {
            inner,
            buf: Vec::with_capacity(cap),
        }
    }

    pub fn get_ref(&self) -> &W {
        &self.inner
    }

    pub fn get_mut(&mut self) -> &mut W {
        &mut self.inner
    }

    pub fn into_inner(mut self) -> error::Result<W> {
        self.flush_buf()?;
        // SAFETY: flush succeeded; take inner out before Drop runs.
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
                    self.buf.drain(..written);
                    return Err(e);
                },
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

/// Returns total bytes copied.
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

use morpheus_foundation::flags::{IOCTL_FIONBIO, IOCTL_FIONREAD, IOCTL_TIOCGWINSZ};

pub fn ioctl(fd: u32, cmd: u64, arg: u64) -> Result<u64, u64> {
    let ret = unsafe { syscall3(SYS_IOCTL, fd as u64, cmd, arg) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret)
    }
}

/// `(rows, cols, xpixel, ypixel)` from framebuffer res / 8×16 font.
/// Falls back to `(25, 80, 0, 0)` if no framebuffer is registered.
pub fn terminal_size() -> (u16, u16, u16, u16) {
    let mut buf = [0u16; 4];
    let _ = ioctl(0, IOCTL_TIOCGWINSZ, buf.as_mut_ptr() as u64);
    (buf[0], buf[1], buf[2], buf[3])
}

pub fn stdin_available() -> usize {
    let mut avail = 0u32;
    let _ = ioctl(0, IOCTL_FIONREAD, &mut avail as *mut u32 as u64);
    avail as usize
}

/// Toggle non-blocking stdin (FIONBIO). When enabled, `read(0, ..)` returns
/// EAGAIN instead of blocking on an empty input buffer.
pub fn set_stdin_nonblocking(enable: bool) -> Result<(), u64> {
    let flag: u32 = enable as u32;
    ioctl(0, IOCTL_FIONBIO, &flag as *const u32 as u64).map(|_| ())
}
