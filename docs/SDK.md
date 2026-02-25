# MorpheusX SDK Reference

> **Accuracy guarantee**: every API, type, constant, and syscall in this document was
> verified directly from source code.  If something is not listed here, it is not
> implemented.  If something is listed, it compiles and functions as described.

---

## Table of Contents

1. [Design Philosophy](#1-design-philosophy)
2. [Toolchain & Build](#2-toolchain--build)
3. [Syscall ABI](#3-syscall-abi)
4. [Complete Syscall Reference](#4-complete-syscall-reference)
5. [libmorpheus — Module Reference](#5-libmorpheus--module-reference)
   - [error](#51-error)
   - [io](#52-io)
   - [fs](#53-fs)
   - [net](#54-net)
   - [sync](#55-sync)
   - [time](#56-time)
   - [thread](#57-thread)
   - [task (async)](#58-task-async)
   - [env](#59-env)
   - [process](#510-process)
   - [mem](#511-mem)
   - [hw](#512-hw)
   - [persist](#513-persist)
   - [sys](#514-sys)
   - [raw](#515-raw)
6. [Network Stack Deep-Dive](#6-network-stack-deep-dive)
7. [IPC: Pipes, Poll, Futex](#7-ipc-pipes-poll-futex)
8. [Hardware Driver Development](#8-hardware-driver-development)
9. [Async Programming Model](#9-async-programming-model)
10. [Capability Matrix](#10-capability-matrix)
11. [Example Applications](#11-example-applications)

---

## 1. Design Philosophy

MorpheusX is an exokernel.  The kernel's job is to protect hardware resources, not to
abstract them.  Applications carry their own abstractions.

Key invariants:
- **Ring 3 is real.** User processes run at CPL=3 with page-fault isolation.
- **No hidden control flow.** Every transition is explicit and auditable.
- **`no_std` everywhere.** `libmorpheus` has zero external dependencies.
- **Probe before assume.** The system discovers hardware at boot; it does not guess.
- **Security by elimination.** Less surface area beats more policy.

---

## 2. Toolchain & Build

### Prerequisites

```bash
rustup toolchain install nightly
rustup target add x86_64-unknown-uefi
rustup component add rust-src --toolchain nightly
```

### Building libmorpheus

```bash
cargo +nightly build --release \
  --target x86_64-morpheus.json \
  -Z build-std=core,alloc \
  -Z build-std-features=compiler-builtins-mem \
  -Z json-target-spec \
  -p libmorpheus
```

### Custom target `x86_64-morpheus.json`

| Field | Value |
|---|---|
| arch | x86_64 |
| os | none |
| env | (none) |
| ABI | sysv64 |
| features | +sse,+sse2 |
| relocation-model | pic |
| executables | true |

Kernel target for the bootloader/hwinit is `x86_64-unknown-uefi`.

### Cargo.toml for a userspace app

```toml
[package]
name    = "myapp"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "myapp"

[dependencies]
libmorpheus = { path = "../../libmorpheus" }

[profile.release]
opt-level = "z"
lto       = true
panic     = "abort"
```

### Entry point

```rust
#![no_std]
#![no_main]

extern crate alloc;
use libmorpheus::process;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // your app here
    process::exit(0);
}
```

---

## 3. Syscall ABI

All syscalls use the x86-64 `syscall` instruction with the System V AMD64 ABI register
convention as extended for MorpheusX:

| Register | Role |
|---|---|
| `rax` | syscall number (in), return value (out) |
| `rdi` | argument 1 |
| `rsi` | argument 2 |
| `rdx` | argument 3 |
| `r10` | argument 4 |
| `r8`  | argument 5 (rarely used) |
| `rcx`, `r11` | clobbered by `syscall` |

**Error convention**: return values `>= 0xFFFF_FFFF_FFFF_F000` are error codes (top 4 KB
of the 64-bit address space).  `libmorpheus::is_error(ret)` tests this.

Common error constants re-exported from `libmorpheus::raw`:

| Constant | Meaning |
|---|---|
| `EPERM`   | Operation not permitted |
| `ENOENT`  | No such file or directory |
| `EBADF`   | Bad file descriptor |
| `ENOMEM`  | Out of memory |
| `EACCES`  | Permission denied |
| `EFAULT`  | Bad address |
| `EINVAL`  | Invalid argument |
| `ENOSPC`  | No space left on device |
| `ENOSYS`  | Function not implemented |
| `ENODEV`  | No such device |
| `EIO`     | I/O error |
| `ENOTEMPTY` | Directory not empty |
| `EEXIST`  | File exists |

---

## 4. Complete Syscall Reference

83 syscalls total (0-82).  Arguments shown as `(type name)`.

### Core (0-9)

| # | Name | Signature | Returns |
|---|---|---|---|
| 0 | `SYS_EXIT` | `(u64 code)` | `!` never returns |
| 1 | `SYS_WRITE` | `(u64 fd, u64 buf, u64 len)` | bytes written |
| 2 | `SYS_READ` | `(u64 fd, u64 buf, u64 len)` | bytes read; routes to blocking pipe read for pipe fds |
| 3 | `SYS_YIELD` | `()` | `0` |
| 4 | `SYS_ALLOC` | `(u64 pages)` | physical address |
| 5 | `SYS_FREE` | `(u64 phys, u64 pages)` | `0` |
| 6 | `SYS_GETPID` | `()` | process ID |
| 7 | `SYS_KILL` | `(u64 pid, u64 signal)` | `0` |
| 8 | `SYS_WAIT` | `(u64 pid)` | child exit code |
| 9 | `SYS_SLEEP` | `(u64 ms)` | `0` |

### HelixFS (10-21)

| # | Name | Signature | Returns |
|---|---|---|---|
| 10 | `SYS_OPEN` | `(u64 path, u64 plen, u64 flags)` | fd |
| 11 | `SYS_CLOSE` | `(u64 fd)` | `0` |
| 12 | `SYS_SEEK` | `(u64 fd, u64 offset, u64 whence)` | new offset |
| 13 | `SYS_STAT` | `(u64 path, u64 plen, u64 stat_buf)` | `0` |
| 14 | `SYS_READDIR` | `(u64 path, u64 plen, u64 buf, u64 offset)` | entries written |
| 15 | `SYS_MKDIR` | `(u64 path, u64 plen)` | `0` |
| 16 | `SYS_UNLINK` | `(u64 path, u64 plen)` | `0` |
| 17 | `SYS_RENAME` | `(u64 src, u64 slen, u64 dst, u64 dlen)` | `0` |
| 18 | `SYS_TRUNCATE` | `(u64 fd, u64 new_size)` | `0` |
| 19 | `SYS_SYNC` | `()` | `0` |
| 20 | `SYS_SNAPSHOT` | `(u64 path, u64 plen)` | snapshot LSN |
| 21 | `SYS_VERSIONS` | `(u64 fd, u64 buf, u64 buf_len)` | version count |

**Open flags** (pass in `flags` to `SYS_OPEN`):

| Flag | Value | Meaning |
|---|---|---|
| `O_READ`   | `0x01` | Open for reading |
| `O_WRITE`  | `0x02` | Open for writing |
| `O_CREATE` | `0x04` | Create if absent |
| `O_TRUNC`  | `0x08` | Truncate on open |
| `O_APPEND` | `0x10` | Seek to end before each write |

### System / Process / Memory (22-31)

| # | Name | Signature | Returns |
|---|---|---|---|
| 22 | `SYS_CLOCK` | `(u64 buf)` | nanoseconds since boot |
| 23 | `SYS_SYSINFO` | `(u64 sysinfo_buf)` | `0` |
| 24 | `SYS_GETPPID` | `()` | parent PID |
| 25 | `SYS_SPAWN` | `(u64 path, u64 plen, u64 args, u64 alen)` | child PID |
| 26 | `SYS_MMAP` | `(u64 pages)` | virtual address |
| 27 | `SYS_MUNMAP` | `(u64 vaddr, u64 pages)` | `0` |
| 28 | `SYS_DUP` | `(u64 fd)` | new fd |
| 29 | `SYS_SYSLOG` | `(u64 msg, u64 len)` | `0` |
| 30 | `SYS_GETCWD` | `(u64 buf, u64 len)` | bytes written |
| 31 | `SYS_CHDIR` | `(u64 path, u64 plen)` | `0` |

### Networking (32-41)

#### Raw NIC access (32-37)

These give Ring-3 userspace direct access to NIC ring buffers — the exokernel
model applied to networking.

| # | Name | Signature | Returns |
|---|---|---|---|
| 32 | `SYS_NIC_INFO` | `(u64 buf, u64 len)` | NIC count |
| 33 | `SYS_NIC_TX` | `(u64 nic_id, u64 buf, u64 len)` | bytes sent |
| 34 | `SYS_NIC_RX` | `(u64 nic_id, u64 buf, u64 len)` | bytes received |
| 35 | `SYS_NIC_LINK` | `(u64 nic_id)` | 1=up, 0=down |
| 36 | `SYS_NIC_MAC` | `(u64 nic_id, u64 mac_buf)` | `0` (6-byte MAC at `mac_buf`) |
| 37 | `SYS_NIC_REFILL` | `(u64 nic_id)` | `0` |

#### TCP/UDP socket layer (38 - SYS_NET)

`SYS_NET(subcmd, a2, a3, a4)` is multiplexed.  The network stack must be registered
and ready (DHCP complete or static IP configured) before TCP/UDP subcmds succeed.

| Subcmd | Name | Args | Returns |
|---|---|---|---|
| 0 | `NET_TCP_SOCKET` | `()` | handle |
| 1 | `NET_TCP_CONNECT` | `(handle, ipv4_nbo: u32, port: u16)` | `0` |
| 2 | `NET_TCP_SEND` | `(handle, buf, len)` | bytes_sent |
| 3 | `NET_TCP_RECV` | `(handle, buf, len)` | bytes_received |
| 4 | `NET_TCP_CLOSE` | `(handle)` | `0` |
| 5 | `NET_TCP_STATE` | `(handle)` | state ordinal (0=Closed ... 10=TimeWait) |
| 6 | `NET_TCP_LISTEN` | `(handle, port: u16)` | `0` |
| 7 | `NET_TCP_ACCEPT` | `(listen_handle)` | new handle |
| 8 | `NET_TCP_SHUTDOWN` | `(handle)` | `0` (half-close write side) |
| 9 | `NET_TCP_NODELAY` | `(handle, 1=on/0=off)` | `0` |
| 10 | `NET_TCP_KEEPALIVE` | `(handle, interval_ms)` | `0` (0=disable) |
| 11 | `NET_UDP_SOCKET` | `()` | handle |
| 12 | `NET_UDP_SEND_TO` | `(handle, dest_ip_nbo, dest_port, buf, len)` | bytes_sent |
| 13 | `NET_UDP_RECV_FROM` | `(handle, buf, len, src_out)` | bytes (0=nothing ready); `src_out` is `[u32 ip_nbo, u16 port, u16 pad]` |
| 14 | `NET_UDP_CLOSE` | `(handle)` | `0` |

#### DNS (39 - SYS_DNS)

| Subcmd | Name | Args | Returns |
|---|---|---|---|
| 0 | `DNS_START` | `(name_ptr, name_len)` | query_handle |
| 1 | `DNS_RESULT` | `(query, out_ptr)` | 0=resolved (4-byte IPv4 at `out`), 1=pending, <0=error |
| 2 | `DNS_SET_SERVERS` | `(servers_ptr, count)` | `0` (packed u32 IPv4 NBO array) |

#### Network configuration (40 - SYS_NET_CFG)

| Subcmd | Name | Args | Returns |
|---|---|---|---|
| 0 | `NET_CFG_GET` | `(buf_ptr)` | `0` (fills `NetConfigInfo` at `buf`) |
| 1 | `NET_CFG_DHCP` | `()` | `0` |
| 2 | `NET_CFG_STATIC` | `(ip_nbo: u32, prefix_len: u8, gateway_nbo: u32)` | `0` |
| 3 | `NET_CFG_HOSTNAME` | `(name_ptr, name_len)` | `0` |

`NetConfigInfo` (C repr, passed by pointer):

```rust
#[repr(C)]
pub struct NetConfigInfo {
    pub state:         u32, // 0=unconfigured, 1=dhcp, 2=ready, 3=error
    pub flags:         u32, // bit0=DHCP active, bit1=has gateway, bit2=has DNS
    pub ipv4_addr:     u32, // NBO
    pub prefix_len:    u8,
    pub _pad0:         [u8; 3],
    pub gateway:       u32, // NBO
    pub dns_primary:   u32, // NBO
    pub dns_secondary: u32, // NBO
    pub mac:           [u8; 6],
    pub _pad1:         [u8; 2],
    pub mtu:           u32,
    pub hostname:      [u8; 64], // NUL-terminated
}
```

#### Network poll / stats (41 - SYS_NET_POLL)

| Subcmd | Name | Args | Returns |
|---|---|---|---|
| 0 | `NET_POLL_DRIVE` | `(timestamp_ms: u64)` | 1=activity, 0=quiet |
| 1 | `NET_POLL_STATS` | `(buf_ptr)` | `0` (fills `NetStats`) |

```rust
#[repr(C)]
pub struct NetStats {
    pub tx_packets: u64,
    pub rx_packets: u64,
    pub tx_bytes:   u64,
    pub rx_bytes:   u64,
    pub tx_errors:  u64,
    pub rx_errors:  u64,
    pub tcp_active: u32,
    pub _pad:       u32,
}
```

### Device / Mount (42-45)

| # | Name | Signature | Returns |
|---|---|---|---|
| 42 | `SYS_IOCTL` | `(u64 fd, u64 cmd, u64 arg)` | cmd-dependent |
| 43 | `SYS_MOUNT` | `(u64 src, u64 slen, u64 dst, u64 dlen)` | `0` |
| 44 | `SYS_UMOUNT` | `(u64 path, u64 plen)` | `0` (syncs then returns) |
| 45 | `SYS_POLL` | `(u64 fds_ptr, u64 nfds, u64 timeout_ms)` | ready count |

`SYS_IOCTL` commands:

| Command | Value | Description |
|---|---|---|
| `IOCTL_FIONREAD` | `0x541B` | fd=0 (stdin): bytes available; writes `u32` at `arg` |
| `IOCTL_TIOCGWINSZ` | `0x5413` | fd=0-2: writes `[ws_row: u16, ws_col: u16, xpixel: u16, ypixel: u16]` at `arg`; fixed 80x25 |

`SYS_POLL` uses POSIX `pollfd` layout: `{ fd: i32, events: i16, revents: i16 }`.
Events: `POLLIN=0x0001`, `POLLOUT=0x0004`, `POLLERR=0x0008`.
- fd 0 (stdin): `POLLIN` if keyboard data available
- fd 1/2 (stdout/stderr): always `POLLOUT`
- fd >= 3 (VFS): always `POLLIN | POLLOUT`

### Persistence / Introspection (46-51)

| # | Name | Signature | Returns |
|---|---|---|---|
| 46 | `SYS_PERSIST_PUT` | `(u64 key, u64 klen, u64 val, u64 vlen)` | `0` |
| 47 | `SYS_PERSIST_GET` | `(u64 key, u64 klen, u64 buf, u64 blen)` | bytes read; pass `buf=0, blen=0` to query size |
| 48 | `SYS_PERSIST_DEL` | `(u64 key, u64 klen)` | `0` |
| 49 | `SYS_PERSIST_LIST` | `(u64 buf, u64 blen, u64 offset)` | key count written |
| 50 | `SYS_PERSIST_INFO` | `(u64 persist_info_buf)` | `0` |
| 51 | `SYS_PE_INFO` | `(u64 path, u64 plen, u64 binary_info_buf)` | `0` |

### Hardware Primitives (52-64)

| # | Name | Signature | Returns |
|---|---|---|---|
| 52 | `SYS_PORT_IN` | `(u64 port, u64 width)` | value (width: 1/2/4 bytes) |
| 53 | `SYS_PORT_OUT` | `(u64 port, u64 width, u64 value)` | `0` |
| 54 | `SYS_PCI_CFG_READ` | `(u64 bdf, u64 offset, u64 width)` | value |
| 55 | `SYS_PCI_CFG_WRITE` | `(u64 bdf, u64 offset, u64 width, u64 value)` | `0` |
| 56 | `SYS_DMA_ALLOC` | `(u64 pages)` | physical address (below 4 GB, zeroed, contiguous) |
| 57 | `SYS_DMA_FREE` | `(u64 phys, u64 pages)` | `0` |
| 58 | `SYS_MAP_PHYS` | `(u64 phys, u64 pages, u64 flags)` | virtual address |
| 59 | `SYS_VIRT_TO_PHYS` | `(u64 vaddr)` | physical address |
| 60 | `SYS_IRQ_ATTACH` | `(u64 irq)` | `0` (enables PIC IRQ 0-15) |
| 61 | `SYS_IRQ_ACK` | `(u64 irq)` | `0` (sends EOI) |
| 62 | `SYS_CACHE_FLUSH` | `(u64 addr, u64 len)` | `0` (clflush range) |
| 63 | `SYS_FB_INFO` | `(u64 fb_info_buf)` | `0` |
| 64 | `SYS_FB_MAP` | `()` | virtual address of framebuffer |

`SYS_MAP_PHYS` flags: bit 0 = writable, bit 1 = uncacheable (pass 3 for MMIO).

### Process Management (65-68)

| # | Name | Signature | Returns |
|---|---|---|---|
| 65 | `SYS_PS` | `(u64 buf, u64 len)` | process count |
| 66 | `SYS_SIGACTION` | `(u64 sig, u64 handler_ptr)` | old handler |
| 67 | `SYS_SETPRIORITY` | `(u64 pid, u64 prio)` | `0` |
| 68 | `SYS_GETPRIORITY` | `(u64 pid)` | priority level |

### CPU / Diagnostics (69-72)

| # | Name | Signature | Returns |
|---|---|---|---|
| 69 | `SYS_CPUID` | `(u64 leaf, u64 subleaf, u64 result_buf)` | `0` |
| 70 | `SYS_RDTSC` | `(u64 result_buf)` | TSC value |
| 71 | `SYS_BOOT_LOG` | `(u64 buf, u64 len)` | bytes written (0,0 -> returns total size) |
| 72 | `SYS_MEMMAP` | `(u64 entries_buf, u64 max_count)` | entry count |

### Memory Sharing / Protection (73-74)

| # | Name | Signature | Returns |
|---|---|---|---|
| 73 | `SYS_SHM_GRANT` | `(u64 target_pid, u64 src_vaddr, u64 pages, u64 flags)` | remote virtual address |
| 74 | `SYS_MPROTECT` | `(u64 vaddr, u64 pages, u64 prot)` | `0` |

`MPROTECT` flags: `PROT_READ=0`, `PROT_WRITE=1`, `PROT_EXEC=2`.

### IPC / Shell (75-78)

| # | Name | Signature | Returns |
|---|---|---|---|
| 75 | `SYS_PIPE` | `(u64 result_ptr)` | `0`; writes `[read_fd: u32, write_fd: u32]` at `result_ptr` |
| 76 | `SYS_DUP2` | `(u64 old_fd, u64 new_fd)` | `new_fd` |
| 77 | `SYS_SET_FG` | `(u64 pid)` | `0` |
| 78 | `SYS_GETARGS` | `(u64 buf, u64 len)` | argc; NUL-separated arg blob written at `buf` |

### Synchronization (79)

| # | Name | Signature | Returns |
|---|---|---|---|
| 79 | `SYS_FUTEX` | `(u64 addr, u64 op, u64 val[, u64 timeout_ms])` | `0` |

Ops: `FUTEX_WAIT=0` (sleep while `*addr == val`; optional 4th arg = timeout ms),
`FUTEX_WAKE=1` (wake `val` waiters).

### Threading (80-82)

| # | Name | Signature | Returns |
|---|---|---|---|
| 80 | `SYS_THREAD_CREATE` | `(u64 entry_fn, u64 stack_ptr, u64 stack_size, u64 arg)` | tid |
| 81 | `SYS_THREAD_EXIT` | `(u64 code)` | `!` |
| 82 | `SYS_THREAD_JOIN` | `(u64 tid)` | thread exit code |

---

## 5. libmorpheus — Module Reference

`libmorpheus` is `#![no_std]` + `extern crate alloc`.  Every public item listed
below is verified from source and compiles for `x86_64-morpheus`.

### 5.1 `error`

Structured error type for all SDK operations.

```rust
pub struct Error {
    pub kind: ErrorKind,
    pub raw:  u64,        // raw kernel return value
}

pub enum ErrorKind {
    NotFound,            // ENOENT
    PermissionDenied,    // EACCES / EPERM
    AlreadyExists,       // EEXIST
    InvalidInput,        // EINVAL
    Interrupted,         // EINTR
    WouldBlock,          // EAGAIN
    NotConnected,        // ENOTCONN
    ConnectionRefused,   // ECONNREFUSED
    ConnectionReset,     // ECONNRESET
    TimedOut,            // ETIMEDOUT
    BrokenPipe,          // EPIPE
    OutOfMemory,         // ENOMEM
    NoSpace,             // ENOSPC
    BadFileDescriptor,   // EBADF
    Io,                  // EIO / generic
    NotImplemented,      // ENOSYS
    Other(u64),          // catch-all
}

pub type Result<T> = core::result::Result<T, Error>;

// Convert a raw kernel u64 return into Result<u64>.
pub fn check(raw: u64) -> Result<u64>;

impl From<u64>              for Error { ... }
impl core::fmt::Display     for Error { ... }
```

### 5.2 `io`

I/O traits, console handles, buffered wrappers, and format macros.

#### Traits

```rust
pub trait Read {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()>;
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> Result<usize>;
}

pub trait Write {
    fn write(&mut self, buf: &[u8]) -> Result<usize>;
    fn write_all(&mut self, buf: &[u8]) -> Result<()>;
    fn flush(&mut self) -> Result<()>;
}

pub trait Seek {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64>;
    fn stream_position(&mut self) -> Result<u64>;
    fn stream_len(&mut self) -> Result<u64>;
}

pub trait BufRead: Read {
    fn fill_buf(&mut self) -> Result<&[u8]>;
    fn consume(&mut self, amt: usize);
    fn read_line(&mut self, buf: &mut String) -> Result<usize>;
    fn lines(self) -> Lines<Self> where Self: Sized;
}

pub enum SeekFrom {
    Start(u64),
    End(i64),
    Current(i64),
}
```

#### Buffered wrappers

```rust
pub struct BufReader<R: Read> { ... }
impl<R: Read> BufReader<R> {
    pub fn new(inner: R) -> Self;
    pub fn with_capacity(cap: usize, inner: R) -> Self;
    pub fn get_ref(&self) -> &R;
    pub fn get_mut(&mut self) -> &mut R;
    pub fn into_inner(self) -> R;
    pub fn buffer(&self) -> &[u8];
}
impl<R: Read> Read    for BufReader<R> { ... }
impl<R: Read> BufRead for BufReader<R> { ... }

pub struct BufWriter<W: Write> { ... }
impl<W: Write> BufWriter<W> {
    pub fn new(inner: W) -> Self;
    pub fn with_capacity(cap: usize, inner: W) -> Self;
    pub fn into_inner(self) -> W;    // flushes then moves inner out
    pub fn get_ref(&self) -> &W;
    pub fn get_mut(&mut self) -> &mut W;
    pub fn buffer(&self) -> &[u8];
}
impl<W: Write> Write for BufWriter<W> { ... }
```

#### Console handles

```rust
pub struct Stdin;
pub struct Stdout;
pub struct Stderr;

pub fn stdin()  -> Stdin;
pub fn stdout() -> Stdout;
pub fn stderr() -> Stderr;

impl Read             for Stdin  { ... }  // SYS_READ fd=0
impl Write            for Stdout { ... }  // SYS_WRITE fd=1
impl Write            for Stderr { ... }  // SYS_WRITE fd=2
impl core::fmt::Write for Stdout { ... }
impl core::fmt::Write for Stderr { ... }
```

#### Format macros

```rust
print!(...)     // Stdout via format_args!
println!(...)   // Stdout + newline
eprint!(...)    // Stderr via format_args!
eprintln!(...)  // Stderr + newline
```

#### Utility

```rust
pub fn copy<R: Read, W: Write>(reader: &mut R, writer: &mut W) -> Result<u64>;

pub struct Lines<B> { ... }
impl<B: BufRead> Iterator for Lines<B> {
    type Item = Result<String>;
}
```

### 5.3 `fs`

Filesystem operations backed by HelixFS.

#### `File`

```rust
pub struct File { fd: usize }

impl File {
    pub fn open(path: &str)                          -> crate::error::Result<Self>;
    pub fn create(path: &str)                        -> crate::error::Result<Self>;
    pub fn from_raw_fd(fd: usize)                    -> Self;
    pub fn into_raw_fd(self)                         -> usize;
    pub fn metadata(&self)                           -> crate::error::Result<Metadata>;
    pub fn set_len(&self, size: u64)                 -> crate::error::Result<()>;
    pub fn sync_all(&self)                           -> crate::error::Result<()>;
    pub fn try_clone(&self)                          -> crate::error::Result<Self>;
}

impl Read  for File { ... }
impl Write for File { ... }
impl Seek  for File { ... }
impl Drop  for File { ... }  // calls SYS_CLOSE
```

#### `OpenOptions`

```rust
pub struct OpenOptions { ... }

impl OpenOptions {
    pub fn new()                         -> Self;
    pub fn read(self, yes: bool)         -> Self;
    pub fn write(self, yes: bool)        -> Self;
    pub fn create(self, yes: bool)       -> Self;
    pub fn truncate(self, yes: bool)     -> Self;
    pub fn append(self, yes: bool)       -> Self;
    pub fn open(self, path: &str)        -> crate::error::Result<File>;
}
```

#### `Metadata`

```rust
pub struct Metadata {
    pub size:          u64,
    pub is_dir:        bool,
    pub is_file:       bool,
    pub readonly:      bool,
    // Raw HelixFS FileStat fields
    pub file_size:     u64,
    pub block_count:   u64,
    pub inode:         u64,
    pub kind:          u32,
    pub flags:         u32,
    pub created_lsn:   u64,
    pub modified_lsn:  u64,
}

impl Metadata {
    pub fn len(&self)     -> u64;
    pub fn is_dir(&self)  -> bool;
    pub fn is_file(&self) -> bool;
}
```

#### Directory iteration

```rust
pub struct DirEntry {
    pub name:     [u8; 256],
    pub name_len: u32,
    pub kind:     u32,    // 0=file, 1=dir
    pub size:     u64,
}

impl DirEntry {
    pub fn name(&self)      -> &str;
    pub fn is_dir(&self)    -> bool;
    pub fn is_file(&self)   -> bool;
    pub fn file_size(&self) -> u64;
}

pub struct ReadDir { ... }
impl Iterator for ReadDir {
    type Item = crate::error::Result<DirEntry>;
}

pub fn read_dir(path: &str) -> crate::error::Result<ReadDir>;
```

#### Convenience functions

```rust
pub fn read_to_vec(path: &str)             -> crate::error::Result<Vec<u8>>;
pub fn read_to_string(path: &str)          -> crate::error::Result<String>;
pub fn write_bytes(path: &str, data: &[u8]) -> crate::error::Result<()>;
pub fn create_dir(path: &str)              -> crate::error::Result<()>;
pub fn remove_file(path: &str)             -> crate::error::Result<()>;
pub fn rename_path(from: &str, to: &str)   -> crate::error::Result<()>;
pub fn copy(from: &str, to: &str)          -> crate::error::Result<u64>; // bytes copied
```

#### HelixFS extensions

```rust
pub fn sync()               -> crate::error::Result<()>;
pub fn snapshot(path: &str) -> crate::error::Result<u64>; // returns LSN
```

### 5.4 `net`

TCP/UDP sockets, DNS, and network configuration.

> `TcpStream::connect_host` performs a synchronous DNS lookup followed by connect.
> All blocking calls yield via `SYS_YIELD` internally — no busy-spinning.

#### Address types

```rust
pub struct Ipv4Addr(pub u32);  // stored in network byte order

impl Ipv4Addr {
    pub const fn new(a: u8, b: u8, c: u8, d: u8) -> Self;
    pub const LOCALHOST:     Self;
    pub const UNSPECIFIED:   Self;
    pub fn from_nbo(nbo: u32) -> Self;
    pub fn to_nbo(self)        -> u32;
    pub fn octets(self)        -> [u8; 4];
}

pub struct SocketAddr {
    pub ip:   Ipv4Addr,
    pub port: u16,
}
```

#### `TcpStream`

```rust
pub struct TcpStream { handle: i64 }

impl TcpStream {
    pub fn connect(ip_nbo: u32, port: u16)             -> crate::error::Result<Self>;
    pub fn connect_host(host: &str, port: u16)         -> crate::error::Result<Self>;
    pub fn state(&self)                                -> i64;   // 0=Closed...10=TimeWait
    pub fn wait_connected(&self)                       -> crate::error::Result<()>;
    pub fn send_all(&mut self, data: &[u8])            -> crate::error::Result<()>;
    pub fn recv_blocking(&mut self, buf: &mut [u8])    -> crate::error::Result<usize>;
    pub fn set_nodelay(&self, on: bool)                -> crate::error::Result<()>;
    pub fn set_keepalive(&self, ms: u64)               -> crate::error::Result<()>;
    pub fn shutdown(&self)                             -> crate::error::Result<()>;
}

impl Read  for TcpStream { ... }
impl Write for TcpStream { ... }
impl Drop  for TcpStream { ... }  // NET_TCP_CLOSE
```

#### `TcpListener`

```rust
pub struct TcpListener { handle: i64 }

impl TcpListener {
    pub fn bind(port: u16)           -> crate::error::Result<Self>;
    pub fn accept(&self)             -> crate::error::Result<Option<TcpStream>>;  // non-blocking
    pub fn accept_blocking(&self)    -> crate::error::Result<TcpStream>;
}

impl Drop for TcpListener { ... }  // NET_TCP_CLOSE
```

#### `UdpSocket`

```rust
pub struct UdpSocket { handle: i64 }

impl UdpSocket {
    pub fn new()                                                       -> crate::error::Result<Self>;
    pub fn send_to(&self, buf: &[u8], ip_nbo: u32, port: u16)        -> crate::error::Result<usize>;
    pub fn recv_from(&self, buf: &mut [u8])                           -> crate::error::Result<Option<(usize, Ipv4Addr, u16)>>; // non-blocking
    pub fn recv_from_blocking(&self, buf: &mut [u8])                  -> crate::error::Result<(usize, Ipv4Addr, u16)>;
}

impl Drop for UdpSocket { ... }  // NET_UDP_CLOSE
```

#### DNS

```rust
pub fn dns_lookup_start(name: &str)     -> crate::error::Result<i64>;
pub fn dns_lookup_poll(handle: i64)     -> crate::error::Result<Option<Ipv4Addr>>;
pub fn dns_resolve(host: &str)          -> crate::error::Result<Ipv4Addr>;      // blocking
pub fn dns_set_servers(servers: &[u32]) -> crate::error::Result<()>;            // packed u32 NBO
```

#### Network configuration

```rust
pub fn net_config()                                            -> crate::error::Result<NetConfigInfo>;
pub fn net_dhcp()                                              -> crate::error::Result<()>;
pub fn net_static(ip: Ipv4Addr, prefix_len: u8, gw: Ipv4Addr) -> crate::error::Result<()>;
pub fn net_set_hostname(name: &str)                            -> crate::error::Result<()>;
pub fn net_stats()                                             -> crate::error::Result<NetStats>;
pub fn net_drive(timestamp_ms: u64)                            -> crate::error::Result<bool>; // true=activity
```

### 5.5 `sync`

Synchronization primitives, all backed by `SYS_FUTEX`.

#### `Mutex<T>`

```rust
pub struct Mutex<T> { ... }
impl<T> Mutex<T> {
    pub const fn new(val: T) -> Self;
    pub fn lock(&self)        -> MutexGuard<'_, T>;
    pub fn try_lock(&self)    -> Option<MutexGuard<'_, T>>;
    pub fn into_inner(self)   -> T;
}
```

#### `RwLock<T>`

```rust
pub struct RwLock<T> { ... }
impl<T> RwLock<T> {
    pub const fn new(val: T)   -> Self;
    pub fn read(&self)          -> RwLockReadGuard<'_, T>;
    pub fn write(&self)         -> RwLockWriteGuard<'_, T>;
    pub fn try_read(&self)      -> Option<RwLockReadGuard<'_, T>>;
    pub fn try_write(&self)     -> Option<RwLockWriteGuard<'_, T>>;
}
```

State encoding: readers = `1..0x7FFF_FFFE` (concurrent); writer = `0xFFFF_FFFF` (exclusive).

#### `mpsc` channel

```rust
pub mod mpsc {
    pub fn channel<T>() -> (Sender<T>, Receiver<T>);

    pub struct Sender<T> { ... }
    impl<T> Sender<T> {
        pub fn send(&self, val: T) -> Result<(), SendError<T>>;
    }
    impl<T> Clone for Sender<T> { ... }  // multiple producers supported

    pub struct Receiver<T> { ... }
    impl<T> Receiver<T> {
        pub fn recv(&self)      -> Result<T, RecvError>;
        pub fn try_recv(&self)  -> Result<T, TryRecvError>;
        pub fn iter(&self)      -> impl Iterator<Item = T> + '_;
    }
}
```

Backed by `Mutex<VecDeque<T>>` + futex for blocking `recv`.

### 5.6 `time`

Time primitives.

```rust
pub fn clock_gettime() -> u64;  // nanoseconds since boot (SYS_CLOCK)

pub struct Duration { nanos: u128 }
impl Duration {
    pub const ZERO:          Self;
    pub fn from_nanos(n: u64)  -> Self;
    pub fn from_micros(n: u64) -> Self;
    pub fn from_millis(n: u64) -> Self;
    pub fn from_secs(n: u64)   -> Self;
    pub fn as_nanos(&self)  -> u128;
    pub fn as_micros(&self) -> u128;
    pub fn as_millis(&self) -> u128;
    pub fn as_secs(&self)   -> u64;
    pub fn as_secs_f64(&self) -> f64;
    pub fn subsec_nanos(&self) -> u32;
    pub fn checked_add(self, rhs: Self) -> Option<Self>;
    pub fn checked_sub(self, rhs: Self) -> Option<Self>;
    pub fn saturating_add(self, rhs: Self) -> Self;
    pub fn saturating_sub(self, rhs: Self) -> Self;
}
impl Add<Duration> for Duration { ... }
impl Sub<Duration> for Duration { ... }
impl core::fmt::Display for Duration { ... }  // "1.234s" / "500ms" / "20µs"

pub struct Instant { nanos: u64 }
impl Instant {
    pub fn now()                              -> Self;
    pub fn elapsed(&self)                     -> Duration;
    pub fn duration_since(&self, earlier: Self) -> Duration;
}
impl Add<Duration> for Instant { ... }
impl Sub<Duration> for Instant { ... }
impl Sub<Instant>  for Instant { ... }

pub fn sleep(d: Duration);  // blocks current thread via SYS_SLEEP
```

### 5.7 `thread`

OS threads with preemptive scheduling (~100 Hz).

```rust
pub struct JoinHandle<T> { ... }
impl<T> JoinHandle<T> {
    pub fn join(self) -> crate::error::Result<T>;
    pub fn tid(&self) -> u64;
}

pub fn spawn<F, T>(f: F) -> crate::error::Result<JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static;

pub struct Builder { ... }
impl Builder {
    pub fn new()                           -> Self;
    pub fn stack_size(self, size: usize)   -> Self;
    pub fn spawn<F, T>(self, f: F)         -> crate::error::Result<JoinHandle<T>>
    where F: FnOnce() -> T + Send + 'static, T: Send + 'static;
}

pub fn current_tid() -> u64;   // SYS_GETPID of the thread slot
pub fn yield_now();             // SYS_YIELD
pub fn sleep_ms(ms: u64);      // SYS_SLEEP
```

Default stack size: 1 MiB.

### 5.8 `task` (async)

Cooperative async executor.  Any future that implements `core::future::Future`
works here without modification.

```rust
// Block the calling thread on a single future.
pub fn block_on<F: Future>(future: F) -> F::Output;

// Yield the current task once, then resume.
pub fn yield_now() -> YieldFuture;

// Async sleep (milliseconds); relies on FUTEX_WAIT timeout.
pub fn sleep(millis: u64) -> SleepFuture;

pub struct Runtime { ... }
impl Runtime {
    pub fn new() -> Self;

    // Fire-and-forget spawn.
    pub fn spawn<F>(&self, future: F)
    where F: Future<Output = ()> + Send + 'static;

    // Spawn with result.
    pub fn spawn_with_handle<F>(&self, future: F) -> JoinHandle<F::Output>
    where F: Future + Send + 'static, F::Output: Send + 'static;

    // Run event loop until all tasks complete (single-threaded).
    pub fn run(&self);

    // Distribute work across N OS threads.
    pub fn run_threaded(&self, workers: usize);
}

pub struct JoinHandle<T> { ... }
impl<T> Future for JoinHandle<T> { type Output = T; }
```

**Waker mechanism**: each task has an embedded `AtomicU32` futex word.
`Waker::wake()` sets it and calls `FUTEX_WAKE`; the worker re-polls the task.
No busy-waiting.

### 5.9 `env`

Command-line arguments and working directory.

```rust
pub struct Args { ... }
impl Iterator          for Args { type Item = &'static str; }
impl ExactSizeIterator for Args { fn len(&self) -> usize; }

pub fn args()          -> Args;              // lazy iterator
pub fn args_vec()      -> Vec<&'static str>; // collected

pub fn current_dir()          -> crate::error::Result<String>;
pub fn set_current_dir(path: &str) -> crate::error::Result<()>;
```

### 5.10 `process`

Process management and control.

```rust
pub fn exit(code: i32)  -> !;     // SYS_EXIT
pub fn abort()          -> !;     // SYS_EXIT(1) without cleanup
pub fn self_pid()       -> u32;   // SYS_GETPID
pub fn getppid()        -> u32;   // SYS_GETPPID
pub fn exec(path: &str, args: &[&str]) -> crate::error::Result<()>;
pub fn wait()           -> crate::error::Result<(u32, u32)>; // (pid, exit_code)
pub fn wait_pid(pid: u32) -> crate::error::Result<u32>;
pub fn sched_yield();
pub fn getenv(key: &str)             -> Option<&'static str>;
pub fn setenv(key: &str, val: &str)  -> crate::error::Result<()>;
pub fn getcwd()                      -> crate::error::Result<String>;
pub fn chdir(path: &str)             -> crate::error::Result<()>;
pub fn sbrk(increment: i64)          -> crate::error::Result<u64>;

pub struct Command { ... }
impl Command {
    pub fn new(prog: &str)             -> Self;
    pub fn arg(self, a: &str)          -> Self;
    pub fn args(self, args: &[&str])   -> Self;
    pub fn spawn_pid(self)             -> crate::error::Result<u32>;
    pub fn status(self)                -> crate::error::Result<ExitStatus>;
}

pub struct ExitStatus { code: u32 }
impl ExitStatus {
    pub fn code(&self)    -> u32;
    pub fn success(&self) -> bool;
}
```

### 5.11 `mem`

Virtual memory management.

```rust
pub fn alloc_pages(pages: u64)             -> Result<u64, u64>;  // phys addr
pub fn free_pages(phys_base: u64, pages: u64) -> Result<(), u64>;
pub fn mmap(pages: u64)                    -> Result<u64, u64>;  // vaddr
pub fn munmap(vaddr: u64, pages: u64)      -> Result<(), u64>;

pub const PROT_READ:  u64 = 0;
pub const PROT_WRITE: u64 = 1;
pub const PROT_EXEC:  u64 = 2;  // clears NX bit

pub fn shm_grant(target_pid: u32, src_vaddr: u64, pages: u64, flags: u64)
    -> Result<u64, u64>;  // returns remote vaddr
pub fn mprotect(vaddr: u64, pages: u64, prot: u64) -> Result<(), u64>;
```

### 5.12 `hw`

Hardware primitives — the exokernel escape hatch.

#### Port I/O

```rust
pub fn port_inb(port: u16) -> u8;
pub fn port_inw(port: u16) -> u16;
pub fn port_inl(port: u16) -> u32;
pub fn port_outb(port: u16, value: u8);
pub fn port_outw(port: u16, value: u16);
pub fn port_outl(port: u16, value: u32);
```

#### PCI configuration space

```rust
// Encode bus/device/function as a BDF word.
pub fn pci_bdf(bus: u8, device: u8, function: u8) -> u64;

pub fn pci_cfg_read8 (bus: u8, dev: u8, fn_: u8, offset: u8) -> u8;
pub fn pci_cfg_read16(bus: u8, dev: u8, fn_: u8, offset: u8) -> u16;
pub fn pci_cfg_read32(bus: u8, dev: u8, fn_: u8, offset: u8) -> u32;
pub fn pci_cfg_write8 (bus: u8, dev: u8, fn_: u8, offset: u8, value: u8);
pub fn pci_cfg_write16(bus: u8, dev: u8, fn_: u8, offset: u8, value: u16);
pub fn pci_cfg_write32(bus: u8, dev: u8, fn_: u8, offset: u8, value: u32);
```

#### DMA and physical memory

```rust
// Allocate physically contiguous memory below 4 GB, zeroed on alloc.
pub fn dma_alloc(pages: u64)           -> Result<u64, u64>;  // phys addr
pub fn dma_free(phys: u64, pages: u64) -> Result<(), u64>;

// Map a physical range into the calling process's VA space.
// flags: bit0=writable, bit1=uncacheable.  Pass 3 for MMIO.
pub fn map_phys(phys: u64, pages: u64, flags: u64) -> Result<u64, u64>;
pub fn map_phys_rw(phys: u64, pages: u64)          -> Result<u64, u64>;
pub fn map_mmio(phys: u64, pages: u64)              -> Result<u64, u64>;

pub fn virt_to_phys(virt: u64) -> Result<u64, u64>;
```

#### IRQ

```rust
// Enable an 8259A PIC IRQ line (0-15).
pub fn irq_attach(irq: u8) -> Result<(), u64>;
// Send End-Of-Interrupt to PIC.
pub fn irq_ack(irq: u8)    -> Result<(), u64>;
```

Call `irq_attach` before any operation that generates that interrupt.
After handling the interrupt, call `irq_ack` or the PIC stops delivering it.

#### CPU cache

```rust
// clflush the address range.  Do this before triggering a DMA read.
pub fn cache_flush(addr: u64, len: u64) -> Result<(), u64>;
```

#### CPUID / RDTSC

```rust
#[repr(C)]
pub struct CpuidResult { pub eax: u32, pub ebx: u32, pub ecx: u32, pub edx: u32 }

pub fn cpuid(leaf: u32, subleaf: u32) -> CpuidResult;

#[repr(C)]
pub struct TscResult { pub tsc: u64, pub frequency: u64 }

pub fn rdtsc()     -> TscResult;  // TSC + calibrated frequency in Hz
pub fn rdtsc_raw() -> u64;        // raw TSC only (faster, no frequency)
```

#### Framebuffer

```rust
#[repr(C)]
pub struct FbInfo {
    pub base:   u64,
    pub size:   u64,
    pub width:  u32,
    pub height: u32,
    pub stride: u32,  // bytes per row
    pub format: u32,  // 0=RGBX, 1=BGRX
}

pub fn fb_info() -> Result<FbInfo, u64>;
pub fn fb_map()  -> Result<u64, u64>;  // VA of framebuffer (writable, uncached)
```

#### Boot log and memory map

```rust
pub fn boot_log_size()              -> u64;
pub fn boot_log(buf: &mut [u8])     -> Result<usize, u64>;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MemmapEntry {
    pub phys_start: u64,
    pub num_pages:  u64,
    pub mem_type:   u32,  // UEFI EFI_MEMORY_TYPE ordinal
    pub _pad:       u32,
}

pub fn memmap_count()                        -> u64;
pub fn memmap(entries: &mut [MemmapEntry])   -> Result<usize, u64>;
```

### 5.13 `persist`

Persistent key-value store backed by HelixFS, and binary introspection.

#### KV store

```rust
// Store.  Key: 1-255 bytes, no '/' or '\0'.  Value: at most 4 MiB.
pub fn put(key: &str, data: &[u8])          -> Result<(), u64>;
// Load into buf; returns bytes written.  Returns Err(ENOENT) if missing.
pub fn get(key: &str, buf: &mut [u8])       -> Result<usize, u64>;
// Size query without reading (pass empty buf + len=0 internally).
pub fn get_size(key: &str)                  -> Result<u64, u64>;
// Delete.
pub fn del(key: &str)                       -> Result<(), u64>;
// List keys (NUL-separated); offset = skip N entries for pagination.
pub fn list(buf: &mut [u8], offset: u64)    -> Result<u64, u64>;
// Stats.
pub fn info()                               -> Result<PersistInfo, u64>;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PersistInfo {
    pub backend_flags: u32,  // bit0 = HelixFS active
    pub _pad0:         u32,
    pub num_keys:      u64,
    pub used_bytes:    u64,
}
```

#### Binary introspection

```rust
pub fn pe_info(path: &str) -> Result<BinaryInfo, u64>;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct BinaryInfo {
    pub format:       u32,  // 0=unknown, 1=ELF64, 2=PE32+
    pub arch:         u32,  // 0=unknown, 1=x86_64, 2=aarch64, 3=arm
    pub entry_point:  u64,
    pub image_base:   u64,
    pub image_size:   u64,
    pub num_sections: u32,
    pub _pad0:        u32,
}
```

### 5.14 `sys`

System information.

```rust
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SysInfo {
    pub total_mem:    u64,
    pub free_mem:     u64,
    pub num_procs:    u32,
    pub _pad0:        u32,
    pub uptime_ticks: u64,
    pub tsc_freq:     u64,  // divide uptime_ticks by this -> seconds
    pub heap_total:   u64,
    pub heap_used:    u64,
    pub heap_free:    u64,
}

impl SysInfo {
    pub const fn zeroed() -> Self;
    pub fn uptime_ms(&self) -> u64;
}

pub fn sysinfo(info: &mut SysInfo) -> Result<(), u64>;
// Write a debug message directly to the kernel serial port.
pub fn syslog(msg: &str);
```

### 5.15 `raw`

Direct syscall invocation and all syscall number constants.

```rust
// All 83 SYS_* constants (SYS_EXIT=0 through SYS_THREAD_JOIN=82).
pub const SYS_EXIT:         u64 = 0;
pub const SYS_WRITE:        u64 = 1;
// ... all others as listed in §4 ...
pub const SYS_THREAD_JOIN:  u64 = 82;

// Futex op constants.
pub const FUTEX_WAIT: u64 = 0;
pub const FUTEX_WAKE: u64 = 1;

// Inline-asm wrappers.
pub unsafe fn syscall0(nr: u64)                                        -> u64;
pub unsafe fn syscall1(nr: u64, a1: u64)                               -> u64;
pub unsafe fn syscall2(nr: u64, a1: u64, a2: u64)                      -> u64;
pub unsafe fn syscall3(nr: u64, a1: u64, a2: u64, a3: u64)             -> u64;
pub unsafe fn syscall4(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64)   -> u64;
pub unsafe fn syscall5(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> u64;
```

`libmorpheus::is_error(ret: u64) -> bool` is in the crate root.

---

## 6. Network Stack Deep-Dive

### Architecture

```
Ring 3 userspace
  libmorpheus::net  (TcpStream / TcpListener / UdpSocket / dns_resolve / net_config)
         |
         |  SYS_NET (38) / SYS_DNS (39) / SYS_NET_CFG (40) / SYS_NET_POLL (41)
         v
Ring 0 kernel (hwinit)
  NetStackOps fn-ptr table   <- registered by bootloader at startup
         |
         v
  morpheus_network crate
    NativeHttpClient<D>   (kernel-internal; used by auto-updater, not Ring 3)
    NetInterface<D>       (smoltcp wrapper)
    URL parser            (url/parser.rs)
    HTTP/1.1 request/response/headers
         |
         v
  smoltcp  (bare-metal TCP/IP)
         |
         v
  NIC driver (virtio-net / e1000 / raw ring buffers via SYS_NIC_TX/RX)
```

### What is and is not accessible from Ring 3

`NativeHttpClient` is **kernel-internal** — it is used by the MorpheusX update
subsystem, not exposed as a userspace API.  Ring-3 apps do plain HTTP manually
over `TcpStream`.  This is straightforward because HTTP/1.1 is text-based.

### Exokernel note on TLS, crypto, and custom NICs

TLS is not a kernel responsibility in MorpheusX — this is intentional.
A userspace app that needs HTTPS ports a `no_std` TLS library (e.g. `embedded-tls`
or a custom state machine) and layers it over `TcpStream`.  No kernel changes
needed.  The same logic applies to any other protocol or cryptographic primitive.

Similarly, a custom NIC driver is just a userspace app using `hw::map_mmio`,
`hw::dma_alloc`, `hw::irq_attach`, and `SYS_NIC_TX`/`SYS_NIC_RX` to drive
the hardware directly from Ring 3.  The kernel exposes the resource; the driver
owns the abstraction.

This is the exokernel model: the kernel surface area is fixed and minimal.
Every abstraction above it lives in userspace and is owned by the application.

### What the kernel provides

| Feature | Status |
|---|---|
| TCP sockets (connect / listen / accept / send / recv) | Yes |
| UDP sockets | Yes |
| DNS (async + blocking + server override) | Yes |
| DHCP | Yes (kernel-side, automatic) |
| Static IP configuration | Yes |
| Raw NIC ring access (SYS_NIC_TX/RX) | Yes |
| Network statistics | Yes |
| Plain HTTP/1.1 via TcpStream | Yes |
| TLS / HTTPS | Userland — port any `no_std` TLS crate over `TcpStream` |
| Custom NIC driver | Userland — `hw::map_mmio` + DMA + IRQ + raw ring access |
| WebSockets | Userland — implement handshake/framing over `TcpStream` |
| IPv6 | Not wired up (smoltcp has support) |
| ICMP from Ring 3 | Not exposed directly; achievable via raw `SYS_NIC_TX/RX` |

### Plain HTTP/1.1 example

```rust
use libmorpheus::net::TcpStream;
use libmorpheus::io::Write;

let mut s = TcpStream::connect_host("httpbin.org", 80)?;
write!(s, "GET /ip HTTP/1.0\r\nHost: httpbin.org\r\n\r\n").ok();
s.flush()?;

let mut buf = [0u8; 4096];
let n = s.recv_blocking(&mut buf)?;
let body = core::str::from_utf8(&buf[..n]).unwrap_or("");
println!("{body}");
```

### TCP server

```rust
use libmorpheus::net::TcpListener;
use libmorpheus::io::{Read, Write};
use libmorpheus::thread;

let listener = TcpListener::bind(8080)?;
loop {
    let mut client = listener.accept_blocking()?;
    thread::spawn(move || {
        let mut buf = [0u8; 2048];
        loop {
            let n = match client.recv_blocking(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let _ = client.write_all(&buf[..n]);
        }
    }).ok();
}
```

---

## 7. IPC: Pipes, Poll, Futex

### Pipes

`SYS_PIPE` creates a kernel-buffered unidirectional pipe and returns two file
descriptors: a read end and a write end.  Both ends behave as normal fds:
`SYS_WRITE` on the write end, `SYS_READ` on the read end.  `SYS_READ` on a
pipe fd is routed to `sys_pipe_read_blocking` which parks the process under
`BlockReason::PipeRead` until data arrives or all writers close (EOF).

```rust
use libmorpheus::raw::{syscall1, SYS_PIPE};
use libmorpheus::fs::File;
use libmorpheus::io::{Read, Write};

let mut fds = [0u32; 2];
unsafe { syscall1(SYS_PIPE, fds.as_mut_ptr() as u64); }
let mut reader = File::from_raw_fd(fds[0] as usize);
let mut writer = File::from_raw_fd(fds[1] as usize);

writer.write_all(b"hello pipe\n").unwrap();
drop(writer);  // close write end -> causes EOF on reader

let mut buf = [0u8; 64];
let n = reader.read(&mut buf).unwrap();
// &buf[..n] == b"hello pipe\n"
```

### Redirecting stdin/stdout (shell pipeline)

```rust
use libmorpheus::raw::{syscall1, syscall2, SYS_PIPE, SYS_DUP2};

let mut fds = [0u32; 2];
unsafe { syscall1(SYS_PIPE, fds.as_mut_ptr() as u64); }
let (read_fd, write_fd) = (fds[0] as u64, fds[1] as u64);

// In child: redirect stdout -> pipe write end, then exec
// unsafe { syscall2(SYS_DUP2, write_fd, 1); }
// process::exec("/bin/ls", &["/data"]).unwrap();

// In parent: read output from pipe
let mut buf = [0u8; 4096];
let n = unsafe {
    libmorpheus::raw::syscall3(
        libmorpheus::raw::SYS_READ,
        read_fd,
        buf.as_mut_ptr() as u64,
        buf.len() as u64,
    )
};
```

`SYS_DUP2(old_fd, new_fd)` closes `new_fd` if open, then installs `old_fd`
as `new_fd`.  It correctly updates pipe reader/writer reference counts.

### Poll

```rust
#[repr(C)]
struct PollFd { fd: i32, events: i16, revents: i16 }

const POLLIN:  i16 = 0x0001;
const POLLOUT: i16 = 0x0004;

let mut fds = [
    PollFd { fd: 0, events: POLLIN,  revents: 0 },
    PollFd { fd: 1, events: POLLOUT, revents: 0 },
];
let ready = unsafe {
    libmorpheus::raw::syscall3(
        libmorpheus::raw::SYS_POLL,
        fds.as_mut_ptr() as u64,
        fds.len() as u64,
        500, // timeout ms; 0 = non-blocking; !0 = sleep up to N ms
    )
};
if fds[0].revents & POLLIN != 0 { /* stdin has data */ }
```

### Futex

```rust
use core::sync::atomic::{AtomicU32, Ordering};
use libmorpheus::raw::{syscall3, SYS_FUTEX, FUTEX_WAIT, FUTEX_WAKE};

static GATE: AtomicU32 = AtomicU32::new(0);

// Thread A: wait until GATE != 0
unsafe { syscall3(SYS_FUTEX, &GATE as *const _ as u64, FUTEX_WAIT, 0); }

// Thread B: open the gate
GATE.store(1, Ordering::Release);
unsafe { syscall3(SYS_FUTEX, &GATE as *const _ as u64, FUTEX_WAKE, 1); }
```

Optional 4th argument to `FUTEX_WAIT`: timeout in milliseconds.

---

## 8. Hardware Driver Development

MorpheusX is an exokernel.  Ring-3 processes can write complete device drivers
without any kernel changes.  All primitives are in `libmorpheus::hw`.

### Pattern: PCI MMIO device

```rust
use libmorpheus::hw::*;

fn find_device(vendor: u16, device: u16) -> Option<(u8, u8, u8)> {
    for bus in 0u8..=255 {
        for dev in 0u8..32 {
            if pci_cfg_read16(bus, dev, 0, 0x00) == vendor
            && pci_cfg_read16(bus, dev, 0, 0x02) == device {
                return Some((bus, dev, 0));
            }
        }
    }
    None
}

fn init_device(bus: u8, dev: u8, fn_: u8) -> u64 {
    // Read MMIO BAR0 (mask off type/prefetch bits)
    let bar0 = (pci_cfg_read32(bus, dev, fn_, 0x10) & !0xF) as u64;

    // Enable bus mastering + memory space
    let cmd = pci_cfg_read16(bus, dev, fn_, 0x04);
    pci_cfg_write16(bus, dev, fn_, 0x04, cmd | 0x06);

    // Map MMIO registers (writable, uncacheable)
    let mmio = map_mmio(bar0, 16).unwrap();

    // Allocate DMA ring buffer below 4 GB
    let ring_phys = dma_alloc(4).unwrap();
    cache_flush(ring_phys, 4 * 4096).unwrap();

    // Attach IRQ
    irq_attach(11).unwrap();

    mmio
}

fn read_reg(mmio: u64, offset: u64) -> u32 {
    unsafe { core::ptr::read_volatile((mmio + offset) as *const u32) }
}

fn write_reg(mmio: u64, offset: u64, val: u32) {
    unsafe { core::ptr::write_volatile((mmio + offset) as *mut u32, val) }
}
```

### Pattern: framebuffer renderer

```rust
use libmorpheus::hw::{fb_info, fb_map};

let info = fb_info().unwrap();
let fb   = fb_map().unwrap();

let pixels = unsafe {
    core::slice::from_raw_parts_mut(fb as *mut u32, (info.stride / 4 * info.height) as usize)
};

// Clear screen to black
pixels.fill(0x000000FF);

// Draw a filled rectangle (x0,y0)-(x1,y1) in red
let (x0, y0, x1, y1) = (100u32, 100u32, 300u32, 200u32);
for row in y0..y1 {
    let base = (row * info.stride / 4 + x0) as usize;
    pixels[base..base + (x1 - x0) as usize].fill(0xFF0000FF);
}
```

### Pattern: x86 port I/O device

```rust
use libmorpheus::hw::{port_inb, port_outb};

// Read a byte from COM1 UART
const COM1: u16 = 0x3F8;
fn uart_read() -> u8 {
    while port_inb(COM1 + 5) & 0x01 == 0 {}  // wait until data ready
    port_inb(COM1)
}

fn uart_write(byte: u8) {
    while port_inb(COM1 + 5) & 0x20 == 0 {}  // wait until TX empty
    port_outb(COM1, byte);
}
```

---

## 9. Async Programming Model

MorpheusX provides two complementary concurrency models.  Both are fully
implemented and can be mixed freely.

| Model | When to use |
|---|---|
| `libmorpheus::thread` | CPU-bound work, simple parallelism, blocking-IO loops |
| `libmorpheus::task` | I/O-bound work, many concurrent connections |

### Async fundamentals

The async executor is a futex-backed cooperative scheduler.  There is no
preemption inside the async runtime; tasks yield voluntarily.  Blocking
OS syscalls (like `TcpStream::recv_blocking`) should be called from threads,
not from async tasks, unless wrapped in their own `spawn` so they do not
starve other tasks.

```rust
use libmorpheus::task::{block_on, yield_now, sleep, Runtime};

// Simple: await one future.
let result = block_on(async { 42u32 });

// Structured concurrency: runtime with multiple tasks.
let rt = Runtime::new();
rt.spawn(async {
    sleep(200).await;
    println!("200ms later");
});
rt.spawn(async {
    for _ in 0..5 {
        yield_now().await;  // let other tasks run
        println!("tick");
    }
});
rt.run();
```

### Multi-threaded execution

```rust
let rt = Runtime::new();
for i in 0..100 {
    rt.spawn(async move {
        sleep(10).await;
        println!("task {i}");
    });
}
rt.run_threaded(4);  // 4 OS threads pulling from the shared queue
```

### Mixing async and threads

```rust
let rt = Runtime::new();
let handle = rt.spawn_with_handle(async {
    // I/O-bound work in async
    sleep(50).await;
    "done"
});

// Meanwhile, CPU work on a thread
let t = libmorpheus::thread::spawn(|| {
    let mut acc = 0u64;
    for i in 0..1_000_000 { acc += i; }
    acc
}).unwrap();

let async_result = block_on(handle);
let thread_result = t.join().unwrap();
println!("async={async_result} thread={thread_result}");
```

---

## 10. Capability Matrix

Every entry in this table was verified from source code.

### Core OS

| Capability | Status |
|---|---|
| Preemptive scheduling (~100 Hz) | Yes |
| Ring 0 / Ring 3 separation with page-fault isolation | Yes |
| Multiple processes | Yes |
| OS threads (create / join / exit) | Yes |
| HelixFS (log-structured, copy-on-write, snapshots) | Yes |
| Persistent KV store | Yes |
| Virtual memory (mmap / munmap / mprotect) | Yes |
| Shared memory between processes | Yes |
| Pipes (unidirectional, blocking) | Yes |
| fd poll() with timeout | Yes |
| Futex (WAIT / WAKE) | Yes |
| Signal delivery (SIGACTION) | Yes |
| Process priority | Yes |
| Process list (SYS_PS) | Yes |
| ELF64 / PE32+ binary loading | Yes |
| Binary introspection (pe_info) | Yes |

### Networking

| Capability | Status |
|---|---|
| TCP (connect / listen / accept / send / recv) | Yes |
| UDP (send_to / recv_from) | Yes |
| DNS (async + blocking resolve + server override) | Yes |
| DHCP | Yes (automatic, kernel-side) |
| Static IP | Yes |
| Network statistics | Yes |
| Raw NIC ring access from Ring 3 | Yes |
| TLS / HTTPS | Userland library — port any `no_std` TLS crate over `TcpStream` |
| Custom NIC driver from Ring 3 | Yes — `hw::map_mmio` + DMA + IRQ + `SYS_NIC_TX/RX` |
| IPv6 | Not wired up (smoltcp has support) |
| ICMP from Ring 3 | Not exposed directly; achievable via raw `SYS_NIC_TX/RX` |

### Hardware access from Ring 3

| Capability | Status |
|---|---|
| Port I/O (inb/inw/inl/outb/outw/outl) | Yes |
| PCI config space (read/write 8/16/32-bit) | Yes |
| DMA-safe memory allocation (below 4 GB) | Yes |
| Physical -> virtual mapping (MMIO) | Yes |
| Virtual -> physical translation | Yes |
| IRQ attach / EOI | Yes |
| CPU cache flush | Yes |
| CPUID | Yes |
| RDTSC (raw + calibrated) | Yes |
| Framebuffer (map + info) | Yes |
| Boot log | Yes |
| Physical memory map | Yes |

### What you can build today

The table below uses the exokernel framing correctly: if the kernel exposes
the primitives needed, the application is feasible — the application brings
its own protocol libraries.

| Application | Assessment |
|---|---|
| Shell / command interpreter | Fully feasible |
| File manager | Fully feasible |
| Plain HTTP client | Fully feasible |
| Plain HTTP server | Fully feasible |
| UDP-based protocol daemon | Fully feasible |
| Database (file-backed) | Fully feasible |
| Custom NIC driver (from Ring 3) | Fully feasible — use `hw::map_mmio`, `dma_alloc`, `irq_attach` |
| Framebuffer rendering engine | Fully feasible |
| Process manager / init system | Fully feasible |
| IRC / plain-TCP chat daemon | Fully feasible |
| SSH daemon | Feasible — port a `no_std` crypto crate for the crypto layer; all TCP/process primitives exist |
| HTTPS client | Feasible — port a `no_std` TLS crate (e.g. `embedded-tls`) over `TcpStream` |
| Web browser | Feasible long-term — needs userland TLS + HTML parser + font engine; all kernel primitives exist |
| Custom TLS stack | Feasible — implement as a userland library over `TcpStream`; no kernel changes needed |

### Turing-completeness assessment

MorpheusX is **computationally Turing-complete**.  You can write and run
arbitrary programs, spawn threads, do I/O, listen on sockets, write hardware
drivers from Ring 3, and manage memory at the page level.

The exokernel model means **there are no kernel gaps for userspace
functionality** — only userspace libraries that have not yet been written.
TLS, SSH, custom NICs, ICMP, and any other protocol are all implementable
as pure userland code on top of the primitives already exposed:

- Crypto library → `no_std` Rust crate, links into your app, zero syscall changes
- TLS → state machine over `TcpStream` (recv bytes in, send bytes out)
- Custom NIC driver → `hw::map_mmio` + `hw::dma_alloc` + `hw::irq_attach` + `SYS_NIC_TX/RX`
- New filesystem → `hw::map_phys` for the block device, HelixFS VFS is optional

This is the design intent.  The kernel's surface area is fixed and minimal.
Every abstraction above it lives in userspace and is owned by the application.

---

## 11. Example Applications

### Hello World

```rust
#![no_std]
#![no_main]

extern crate libmorpheus;
use libmorpheus::process;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    libmorpheus::println!("Hello, MorpheusX!");
    process::exit(0);
}
```

### File I/O

```rust
use libmorpheus::fs;

fs::write_bytes("/data/log.txt", b"started\n").unwrap();

let text = fs::read_to_string("/data/log.txt").unwrap();
print!("{text}");

for entry in fs::read_dir("/data").unwrap() {
    let e = entry.unwrap();
    println!("{:20} {:10} bytes", e.name(), e.file_size());
}
```

### Persistent settings

```rust
use libmorpheus::persist;

persist::put("cfg/theme", b"dark").unwrap();
persist::put("cfg/volume", &75u32.to_le_bytes()).unwrap();

let mut buf = [0u8; 32];
let n = persist::get("cfg/theme", &mut buf).unwrap();
println!("theme = {}", core::str::from_utf8(&buf[..n]).unwrap());
```

### Threaded TCP echo server

```rust
use libmorpheus::net::TcpListener;
use libmorpheus::io::Write;
use libmorpheus::thread;

let listener = TcpListener::bind(7777).unwrap();
println!("listening on :7777");

loop {
    let mut conn = listener.accept_blocking().unwrap();
    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        loop {
            let n = match conn.recv_blocking(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            conn.write_all(&buf[..n]).unwrap();
        }
    }).unwrap();
}
```

### Async concurrent connections

```rust
use libmorpheus::task::{Runtime, sleep};
use libmorpheus::net::TcpStream;
use libmorpheus::io::Write;

let rt = Runtime::new();

for port in [80u16, 8080, 9090] {
    rt.spawn(async move {
        match TcpStream::connect_host("example.com", port) {
            Ok(mut s) => {
                write!(s, "HEAD / HTTP/1.0\r\nHost: example.com\r\n\r\n").ok();
                sleep(1000).await;
                println!("port {port}: connected");
            }
            Err(e) => println!("port {port}: {e}"),
        }
    });
}

rt.run_threaded(3);
```

### System information

```rust
use libmorpheus::sys::{SysInfo, sysinfo};

let mut info = SysInfo::zeroed();
sysinfo(&mut info).unwrap();

println!("uptime:    {} ms",  info.uptime_ms());
println!("processes: {}",     info.num_procs);
println!("free mem:  {} MiB", info.free_mem / 1024 / 1024);
println!("heap used: {} KiB", info.heap_used / 1024);
```

### MMIO device register access

```rust
use libmorpheus::hw::*;

let bar0 = (pci_cfg_read32(0, 4, 0, 0x10) & !0xF) as u64;
let mmio  = map_mmio(bar0, 4).unwrap();

let status = unsafe { core::ptr::read_volatile((mmio + 0x08) as *const u32) };
println!("device status: {status:#010x}");

irq_attach(11).unwrap();
// ... service the interrupt ...
irq_ack(11).unwrap();
```

### Spawning and waiting on a child process

```rust
use libmorpheus::process::Command;

let status = Command::new("/bin/compress")
    .arg("--level=9")
    .arg("/data/archive.tar")
    .status()
    .unwrap();

println!("{}", if status.success() { "done" } else { "failed" });
```

---

*Verified from: `hwinit/src/syscall/handler.rs`, `hwinit/src/syscall/mod.rs`,
`libmorpheus/src/`, `network/src/`, `ping/src/`*
