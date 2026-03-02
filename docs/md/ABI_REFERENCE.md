# MorpheusX Syscall ABI Reference

> **Version**: 2.1 — Full exokernel ABI  
> **Date**: 2026-02-26  
> **Status**: Stable — syscall numbers 0-82 are allocated and implemented.

---

## Audit Summary

| Category                 | Implemented | Total |
|--------------------------|-------------|-------|
| Core (0-9)               | 10          | 10    |
| HelixFS (10-21)          | 12          | 12    |
| System (22-31)           | 10          | 10    |
| Networking (32-41)       | 10          | 10    |
| Device / Mount (42-45)   | 4           | 4     |
| Persistence (46-51)      | 6           | 6     |
| Hardware I/O (52-62)     | 11          | 11    |
| Display (63-64)          | 2           | 2     |
| Process Mgmt (65-68)     | 4           | 4     |
| CPU / Diagnostics (69-72)| 4           | 4     |
| Memory / IPC (73-79)     | 7           | 7     |
| Threading (80-82)        | 3           | 3     |
| **TOTAL**                | **83**      | **83**|

Legend: **Implemented** = handler is present in the syscall dispatcher and backend.

### Known Bugs

No currently tracked ABI-level correctness issues in this file's scope.
For current defects, rely on repository issues and test output (e.g. syscall-e2e).

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

| Name       | Value              | Errno |
|------------|--------------------|-------|
| `EINVAL`   | `u64::MAX`         | 255   |
| `ENOENT`   | `u64::MAX - 2`     | 2     |
| `ESRCH`    | `u64::MAX - 3`     | 3     |
| `EIO`      | `u64::MAX - 5`     | 5     |
| `EBADF`    | `u64::MAX - 9`     | 9     |
| `ENOMEM`   | `u64::MAX - 12`    | 12    |
| `EACCES`   | `u64::MAX - 13`    | 13    |
| `EFAULT`   | `u64::MAX - 14`    | 14    |
| `EEXIST`   | `u64::MAX - 17`    | 17    |
| `ENODEV`   | `u64::MAX - 19`    | 19    |
| `EISDIR`   | `u64::MAX - 21`    | 21    |
| `EMFILE`   | `u64::MAX - 24`    | 24    |
| `ENOSPC`   | `u64::MAX - 28`    | 28    |
| `EROFS`    | `u64::MAX - 30`    | 30    |
| `ENOSYS`   | `u64::MAX - 37`    | 37    |
| `ENOTEMPTY`| `u64::MAX - 39`    | 39    |

Use `libmorpheus::is_error(ret)` to check: returns `true` when `ret > 0xFFFF_FFFF_FFFF_FF00`.

---

## Syscall Table

### Core (0-9) — all working

| Nr | Name       | Args                        | Return          | Status |
|----|------------|-----------------------------|-----------------|--------|
| 0  | `EXIT`     | `(code: i32)`               | never returns   | ✅     |
| 1  | `WRITE`    | `(fd, ptr, len)`            | bytes written   | ✅     |
| 2  | `READ`     | `(fd, ptr, len)`            | bytes read      | ✅     |
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
- User pointer is validated with `validate_user_buf()`.

#### `READ` (2)
- fd 0 (stdin): reads from the kernel keyboard ring buffer (256-byte SPSC).
  Returns immediately with however many bytes are available (non-blocking).
- fd ≥ 3: reads from a VFS file descriptor via `vfs_read()`.
- Max `len`: 1 MiB.
- User pointer is validated with `validate_user_buf()`.

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
| 13 | `STAT`     | `(path_ptr, path_len, stat_buf)`  | 0           | ✅     |
| 14 | `READDIR`  | `(path_ptr, path_len, buf_ptr)`   | count       | ✅     |
| 15 | `MKDIR`    | `(path_ptr, path_len)`            | 0           | ✅     |
| 16 | `UNLINK`   | `(path_ptr, path_len)`            | 0           | ✅     |
| 17 | `RENAME`   | `(old_ptr, old_len, new_ptr, new_len)` | 0      | ✅     |
| 18 | `TRUNCATE` | `(path_ptr, path_len, new_size)`  | 0           | ✅     |
| 19 | `SYNC`     | `()`                              | 0           | ✅     |
| 20 | `SNAPSHOT` | `(name_ptr, name_len)`            | checkpoint  | ✅     |
| 21 | `VERSIONS` | `(path_ptr, path_len, buf, max)`  | count       | ✅     |

All VFS-backed syscalls delegate to `morpheus_helix::vfs::vfs_*()` functions.

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
Fills a `FileStat` struct at `stat_buf` after validating the output buffer.
```rust
#[repr(C)]
pub struct FileStat {
    pub size: u64,
    pub created: u64,
    pub modified: u64,
    pub is_dir: bool,
}
```

#### `READDIR` (14)
Returns the number of directory entries. Copies `DirEntry` structs to
`buf_ptr` after validating the output buffer range.

#### `TRUNCATE` (18)
Truncates a file at `path` to `new_size` bytes. Implemented as
open-with-`O_TRUNC` + close. The VFS truncates the file on open; the
`new_size` parameter is accepted for ABI forward-compatibility.

#### `SNAPSHOT` (20)
Syncs all VFS state to disk and returns a monotonic checkpoint ID
(TSC value). The `name` argument is reserved for future named snapshots.

#### `VERSIONS` (21)
Returns 0 entries (count = 0). The VFS doesn't yet expose `list_versions()`.
HelixFS supports CoW versioning at the block layer, but the VFS bridge
is not built. Graceful no-op — no error returned.

---

### System / Process / Memory (22-31)

| Nr | Name       | Args                    | Return      | Status |
|----|------------|-------------------------|-------------|--------|
| 22 | `CLOCK`    | `()`                    | nanoseconds | ✅     |
| 23 | `SYSINFO`  | `(buf_ptr)`             | 0           | ✅ validated |
| 24 | `GETPPID`  | `()`                    | parent_pid  | ✅     |
| 25 | `SPAWN`    | `(path_ptr, path_len)`  | child_pid   | ✅ untested e2e |
| 26 | `MMAP`     | `(pages)`               | virt_addr   | ✅     |
| 27 | `MUNMAP`   | `(vaddr, pages)`        | 0           | ✅     |
| 28 | `DUP`      | `(old_fd)`              | new_fd      | ✅     |
| 29 | `SYSLOG`   | `(ptr, len)`            | len         | ✅ validated |
| 30 | `GETCWD`   | `(buf_ptr, buf_len)`    | cwd_len     | ✅ validated |
| 31 | `CHDIR`    | `(path_ptr, path_len)`  | 0           | ✅     |

#### `CLOCK` (22)
Returns monotonic nanoseconds since boot, derived from the TSC.
Uses 128-bit arithmetic: `nanos = (tsc as u128) * 1_000_000_000 / (freq as u128)`.
Returns 0 if the TSC has not been calibrated.

#### `SYSINFO` (23)
```rust
#[repr(C)]
pub struct SysInfo {
    pub total_mem: u64,
    pub free_mem: u64,
    pub num_procs: u32,
    pub _pad0: u32,
    pub uptime_ticks: u64,
    pub tsc_freq: u64,
    pub heap_total: u64,
    pub heap_used: u64,
    pub heap_free: u64,
}
```

#### `SPAWN` (25)
Reads an ELF binary from the VFS and spawns it as a new user process.
Max ELF size: 4 MiB. Child inherits no file descriptors.

#### `MMAP` (26) / `MUNMAP` (27)
`MMAP` maps user pages in the caller's address space and records VMAs.
`MUNMAP` unmaps by VMA and frees owned physical pages.

#### `DUP` (28)
Duplicates a file descriptor.

#### `SYSLOG` (29)
Writes a message to the kernel serial log. Validated. Prefix: `[USR] `.

#### `GETCWD` (30) / `CHDIR` (31)
Get/set current working directory.

---

### Raw NIC (32-37) — Exokernel Network Primitives

MorpheusX is an exokernel: userland builds its own network stack.
These syscalls expose raw Ethernet frame TX/RX through a function-pointer
registration mechanism. The bootloader calls `register_nic(ops)` after
driver initialization to wire the hardware functions.

If no NIC driver has been registered, all syscalls return `-ENODEV`.

| Nr | Name         | Args                          | Return               | Status |
|----|--------------|-------------------------------|----------------------|--------|
| 32 | `NIC_INFO`   | `(buf_ptr)`                   | 0                    | ✅     |
| 33 | `NIC_TX`     | `(frame_ptr, frame_len)`      | 0                    | ✅     |
| 34 | `NIC_RX`     | `(buf_ptr, buf_len)`          | bytes_received       | ✅     |
| 35 | `NIC_LINK`   | `()`                          | 1=up, 0=down         | ✅     |
| 36 | `NIC_MAC`    | `(buf_ptr)`                   | 0                    | ✅     |
| 37 | `NIC_REFILL` | `()`                          | 0                    | ✅     |

#### `NIC_INFO` (32)
```rust
#[repr(C)]
pub struct NicInfo {
    pub mac: [u8; 8],     // 6-byte MAC, 2 bytes padding
    pub link_up: u32,     // 1 if link up
    pub present: u32,     // 1 if NIC registered
}
```

#### `NIC_TX` (33)
Frame must be 14–9000 bytes (Ethernet header minimum to jumbo max).

#### `NIC_RX` (34)
Non-blocking. Returns 0 if no frame pending.

#### `NIC_LINK` (35)
Returns 1 if link up, 0 if down.

#### `NIC_MAC` (36)
Copies 6-byte MAC address. Buffer ≥ 6 bytes.

#### `NIC_REFILL` (37)
Replenishes RX descriptor ring.

---

### Network Control (38-41)

| Nr | Name       | Args                    | Return | Status |
|----|------------|-------------------------|--------|--------|
| 38 | `NET`      | `(subcmd, a2, a3, a4)`  | result | ✅     |
| 39 | `DNS`      | `(subcmd, a2, a3)`      | result | ✅     |
| 40 | `NET_CFG`  | `(subcmd, a2, a3, a4)`  | result | ✅     |
| 41 | `NET_POLL` | `(subcmd, a2)`          | result | ✅     |

These are active multiplexed networking syscalls (not stubs).

---

### Device / Mount (42-45)

| Nr | Name     | Args                                 | Return  | Status |
|----|----------|--------------------------------------|---------|--------|
| 42 | `IOCTL`  | `(fd, cmd, arg)`                     | varies  | ✅     |
| 43 | `MOUNT`  | `(src_ptr, src_len, dst_ptr, dst_l)` | 0       | ✅     |
| 44 | `UMOUNT` | `(path_ptr, path_len)`               | 0       | ✅     |
| 45 | `POLL`   | `(fds_ptr, nfds, timeout_ms)`        | ready   | ✅     |

#### `IOCTL` (42)
| Command      | Value    | Arg     | Return            |
|--------------|----------|---------|-------------------|
| `FIONREAD`   | `0x541B` | fd      | bytes available   |
| `TIOCGWINSZ` | `0x5413` | buf_ptr | 0 (writes 80×25)  |

#### `MOUNT` (43)
No-op success. HelixFS auto-mounts at `/`.

#### `UMOUNT` (44)
Syncs VFS and returns 0.

#### `POLL` (45)
```rust
#[repr(C)]
pub struct PollFd {
    pub fd: i32,
    pub events: i16,   // POLLIN=1, POLLOUT=4
    pub revents: i16,  // POLLIN, POLLOUT, POLLERR=8
}
```
Timeout 0 = non-blocking. Does NOT support indefinite blocking.

---

### Persistence / Introspection (46-51)

| Nr | Name          | Args                                   | Return      | Status |
|----|---------------|----------------------------------------|-------------|--------|
| 46 | `PERSIST_PUT` | `(key_ptr, key_len, data_ptr, data_l)` | 0           | ✅     |
| 47 | `PERSIST_GET` | `(key_ptr, key_len, buf_ptr, buf_len)` | bytes_read  | ✅     |
| 48 | `PERSIST_DEL` | `(key_ptr, key_len)`                   | 0           | ✅     |
| 49 | `PERSIST_LIST`| `(buf_ptr, buf_len, offset)`           | count       | ✅     |
| 50 | `PERSIST_INFO`| `(info_ptr)`                           | 0           | ✅     |
| 51 | `PE_INFO`     | `(path_ptr, path_len, info_ptr)`       | 0           | ✅     |

See version 1.1 for full details. All pointers validated.

---

### Hardware I/O Primitives (52-62) — Exokernel

These syscalls expose raw hardware access to userland, enabling user-level
device drivers. This is the core exokernel philosophy: the kernel multiplexes
hardware, userland implements policy.

| Nr | Name           | Args                         | Return       | Status |
|----|----------------|------------------------------|--------------|--------|
| 52 | `PORT_IN`      | `(port, width)`              | value        | ✅     |
| 53 | `PORT_OUT`     | `(port, width, value)`       | 0            | ✅     |
| 54 | `PCI_CFG_READ` | `(bdf, offset, width)`       | value        | ✅     |
| 55 | `PCI_CFG_WRITE`| `(bdf, offset, width, value)`| 0            | ✅     |
| 56 | `DMA_ALLOC`    | `(pages)`                    | phys_addr    | ✅     |
| 57 | `DMA_FREE`     | `(phys, pages)`              | 0            | ✅     |
| 58 | `MAP_PHYS`     | `(phys, pages, flags)`       | virt_addr    | ✅     |
| 59 | `VIRT_TO_PHYS` | `(virt)`                     | phys_addr    | ✅     |
| 60 | `IRQ_ATTACH`   | `(irq_num)`                  | 0            | ✅     |
| 61 | `IRQ_ACK`      | `(irq_num)`                  | 0            | ✅     |
| 62 | `CACHE_FLUSH`  | `(addr, len)`                | 0            | ✅     |

#### `PORT_IN` (52) / `PORT_OUT` (53)
Raw x86 port I/O. `width`: 1 = byte, 2 = word, 4 = dword.
Port ≤ 0xFFFF. Returns `-EINVAL` for invalid width/port.

#### `PCI_CFG_READ` (54) / `PCI_CFG_WRITE` (55)
PCI configuration space. `bdf = (bus << 16) | (dev << 8) | func`.
`offset`: 0-255. `width`: 1, 2, or 4.

#### `DMA_ALLOC` (56) / `DMA_FREE` (57)
Contiguous pages below 4 GiB. Zeroed on alloc. Max 256 pages (1 MiB).

#### `MAP_PHYS` (58)
Maps physical pages into the process's virtual address space.
Flags: bit 0 = writable, bit 1 = uncacheable.
Process must have a valid `cr3` (not PID 0). Max 1024 pages.

#### `VIRT_TO_PHYS` (59)
Kernel page table walk. Returns 0 if unmapped.

#### `IRQ_ATTACH` (60) / `IRQ_ACK` (61)
PIC IRQ management. Range: 0-15.

#### `CACHE_FLUSH` (62)
`clflush` over address range. Max 64 MiB, page-aligned.

---

### Display (63-64)

| Nr | Name      | Args          | Return     | Status |
|----|-----------|---------------|------------|--------|
| 63 | `FB_INFO` | `(buf_ptr)`   | 0          | ✅     |
| 64 | `FB_MAP`  | `()`          | virt_addr  | ✅     |

#### `FB_INFO` (63)
```rust
#[repr(C)]
pub struct FbInfo {
    pub base: u64,      // physical address
    pub size: u64,      // bytes
    pub width: u32,     // pixels
    pub height: u32,    // pixels
    pub stride: u32,    // bytes per scanline
    pub format: u32,    // 0=RGBX, 1=BGRX
}
```
Returns `-ENODEV` if framebuffer not registered.

#### `FB_MAP` (64)
Maps framebuffer into process VA (writable + uncacheable).
Process must have a valid `cr3`.

---

### Process Management (65-68)

| Nr | Name          | Args                   | Return   | Status |
|----|---------------|------------------------|----------|--------|
| 65 | `PS`          | `(buf_ptr, max_count)` | count    | ✅     |
| 66 | `SIGACTION`   | `(signum, handler)`    | 0        | ✅     |
| 67 | `SETPRIORITY` | `(pid, priority)`      | 0        | ✅     |
| 68 | `GETPRIORITY` | `(pid)`                | priority | ✅     |

#### `PS` (65)
```rust
#[repr(C)]
pub struct PsEntry {
    pub pid: u32,
    pub ppid: u32,
    pub state: u32,     // 0=Ready, 1=Running, 2=Blocked, 3=Zombie, 4=Terminated
    pub priority: u32,
    pub name: [u8; 32], // NUL-padded
}
```

#### `SIGACTION` (66)
Registers handler address. Delivery mechanism (sigframe + sigreturn) not
yet built — placeholder for Phase 6.

#### `SETPRIORITY` (67) / `GETPRIORITY` (68)
Priority 0-255 (0 = highest). `-ESRCH` if PID not found.

---

### CPU Features / Diagnostics (69-72)

| Nr | Name       | Args                         | Return     | Status |
|----|------------|------------------------------|------------|--------|
| 69 | `CPUID`    | `(leaf, subleaf, result_ptr)`| 0          | ✅     |
| 70 | `RDTSC`    | `(result_ptr)`               | 0          | ✅     |
| 71 | `BOOT_LOG` | `(buf_ptr, buf_len)`         | bytes_read | ✅     |
| 72 | `MEMMAP`   | `(buf_ptr, max_entries)`     | count      | ✅     |

#### `CPUID` (69)
```rust
#[repr(C)]
pub struct CpuidResult { pub eax: u32, pub ebx: u32, pub ecx: u32, pub edx: u32 }
```

#### `RDTSC` (70)
```rust
#[repr(C)]
pub struct TscResult { pub tsc: u64, pub frequency: u64 }
```

#### `BOOT_LOG` (71)
Copies kernel serial log. If `buf_len == 0`, returns total size.

#### `MEMMAP` (72)
```rust
#[repr(C)]
pub struct MemmapEntry { pub base: u64, pub pages: u64, pub mem_type: u32, pub _pad: u32 }
```

---

### Memory / IPC / Threading (73-82)

| Nr | Name            | Args                              | Return      | Status |
|----|-----------------|-----------------------------------|-------------|--------|
| 73 | `SHM_GRANT`     | `(pid, vaddr, pages, flags)`      | target_vaddr| ✅     |
| 74 | `MPROTECT`      | `(vaddr, pages, prot)`            | 0           | ✅     |
| 75 | `PIPE`          | `(result_ptr)`                    | 0           | ✅     |
| 76 | `DUP2`          | `(old_fd, new_fd)`                | new_fd      | ✅     |
| 77 | `SET_FG`        | `(pid)`                           | 0           | ✅     |
| 78 | `GETARGS`       | `(buf_ptr, buf_len)`              | argc        | ✅     |
| 79 | `FUTEX`         | `(addr, op, val, timeout_ms)`     | result      | ✅     |
| 80 | `THREAD_CREATE` | `(entry, stack_top, arg)`         | tid         | ✅     |
| 81 | `THREAD_EXIT`   | `(code)`                          | never       | ✅     |
| 82 | `THREAD_JOIN`   | `(tid)`                           | exit_code   | ✅     |

---

## User Pointer Validation

`validate_user_buf(ptr, len)` checks: `ptr != 0`, `len != 0`,
no overflow, `ptr + len <= 0x0000_8000_0000_0000`.

**Validated (representative)**: WRITE, READ, STAT, READDIR, SYSINFO, SYSLOG,
GETCWD, PERSIST_*, PE_INFO, NIC_INFO, NIC_TX, NIC_RX, NIC_MAC, FB_INFO,
PS, CPUID, RDTSC, BOOT_LOG, MEMMAP.

---

## Registration APIs

### NIC Registration
```rust
use morpheus_hwinit::{register_nic, NicOps};
unsafe {
    register_nic(NicOps {
        tx: Some(driver_tx), rx: Some(driver_rx),
        link_up: Some(driver_link), mac: Some(driver_mac),
        refill: Some(driver_refill),
    });
}
```

### Framebuffer Registration
Called by bootloader before desktop entry. Enables SYS_FB_INFO / SYS_FB_MAP.

---

## libmorpheus SDK Modules

| Module     | Syscalls wrapped                        |
|------------|-----------------------------------------|
| `process`  | exit, getpid, getppid, yield, kill, sleep, wait, spawn, ps, sigaction, setpriority, getpriority |
| `fs`       | open, close, read, write, seek, stat, readdir, mkdir, unlink, rename, sync, dup, getcwd, chdir |
| `io`       | print, println                          |
| `mem`      | alloc_pages, free_pages, mmap, munmap   |
| `time`     | clock_gettime, uptime_ms, uptime_us     |
| `sys`      | sysinfo, syslog                         |
| `net`      | nic_info, nic_tx, nic_rx, nic_link_up, nic_mac, nic_refill |
| `hw`       | port I/O, PCI cfg, DMA, MAP_PHYS, IRQ, cache, CPUID, RDTSC, FB, boot_log, memmap |
| `persist`  | persist_put/get/del/list/info, pe_info  |
| `raw`      | syscall0..syscall5 (inline asm)         |
| `entry`    | `entry!()` macro, panic handler         |

### Quick Start
```rust
#![no_std]
#![no_main]
use libmorpheus::entry;
entry!(main);

fn main() -> i32 {
    libmorpheus::io::println("Hello from userspace!");

    // Hardware primitives (exokernel)
    let vendor = libmorpheus::hw::pci_cfg_read16(0, 0, 0, 0x00);

    // Process snapshot for a task manager
    let mut entries = [libmorpheus::process::PsEntry::zeroed(); 64];
    let count = libmorpheus::process::ps(&mut entries);

    // Raw NIC access for userland TCP/IP
    let mut mac = [0u8; 6];
    libmorpheus::net::nic_mac(&mut mac);

    0
}
```
