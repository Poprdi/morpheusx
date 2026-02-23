# MorpheusX Syscall ABI Reference

> **Version**: 1.1 — Full audit  
> **Date**: 2026-02-23  
> **Status**: Stable — numbers 0-45 are allocated; breaking changes require a version bump.

---

## Audit Summary

| Category       | Working | Buggy | Stubs | Total |
|----------------|---------|-------|-------|-------|
| Core (0-9)     | 10      | 0     | 0     | 10    |
| HelixFS (10-21)| 9       | 0     | 3     | 12    |
| System (22-31) | 8       | 2     | 0     | 10    |
| Network (32-41)| 0       | 0     | 10    | 10    |
| Device (42-45) | 0       | 0     | 4     | 4     |
| **TOTAL**      | **27**  | **2** | **17**| **46**|

Legend: **Working** = handler implemented and functional. **Buggy** = handler
exists but contains known correctness or safety issues. **Stubs** = returns
`-ENOSYS`, no backend exists.

### Known Bugs (must fix before first userspace release)

| Nr | Syscall   | Bug | Severity |
|----|-----------|-----|----------|
| 1  | `WRITE`   | Data buffer `ptr` not passed through `validate_user_buf()` — a malicious/buggy user can crash the kernel by passing a kernel-space pointer | HIGH |
| 2  | `READ`    | Same as WRITE — data buffer not range-checked against USER_ADDR_LIMIT | HIGH |
| 13 | `STAT`    | `stat_buf` pointer checked for `!= 0` but not validated with `validate_user_buf()` | HIGH |
| 14 | `READDIR` | `buf_ptr` not validated — copies DirEntry array to arbitrary address | HIGH |
| 26 | `MMAP`    | For kernel PID 0 (cr3=0), `mmap_brk` lazy-inits to `USER_MMAP_BASE` and tries to map into a nonexistent per-process page table. Also zeroes memory via direct physical pointer (works only because identity-mapped). | MEDIUM |
| 27 | `MUNMAP`  | Uses `kunmap_4k()` which operates on the **kernel** page table, not the per-process one. Does NOT free physical memory (no reverse mapping). `pages_allocated` counter decremented but pages are leaked. | MEDIUM |

---

## Calling Convention

MorpheusX uses the `syscall` / `sysret` mechanism on x86-64.

| Register | Purpose                |
|----------|------------------------|
| `RAX`    | Syscall number (in), return value (out) |
| `RDI`    | Argument 1             |
| `RSI`    | Argument 2             |
| `RDX`    | Argument 3             |
| `R10`    | Argument 4             |
| `R8`     | Argument 5             |
| `RCX`    | Clobbered by `syscall` (saved RIP) |
| `R11`    | Clobbered by `syscall` (saved RFLAGS) |

**Return convention**: `RAX` holds the result.

- `0` = success (for void-returning syscalls)
- Positive value = data (fd, pid, address, byte count, etc.)
- `u64::MAX - errno` = error (e.g., `u64::MAX - 2` = `-ENOENT`)

### Error Codes

| Name      | Value              | Errno |
|-----------|--------------------|-------|
| `EINVAL`  | `u64::MAX`         | 255   |
| `ENOENT`  | `u64::MAX - 2`     | 2     |
| `ESRCH`   | `u64::MAX - 3`     | 3     |
| `EIO`     | `u64::MAX - 5`     | 5     |
| `EBADF`   | `u64::MAX - 9`     | 9     |
| `ENOMEM`  | `u64::MAX - 12`    | 12    |
| `EACCES`  | `u64::MAX - 13`    | 13    |
| `EFAULT`  | `u64::MAX - 14`    | 14    |
| `EEXIST`  | `u64::MAX - 17`    | 17    |
| `EISDIR`  | `u64::MAX - 21`    | 21    |
| `EMFILE`  | `u64::MAX - 24`    | 24    |
| `ENOSPC`  | `u64::MAX - 28`    | 28    |
| `EROFS`   | `u64::MAX - 30`    | 30    |
| `ENOSYS`  | `u64::MAX - 37`    | 37    |
| `ENOTEMPTY`| `u64::MAX - 39`   | 39    |

Use `libmorpheus::is_error(ret)` to check: returns `true` when `ret > 0xFFFF_FFFF_FFFF_FF00`.

---

## Syscall Table

### Core (0-9) — all working

| Nr | Name       | Args                        | Return          | Status |
|----|------------|-----------------------------|-----------------|--------|
| 0  | `EXIT`     | `(code: i32)`               | never returns   | ✅     |
| 1  | `WRITE`    | `(fd, ptr, len)`            | bytes written   | ⚠️ ptr not validated |
| 2  | `READ`     | `(fd, ptr, len)`            | bytes read      | ⚠️ ptr not validated |
| 3  | `YIELD`    | `()`                        | 0               | ✅     |
| 4  | `ALLOC`    | `(pages)`                   | phys_base       | ✅     |
| 5  | `FREE`     | `(phys_base, pages)`        | 0               | ✅     |
| 6  | `GETPID`   | `()`                        | pid             | ✅     |
| 7  | `KILL`     | `(pid, signal)`             | 0               | ✅     |
| 8  | `WAIT`     | `(pid)`                     | exit_code       | ✅     |
| 9  | `SLEEP`    | `(millis)`                  | 0               | ✅     |

#### `EXIT` (0)
Terminates the calling process. Sets state to `Zombie`. `code` is stored as
the exit status. If the parent is blocked on `WAIT`, it is woken. Resources
(kernel stack, user page tables) are freed by `free_process_resources()`.

#### `WRITE` (1)
- fd 1 (stdout) / fd 2 (stderr): writes to serial console.
  - UTF-8 strings go through `puts()`; raw bytes fall back to `putc()`.
- fd ≥ 3: writes to a VFS file descriptor via `vfs_write()`.
- Max `len`: 1 MiB.
- **BUG**: does not call `validate_user_buf()` on `ptr` — kernel will
  blindly dereference any address the user passes.

#### `READ` (2)
- fd 0 (stdin): reads from the kernel keyboard ring buffer (256-byte SPSC).
  Returns immediately with however many bytes are available (non-blocking).
- fd ≥ 3: reads from a VFS file descriptor via `vfs_read()`.
- Max `len`: 1 MiB.
- **BUG**: same as WRITE — `ptr` not validated.

#### `YIELD` (3)
Executes `sti; hlt; cli` — atomic relinquish of CPU until the next timer
tick. The scheduler context-switches away; when this process is scheduled
again, execution resumes here.

#### `ALLOC` (4)
Allocates contiguous physical pages from the kernel memory registry.
Max 1024 pages (4 MiB) per call. Returns the physical base address.
Checks `is_registry_initialized()` first.

#### `FREE` (5)
Frees pages previously allocated with `ALLOC`. The `phys_base` and `pages`
must **exactly** match the values from `ALLOC` — MemoryRegistry does not
support partial frees (has a TODO for coalescing).

#### `KILL` (7)
Send a signal to a process. Supported signals:
- `SIGKILL` (9) — immediate termination (cannot be caught)
- `SIGTERM` (15) — request graceful shutdown
- `SIGCONT` (18) — resume paused process
- `SIGSTOP` (19) — pause process

Returns `-ESRCH` if PID not found. Returns `-EINVAL` if signal number
is not recognized by `Signal::from_u8()`.

#### `WAIT` (8)
Blocks the caller until child `pid` exits. If the child is already a Zombie,
reaps immediately and returns the exit code. If `pid` is not a child of the
caller, returns `-ESRCH`. Implemented via `BlockReason::WaitChild(pid)`.

#### `SLEEP` (9)
Suspends the calling process for at least `millis` milliseconds.
Computes a TSC deadline via `read_tsc() + millis * (tsc_freq / 1000)`.
If TSC is not calibrated, returns 0 immediately (no-op).

---

### HelixFS (10-21)

| Nr | Name       | Args                              | Return      | Status |
|----|------------|-----------------------------------|-------------|--------|
| 10 | `OPEN`     | `(path_ptr, path_len, flags)`     | fd          | ✅     |
| 11 | `CLOSE`    | `(fd)`                            | 0           | ✅     |
| 12 | `SEEK`     | `(fd, offset, whence)`            | new_offset  | ✅     |
| 13 | `STAT`     | `(path_ptr, path_len, stat_buf)`  | 0           | ⚠️ stat_buf not validated |
| 14 | `READDIR`  | `(path_ptr, path_len, buf_ptr)`   | count       | ⚠️ buf_ptr not validated |
| 15 | `MKDIR`    | `(path_ptr, path_len)`            | 0           | ✅     |
| 16 | `UNLINK`   | `(path_ptr, path_len)`            | 0           | ✅     |
| 17 | `RENAME`   | `(old_ptr, old_len, new_ptr, new_len)` | 0      | ✅     |
| 18 | `TRUNCATE` | `(fd, new_size)`                  | -ENOSYS     | 🚫 stub (no `vfs_truncate`) |
| 19 | `SYNC`     | `()`                              | 0           | ✅     |
| 20 | `SNAPSHOT` | `(name_ptr, name_len)`            | -ENOSYS     | 🚫 stub (no `vfs_snapshot`) |
| 21 | `VERSIONS` | `(path_ptr, path_len, buf, max)`  | -ENOSYS     | 🚫 stub (no `vfs_versions`) |

All VFS-backed syscalls delegate to `morpheus_helix::vfs::vfs_*()` functions.
The `helix` crate currently implements: `vfs_open`, `vfs_read`, `vfs_write`,
`vfs_seek`, `vfs_close`, `vfs_stat`, `vfs_readdir`, `vfs_mkdir`,
`vfs_unlink`, `vfs_rename`, `vfs_sync` (11 total). It does **NOT** implement
`vfs_truncate`, `vfs_snapshot`, or `vfs_versions`.

Path arguments go through `user_path()` which validates: `ptr != 0`,
`len ≤ 255`, valid UTF-8. However, `user_path()` does **NOT** call
`validate_user_buf()` — it does not check that the pointer is in
user-space (below `0x0000_8000_0000_0000`).

#### `OPEN` (10) — flags
| Flag       | Value  | Meaning              |
|------------|--------|----------------------|
| `O_READ`   | `0x01` | Open for reading     |
| `O_WRITE`  | `0x02` | Open for writing     |
| `O_CREATE` | `0x04` | Create if not exists |
| `O_TRUNC`  | `0x10` | Truncate on open     |
| `O_APPEND` | `0x20` | Append mode          |

File descriptors are per-process (each process has its own `fd_table`).
fd 0/1/2 are reserved for stdin/stdout/stderr.

#### `SEEK` (12) — whence
| Value | Meaning  |
|-------|----------|
| 0     | SEEK_SET |
| 1     | SEEK_CUR |
| 2     | SEEK_END |

#### `STAT` (13)
Fills a `FileStat` struct at `stat_buf`. **BUG**: `stat_buf` is only
checked for `!= 0`, not validated as a user-space address.
```rust
#[repr(C)]
pub struct FileStat {
    pub size: u64,
    pub created: u64,
    pub modified: u64,
    pub is_dir: bool,
    // ... (see morpheus_helix::types::FileStat for exact layout)
}
```

#### `READDIR` (14)
Returns the number of directory entries. Copies `DirEntry` structs to
`buf_ptr`. **BUG**: `buf_ptr` is only checked for `!= 0`, not validated —
caller can write DirEntry array to an arbitrary kernel address.

---

### System / Process / Memory (22-31)

| Nr | Name       | Args                    | Return      | Status |
|----|------------|-------------------------|-------------|--------|
| 22 | `CLOCK`    | `()`                    | nanoseconds | ✅     |
| 23 | `SYSINFO`  | `(buf_ptr)`             | 0           | ✅ validated |
| 24 | `GETPPID`  | `()`                    | parent_pid  | ✅     |
| 25 | `SPAWN`    | `(path_ptr, path_len)`  | child_pid   | ✅ untested e2e |
| 26 | `MMAP`     | `(pages)`               | virt_addr   | ⚠️ buggy for PID 0 |
| 27 | `MUNMAP`   | `(vaddr, pages)`        | 0           | ⚠️ wrong page table, leaks phys mem |
| 28 | `DUP`      | `(old_fd)`              | new_fd      | ✅     |
| 29 | `SYSLOG`   | `(ptr, len)`            | len         | ✅ validated |
| 30 | `GETCWD`   | `(buf_ptr, buf_len)`    | cwd_len     | ✅ validated |
| 31 | `CHDIR`    | `(path_ptr, path_len)`  | 0           | ✅     |

#### `CLOCK` (22)
Returns monotonic nanoseconds since boot, derived from the TSC.
Uses 128-bit arithmetic: `nanos = (tsc as u128) * 1_000_000_000 / (freq as u128)`.
Returns 0 if the TSC has not been calibrated.

#### `SYSINFO` (23)
Fills a `SysInfo` struct at `buf_ptr`. **Properly validates** the pointer
with `validate_user_buf()`.
```rust
#[repr(C)]
pub struct SysInfo {
    pub total_mem: u64,      // Total physical memory (bytes)
    pub free_mem: u64,       // Free physical memory (bytes)
    pub num_procs: u32,      // Number of live processes
    pub _pad0: u32,
    pub uptime_ticks: u64,   // TSC ticks since boot
    pub tsc_freq: u64,       // TSC frequency (Hz)
    pub heap_total: u64,     // Kernel heap total (bytes)
    pub heap_used: u64,      // Kernel heap used (bytes)
    pub heap_free: u64,      // Kernel heap free (bytes)
}
```

#### `GETPPID` (24)
Returns `parent_pid` from the current process struct. Returns 0 for
the kernel (PID 0) since it has no parent.

#### `SPAWN` (25)
Reads an ELF binary from the VFS at the given path and spawns it as
a new user process. Returns the child PID. Flow:
1. `vfs_open()` + `vfs_stat()` to get file size
2. Allocate temporary physical pages for read buffer
3. `vfs_read()` entire file
4. `spawn_user_process(name, elf_data)` — parses ELF, creates page tables,
   sets up Ring 3 context
5. Free temporary buffer, return child PID

Max ELF size: 4 MiB. Child inherits no file descriptors.

**Note**: Has been code-reviewed but not tested end-to-end.

#### `MMAP` (26)
Allocates `pages` physical pages and maps them into the calling
process's virtual address space. Virtual addresses start from
`0x0000_0040_0000_0000` and grow upward (`mmap_brk`).

**BUG**: For kernel PID 0 (`cr3 = 0`), `mmap_brk` is lazy-initialized
to `USER_MMAP_BASE`, then `PageTableManager { pml4_phys: 0 }` tries
to walk a page table at physical address 0 — this is the real-mode IVT,
not a page table. Will corrupt memory or triple-fault.

**BUG**: Zeroes physical memory via direct physical pointer
(`write_bytes(phys as *mut u8, 0, ...)`), which only works because memory
is identity-mapped. Once processes use separate address spaces, this must
use the virtual address.

Max 1024 pages per call.

#### `MUNMAP` (27)
Walks pages and calls `kunmap_4k()` for each.

**BUG**: `kunmap_4k()` operates on the **kernel** page table (reads CR3),
not the per-process page table (`proc.cr3`).

**BUG**: Does not free physical memory — the physical pages backing the
virtual mappings are leaked. No reverse mapping exists to recover them.

#### `DUP` (28)
Duplicates a file descriptor. Copies the `FdDescriptor` struct (including
VFS superblock key + offset). The new fd shares the same offset counter.

#### `SYSLOG` (29)
Writes a message to the kernel serial log. Uses `validate_user_buf()`.
Messages prefixed with `[USR] `. Falls back to raw byte output if not
valid UTF-8.

#### `GETCWD` (30)
Copies the current working directory into `buf_ptr`. Uses
`validate_user_buf()`. Returns the actual CWD length (may be larger
than `buf_len` — caller should check and re-allocate if needed).

#### `CHDIR` (31)
Changes the calling process's CWD. Validates the path exists via
`vfs_stat()`. **Does not check that the path is a directory** (TODO).

---

### Networking (32-41) — All Stubs

All networking syscalls return `-ENOSYS`. The ABI numbers are
stable and will be implemented once the kernel's network crate is
restructured from its monolithic HTTP-client design into a proper
socket layer (the current `network/` crate is an HTTP download
client with DMA buffers — it cannot back BSD sockets).

| Nr | Name         | Planned Args                     |
|----|--------------|----------------------------------|
| 32 | `SOCKET`     | `(domain, type, protocol) → fd`  |
| 33 | `CONNECT`    | `(fd, addr_ptr, addr_len) → 0`   |
| 34 | `SEND`       | `(fd, buf_ptr, buf_len) → sent`  |
| 35 | `RECV`       | `(fd, buf_ptr, buf_len) → recv`  |
| 36 | `BIND`       | `(fd, addr_ptr, addr_len) → 0`   |
| 37 | `LISTEN`     | `(fd, backlog) → 0`              |
| 38 | `ACCEPT`     | `(fd, addr_ptr, addr_len) → fd`  |
| 39 | `SHUTDOWN`   | `(fd, how) → 0`                  |
| 40 | `SETSOCKOPT` | `(fd, level, opt, val, len) → 0` |
| 41 | `DNS_RESOLVE`| `(name_ptr, name_len, out) → 0`  |

---

### Device / Mount (42-45) — All Stubs

| Nr | Name     | Planned Args                        | Status |
|----|----------|-------------------------------------|--------|
| 42 | `IOCTL`  | `(fd, cmd, arg)`                    | 🚫 stub |
| 43 | `MOUNT`  | `(src_ptr, src_len, dst_ptr, dst_l)`| 🚫 stub |
| 44 | `UMOUNT` | `(path_ptr, path_len)`              | 🚫 stub |
| 45 | `POLL`   | `(fds_ptr, nfds, timeout_ms)`       | 🚫 stub |

---

## User Pointer Validation

`validate_user_buf(ptr, len)` in `handler.rs` checks:
- `ptr != 0` and `len != 0`
- `ptr + len` does not overflow
- `ptr + len <= 0x0000_8000_0000_0000` (canonical user-space boundary)

**Syscalls that correctly use `validate_user_buf()`**: SYSINFO (23), SYSLOG (29), GETCWD (30).

**Syscalls that do NOT validate data buffers**: WRITE (1), READ (2), STAT (13), READDIR (14).

`user_path()` in `handler.rs` validates paths: `ptr != 0`, `len ≤ 255`, valid UTF-8.
But it does **NOT** check that the address is in user-space. A malicious caller could
pass a kernel-space pointer as a "path" and the kernel would read from it.

### Recommendation

Before the first userspace release, add `validate_user_buf()` calls to:
- `sys_write()` — validate `ptr` + `len`
- `sys_read()` — validate `ptr` + `len`
- `sys_fs_stat()` — validate `stat_buf` with `sizeof(FileStat)`
- `sys_fs_readdir()` — validate `buf_ptr` (need to know max entries)
- `user_path()` — add `validate_user_buf(ptr, len)` check

---

## Persistence Crate Status

The `morpheus-persistent` crate exists at `/persistent` and is registered in the
workspace `Cargo.toml`. However, it is **completely unwired** — no other crate
in the project depends on it.

| Property | Value |
|----------|-------|
| Crate | `morpheus-persistent` |
| Path | `persistent/` |
| Dependents | **NONE** — zero `[dependencies]` edges from bootloader or hwinit |
| Contains | PE/COFF parsing, memory capture, `PersistenceBackend` trait, ESP storage backend |
| Backend impl | `storage/esp.rs` — partially implemented |
| Purpose | Extract running UEFI bootloader from memory, create bootable disk image by reversing UEFI relocations |

**Current state**: The bootloader's `storage.rs` handles HelixFS mount directly
via `morpheus_helix::vfs::global::init_root_fs()` without involving the
persistence crate at all. To wire it up:
1. Add `morpheus-persistent = { path = "../persistent" }` to `bootloader/Cargo.toml`
2. Call `PersistenceBackend` during boot to save/restore state
3. Implement a concrete backend (the trait is defined but has no working impl)

---

## Feature Gap Analysis

### What's needed for `std` Rust support

To port Rust's `std` library to MorpheusX, we need a custom target spec
(`x86_64-unknown-morpheus`) and the following additional syscalls:

| Category | Syscalls Needed | Description |
|----------|----------------|-------------|
| **Threading** | `CLONE`, `FUTEX`, `EXIT_GROUP` | Thread creation, synchronization primitives, process-wide exit |
| **Memory** | `BRK`/`SBRK`, `MMAP` with `MAP_ANONYMOUS` | Heap growth for the userspace allocator (currently MMAP exists but only does phys-backed mapping) |
| **File I/O** | `FSTAT` (fd-based stat), `DUP2` (fd→specific fd), `FCNTL` (fd flags, O_NONBLOCK), `PIPE`, `FTRUNCATE` | Core stdio redirection and pipe plumbing |
| **Process** | `EXECVE`, `FORK` or `POSIX_SPAWN`, `GETENV`/`SETENV` | Process replacement, environment variable access |
| **Signals** | `SIGACTION`, `SIGPROCMASK`, `SIGRETURN` | Install signal handlers, block/unblock signals, return from handler |
| **Time** | `CLOCK_GETTIME` (struct-based), `NANOSLEEP` | POSIX-compatible time APIs (our CLOCK returns raw nanos, std wants `timespec`) |
| **Misc** | `GETUID`/`GETGID`, `UNAME`, `GETRANDOM` | Identity, system name, randomness for HashMap seeding |

**Estimate**: ~20-25 additional syscalls. The most critical are **CLONE + FUTEX**
(threading), **BRK** (allocator), and **PIPE + DUP2** (stdio).

### What's needed for async Rust (tokio/smol/embassy)

| Category | Syscalls Needed | Description |
|----------|----------------|-------------|
| **Event multiplexing** | `EPOLL_CREATE`, `EPOLL_CTL`, `EPOLL_WAIT` (or `POLL` #45) | Core async I/O reactor — the most critical piece |
| **Non-blocking I/O** | `FCNTL` with `O_NONBLOCK` | Required for all async I/O |
| **Timers** | `TIMERFD_CREATE`, `TIMERFD_SETTIME` (or integrate with SLEEP) | Async timeouts, intervals |
| **Waker channels** | `PIPE` + `EVENTFD` | Wake the reactor from another thread/signal handler |
| **Networking** | Syscalls 32-41 (SOCKET..DNS_RESOLVE) | Primary use case for async is network I/O |

**Estimate**: ~5-8 additional syscalls on top of the std requirements.
**Note**: A `no_std` async executor (like `embassy`) needs much less — mostly
just a timer and some form of event notification.

### Alternative: Stay `no_std` + libmorpheus

The current approach — building on `#![no_std]` with `libmorpheus` as the SDK —
is **viable for many applications** without needing any std support:

- File I/O works (OPEN/READ/WRITE/CLOSE/SEEK/STAT/READDIR/MKDIR/UNLINK/RENAME/SYNC)
- Process management works (SPAWN/WAIT/KILL/GETPID/GETPPID/EXIT)
- Timing works (CLOCK/SLEEP)
- Physical memory allocation works (ALLOC/FREE)
- Virtual memory works with caveats (MMAP/MUNMAP — see bugs above)
- System introspection works (SYSINFO/SYSLOG)
- HelixFS provides a complete filesystem

**What's missing for practical `no_std` apps**: primarily stdin blocking read
(current `READ` on fd 0 is non-blocking), `PIPE` for inter-process
communication, and the networking syscalls.

### Self-Hosting Roadmap

To compile Rust programs **on** MorpheusX:

1. **End-to-end test SPAWN** — verify ELF loading + execution in Ring 3
2. **Implement PIPE** — needed for compiler ↔ linker communication
3. **Implement DUP2** — needed for stdio redirection
4. **Port a Rust compiler binary** to HelixFS as a static ELF
5. **Add enough POSIX shims** for the compiler to run (file I/O already works)
6. **Ship an allocator** in libmorpheus that uses MMAP internally

---

## libmorpheus SDK Modules

The userspace SDK (`libmorpheus`) provides high-level wrappers:

| Module     | Syscalls wrapped                        |
|------------|-----------------------------------------|
| `process`  | exit, getpid, getppid, yield, kill, sleep, wait, spawn |
| `fs`       | open, close, read, write, seek, stat, readdir, mkdir, unlink, rename, sync, dup, getcwd, chdir |
| `io`       | print, println (convenience over WRITE) |
| `mem`      | alloc_pages, free_pages, mmap, munmap   |
| `time`     | clock_gettime, uptime_ms, uptime_us     |
| `sys`      | sysinfo (SysInfo struct), syslog        |
| `net`      | socket, connect, send, recv, dns_resolve (all return ENOSYS) |
| `raw`      | syscall0..syscall5 (inline asm)         |
| `entry`    | `entry!()` macro, panic handler         |

### Quick Start

```rust
#![no_std]
#![no_main]

use libmorpheus::entry;

entry!(main);

fn main() -> i32 {
    // Print to console
    libmorpheus::io::println("Hello from userspace!");

    // Get system info
    let mut info = libmorpheus::sys::SysInfo::zeroed();
    libmorpheus::sys::sysinfo(&mut info).unwrap();

    // Get time
    let nanos = libmorpheus::time::clock_gettime();

    // Spawn a child
    if let Ok(child_pid) = libmorpheus::process::spawn("/bin/hello") {
        libmorpheus::process::wait(child_pid).unwrap();
    }

    0
}
```
