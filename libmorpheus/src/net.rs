//! Networking — socket API stubs.
//!
//! These syscalls are **reserved** in the ABI (numbers 32-41) but currently
//! return `ENOSYS`.  The kernel's network crate needs restructuring from
//! its current HTTP-download-client design into a proper socket layer
//! before these can be implemented.
//!
//! The ABI numbers are stable — apps can target them today and they will
//! work once the kernel socket layer is built.

use crate::raw::*;

/// Create a socket (AF_INET/AF_INET6, SOCK_STREAM/SOCK_DGRAM).
///
/// **Not yet implemented** — returns `ENOSYS`.
pub fn socket(_domain: u32, _sock_type: u32, _protocol: u32) -> Result<usize, u64> {
    let ret = unsafe {
        syscall3(
            SYS_SOCKET,
            _domain as u64,
            _sock_type as u64,
            _protocol as u64,
        )
    };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}

/// Connect a socket to a remote address.
///
/// **Not yet implemented** — returns `ENOSYS`.
pub fn connect(_fd: usize, _addr_ptr: *const u8, _addr_len: usize) -> Result<(), u64> {
    let ret = unsafe {
        syscall3(
            SYS_CONNECT,
            _fd as u64,
            _addr_ptr as u64,
            _addr_len as u64,
        )
    };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}

/// Send data on a connected socket.
///
/// **Not yet implemented** — returns `ENOSYS`.
pub fn send(_fd: usize, _buf: &[u8]) -> Result<usize, u64> {
    let ret = unsafe { syscall3(SYS_SEND, _fd as u64, _buf.as_ptr() as u64, _buf.len() as u64) };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}

/// Receive data from a connected socket.
///
/// **Not yet implemented** — returns `ENOSYS`.
pub fn recv(_fd: usize, _buf: &mut [u8]) -> Result<usize, u64> {
    let ret = unsafe {
        syscall3(
            SYS_RECV,
            _fd as u64,
            _buf.as_mut_ptr() as u64,
            _buf.len() as u64,
        )
    };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(ret as usize)
    }
}

/// Resolve a hostname to an IPv4 address.
///
/// **Not yet implemented** — returns `ENOSYS`.
pub fn dns_resolve(_hostname: &str, _result_buf: &mut [u8; 4]) -> Result<(), u64> {
    let ret = unsafe {
        syscall3(
            SYS_DNS_RESOLVE,
            _hostname.as_ptr() as u64,
            _hostname.len() as u64,
            _result_buf.as_mut_ptr() as u64,
        )
    };
    if crate::is_error(ret) {
        Err(ret)
    } else {
        Ok(())
    }
}
