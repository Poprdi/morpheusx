//! Canonical syscall error codes — the single source of truth for both the
//! kernel handlers and userland.
//!
//! Encoding is Linux/POSIX-numeric `-errno`: a syscall returns its result in the
//! low range, or `(-(N as i64)) as u64` where `N` is the Linux-numeric value, so
//! errors occupy the very top `[-4095, -1]` window of the u64 range. `is_error`
//! defines the boundary and `errno_value` decodes; both sides of the seam
//! re-export from here so they cannot drift.

/// Encode a Linux-numeric errno `N` as the on-ABI `-errno` return value.
#[inline]
const fn e(n: i64) -> u64 {
    (-n) as u64
}

pub const EPERM: u64 = e(1);
pub const ENOENT: u64 = e(2);
pub const ESRCH: u64 = e(3);
pub const EINTR: u64 = e(4);
pub const EIO: u64 = e(5);
pub const ENXIO: u64 = e(6);
pub const E2BIG: u64 = e(7);
pub const ENOEXEC: u64 = e(8);
pub const EBADF: u64 = e(9);
pub const ECHILD: u64 = e(10);
pub const EAGAIN: u64 = e(11);
/// Alias of `EAGAIN` (same numeric value on Linux).
pub const EWOULDBLOCK: u64 = EAGAIN;
pub const ENOMEM: u64 = e(12);
pub const EACCES: u64 = e(13);
pub const EFAULT: u64 = e(14);
pub const EBUSY: u64 = e(16);
pub const EEXIST: u64 = e(17);
pub const EXDEV: u64 = e(18);
pub const ENODEV: u64 = e(19);
pub const ENOTDIR: u64 = e(20);
pub const EISDIR: u64 = e(21);
pub const EINVAL: u64 = e(22);
pub const ENFILE: u64 = e(23);
pub const EMFILE: u64 = e(24);
pub const ENOTTY: u64 = e(25);
pub const ETXTBSY: u64 = e(26);
pub const EFBIG: u64 = e(27);
pub const ENOSPC: u64 = e(28);
pub const ESPIPE: u64 = e(29);
pub const EROFS: u64 = e(30);
pub const EMLINK: u64 = e(31);
pub const EPIPE: u64 = e(32);
pub const EDOM: u64 = e(33);
pub const ERANGE: u64 = e(34);
pub const EDEADLK: u64 = e(35);
pub const ENAMETOOLONG: u64 = e(36);
pub const ENOLCK: u64 = e(37);
pub const ENOSYS: u64 = e(38);
pub const ENOTEMPTY: u64 = e(39);
pub const ELOOP: u64 = e(40);
pub const ENOMSG: u64 = e(42);
pub const EPROTO: u64 = e(71);
pub const EOVERFLOW: u64 = e(75);
pub const ENOTSOCK: u64 = e(88);
pub const EDESTADDRREQ: u64 = e(89);
pub const EMSGSIZE: u64 = e(90);
pub const EPROTOTYPE: u64 = e(91);
pub const ENOPROTOOPT: u64 = e(92);
pub const EPROTONOSUPPORT: u64 = e(93);
pub const ESOCKTNOSUPPORT: u64 = e(94);
pub const EOPNOTSUPP: u64 = e(95);
/// Alias of `EOPNOTSUPP` (same numeric value on Linux).
pub const ENOTSUP: u64 = EOPNOTSUPP;
pub const EPFNOSUPPORT: u64 = e(96);
pub const EAFNOSUPPORT: u64 = e(97);
pub const EADDRINUSE: u64 = e(98);
pub const EADDRNOTAVAIL: u64 = e(99);
pub const ENETDOWN: u64 = e(100);
pub const ENETUNREACH: u64 = e(101);
pub const ENETRESET: u64 = e(102);
pub const ECONNABORTED: u64 = e(103);
pub const ECONNRESET: u64 = e(104);
pub const ENOBUFS: u64 = e(105);
pub const EISCONN: u64 = e(106);
pub const ENOTCONN: u64 = e(107);
pub const ESHUTDOWN: u64 = e(108);
pub const ETIMEDOUT: u64 = e(110);
pub const ECONNREFUSED: u64 = e(111);
pub const EHOSTDOWN: u64 = e(112);
pub const EHOSTUNREACH: u64 = e(113);
pub const EALREADY: u64 = e(114);
pub const EINPROGRESS: u64 = e(115);
pub const ESTALE: u64 = e(116);
pub const EDQUOT: u64 = e(122);
pub const ECANCELED: u64 = e(125);

/// Linux `MAX_ERRNO`: the error window is `[-4095, -1]`, i.e. the top 4095 u64
/// values. All real returns (pointers/handles/counts/offsets) stay below
/// `USER_ADDR_LIMIT` 0x0000_8000_0000_0000, so they can never alias the window.
pub const MAX_ERRNO: u64 = 4095;

/// True iff `ret` encodes an error: `(ret as i64)` in `[-4095, -1]`, i.e.
/// `ret >= 0xFFFF_FFFF_FFFF_F001`.
#[inline]
pub const fn is_error(ret: u64) -> bool {
    ret >= 0xFFFF_FFFF_FFFF_F001
}

/// Decode the positive Linux-numeric errno from an error return; only meaningful
/// when [`is_error`] holds.
#[inline]
pub const fn errno_value(ret: u64) -> i32 {
    -(ret as i64) as i32
}
