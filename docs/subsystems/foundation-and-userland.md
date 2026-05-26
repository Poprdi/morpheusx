# Foundation + userland ABI

## Current architecture status

Phase 3.7 complete: 12-crate workspace, kernel fully arch-agnostic, the
previous `hwinit/` crate deleted. The foundation+libmorpheus boundary
as documented below is the live ABI. `morpheus-foundation` and
`morpheus-hal-api` are independent leaf crates; the kernel
(`morpheus-kernel`) imports both. The ELF loader (`elf.rs`) lives in
`morpheus-kernel/src/`.

## Purpose

`morpheus-foundation` is the shared FFI ABI between the MorpheusX kernel and
userland. It owns the syscall number table, the `#[repr(C)]` types that
travel through `syscall` register pairs (`FileStat`, `DirEntry`), and the
cross-crate error vocabulary (`NetworkError`). Zero workspace dependencies —
every other crate can pull it in without cycle risk. Arch-specific code (asm,
MSR reads, serial UART) lives in `morpheus-hal-x86_64`.

`libmorpheus` is the userspace consumer. It exposes the syscall instruction
behind type-safe wrappers (`File`, `TcpStream`, `Mutex`, `JoinHandle`, ...),
re-exports the `SYS_*` numbers from foundation, and ships the `entry!` macro
that emits `_start`. Every userspace binary (`init`, `msh`, `compd`,
`shelld`, `settings`, `ping`) consumes libmorpheus and nothing kernel-side.
That boundary — userspace touches the kernel only through foundation-defined
syscall numbers — is the load-bearing invariant of the subsystem.

## Why foundation?

Pre-refactor, the syscall ABI was duplicated by hand: `FileStat` and
`DirEntry` existed in both `helix/src/types.rs` (kernel side, written by the
syscall handlers) and `libmorpheus/src/fs.rs` (userspace side, read back
from the buffer), each with a `// layout must match the other side` comment.
The 99 `SYS_*` constants lived in the kernel's syscall module and
libmorpheus re-typed every number; a reorder would have silently
corrupted the dispatch.

This was sin #9 in the architecture audit. Phase 1.7 made
`morpheus-foundation` the single source of truth: helix re-exports the
canonical types, the kernel pulls `SYS_*` from
`morpheus_foundation::syscall_abi`, libmorpheus re-exports both via
`pub use morpheus_foundation::{types, syscall_abi}::*`. With one
definition shared by all consumers, a compile error replaces silent
corruption.

## Crate inventory

### morpheus-foundation

Leaf crate, `no_std`, three public modules.

| Module          | Contents                                                                                      |
|-----------------|-----------------------------------------------------------------------------------------------|
| `types`         | `FileStat`, `DirEntry` — `#[repr(C)]` with `pub` fields per the kernel-FFI ABI policy.        |
| `syscall_abi`   | 99 `SYS_*` constants plus `SEEK_SET / SEEK_CUR / SEEK_END`. Forever-stable allocation table.  |
| `error`         | `NetworkError` enum + `pub type Result<T> = core::result::Result<T, NetworkError>`.           |

`FileStat`: `key`, `size`, `is_dir`, `created_ns`, `modified_ns`,
`version_count`, `lsn`, `first_lsn`, `flags`. `DirEntry`: `name[256]`,
`name_len`, `is_dir`, `size`, `modified_ns`, `version_count`. Layouts are the
contract; the kernel writes these structures directly into user buffers.

`NetworkError` carries 25 variants spanning DNS, TCP, TLS, device, and buffer
faults; the `Display` impl turns them into human-readable strings without
leaking the variant names. It moved here from `morpheus-net-stack` in
Phase 3.1 Wave 4 so the kernel TCP/IP stack, the NIC drivers, and the future
userspace bindings can speak one error vocabulary.

A `MAX_CPUS` constant is planned (per locked decision 21, value 64) but has
not landed; today `libmorpheus::sys::SYSINFO_MAX_CPUS = 16` is the userland
ceiling on per-core counters.

### morpheus-hal-api

Object-safe trait surface implemented per-arch and consumed by the kernel.
Detailed in its own deep-dive doc; from foundation's perspective the relevant
facts are:

- Zero workspace deps. All HAL-side ABI types (`MemoryType`, `DmaRegion`,
  `PageFlags`, `BusAddr`, `MsiError`, ...) live here as plain `#[repr]`
  structures.
- Root trait `Hal` exposes sub-traits via accessor methods: `Mmio`, `Cpu`,
  `Serial`, `PhysAlloc`, `PageTable`, `InterruptController`, `Timer`,
  `DmaAllocator`, `BusEnumerator`, `UsbHost`, `Reset`, `Smp`.
- All sub-traits are `&dyn`-callable — no generics, no `Self` returns, no
  `impl Iterator`. Callback/slice patterns only.
- Kernel never depends on `morpheus-hal-x86_64`; only `morpheus-hal-api`. The
  concrete impl is installed at boot by the bootloader.

### libmorpheus

Userspace syscall library. `no_std`, depends only on `morpheus-foundation`.
Pulls in `alloc` and registers its own buddy allocator (`buddy.rs`). Layered
as: `raw` (inline-asm `syscall` thunks) → bare-`Result<T, u64>` wrappers →
RAII / `core::io::*`-style types.

| Module        | Lines | Summary |
|---------------|-------|---------|
| `raw`         | 141   | `syscall0..syscall5` inline-asm thunks; re-exports `morpheus_foundation::syscall_abi::*`; defines `FUTEX_WAIT/WAKE` and `SYSCTL_*` mode codes that are userland-internal flag spaces. |
| `fs`          | 528   | `open`/`close`/`read`/`write`/`seek`/`stat`/`readdir`/`mkdir`/`unlink`/`rename`/`sync`/`dup`/`getcwd`/`chdir` thin wrappers, plus `File`, `OpenOptions`, `ReadDir`, and the `Metadata`/`DirEntry` newtypes over foundation. |
| `io`          | 563   | `print`/`println`/`read_stdin`/`read_line`, the `Read`/`Write`/`Seek`/`BufRead` traits, `BufReader`/`BufWriter`. `println!`/`eprintln!`/`print!` macros live here. |
| `net`         | 1102  | NIC info/TX/RX/link/MAC/refill primitives, `SYS_NET` subcmd table for TCP socket/connect/send/recv/close/state/listen/accept/shutdown/nodelay/keepalive, `SYS_DNS` start/result, `SYS_NET_CFG` get/dhcp/static/hostname/activate, NIC ctrl subcmds (promisc, MAC, stats, MTU, VLAN, csum, TSO, ring sizing, IRQ coalescing). RAII `TcpStream`/`TcpListener`/`UdpSocket`. |
| `process`     | 377   | `exit`/`getpid`/`getppid`/`yield_cpu`/`kill`/`sleep`/`wait`/`try_wait`/`spawn`/`spawn_with_args`/`ps`/`sigaction`/`sigreturn`/`setpriority`/`getpriority`/`pipe`/`dup2`/`set_foreground`/`getargs`/`parse_args`. `PsEntry` (`#[repr(C)]`) + `signal::*` module. |
| `time`        | 240   | `clock_gettime` (TSC ns), `uptime_ms/us`, `Duration` (nanos-backed), `Instant`. |
| `mem`         | 71    | `alloc_pages`/`free_pages`/`mmap`/`munmap`/`shm_grant`/`mprotect`. `PROT_*` constants. |
| `hw`          | 356   | The exokernel escape hatch — `port_in/out` (b/w/l), `pci_cfg_read/write` (8/16/32), `dma_alloc/free`, `map_phys`/`map_mmio`/`virt_to_phys`, `irq_attach/ack`, `cache_flush`, `cpuid`, `rdtsc`, `fb_info`/`fb_map`/`fb_present`/`fb_blit`/`fb_lock`/`fb_unlock`/`fb_mark_dirty`. `CpuidResult`, `TscResult`, `FbInfo` FFI structs live here. |
| `sys`         | 134   | `sysinfo` + `SysInfo` `#[repr(C)]` (16-core per-core idle TSC array), `syslog`, `system_control`/`reboot`/`shutdown`/`shutdown_panic`. |
| `task`        | 447   | Futex-driven async executor — workers pull from a shared queue, park on per-task notify words via `SYS_FUTEX`. Standard `core::task` vtable so any `Future` works. |
| `thread`      | 181   | `spawn`/`JoinHandle::join`. Threads share CR3; kernel allocates a 64 KiB stack via `SYS_MMAP` and treats threads as lightweight processes. |
| `sync`        | 488   | `Mutex`, `Condvar`, `OnceLock`, `RwLock`, mpsc channel — all built on `SYS_FUTEX` with the 3-state lock protocol to avoid thundering-herd wakes. |
| `persist`     | 177   | `put`/`get`/`delete`/`list`/`info` against the HelixFS-backed KV store. `PersistInfo`, `BinaryInfo` (ELF/PE introspection) FFI structs. |
| `compositor`  | 94    | `compositor_set`, `surface_list`, `surface_map`, `mouse_forward`, `forward_input`, `surface_dirty_clear`. `SurfaceEntry` `#[repr(C)]` mirroring the kernel handler. |
| `desktop`     | 154   | `DesktopAppearance` profile (dark mode, accent RGB, panel/start/title/border colors) persisted under `de_appearance_v1`; shared by settings, shelld, compd. |
| `env`         | 71    | `args()`/`args_vec()` over `SYS_GETARGS`, `current_dir`/`set_current_dir`. |
| `error`       | 117   | `ErrorKind` enum (NotFound, BadFd, BrokenPipe, ...), `Error` newtype with `from_raw(u64)` mapping of high-u64 kernel errnos, `Result<T> = core::result::Result<T, Error>`. |
| `entry`       | 31    | `entry!($main)` macro — emits `_start` that calls `$main`, exits with its return code; ships the `panic_handler` that writes to fd 2 and exits 101. |
| `buddy`       | 304   | Userspace heap allocator. Registers itself as `#[global_allocator]` so `alloc::*` works. |

Re-export discipline: `lib.rs` exposes the 9 named kernel errnos
(`EINVAL`/`ENOMEM`/`ENOENT`/`EBADF`/`EPIPE`/`EFAULT`/`ESRCH`/`EIO`/`ENOSYS`)
plus the helper `is_error(ret: u64) -> bool` (`ret > 0xFFFF_FFFF_FFFF_FF00`).
The high-`u64` errno encoding is the userland-facing ABI; kernel returns are
the same `u64` values verbatim.

## Syscall ABI table

Cited from `morpheus_foundation::syscall_abi`. 99 stable allocations across
nine functional groups; subcmd-multiplexed entries (`SYS_NET`, `SYS_DNS`,
`SYS_NET_CFG`, `SYS_SYSTEM_CONTROL`) have their subcmd table documented in
the per-module summary above.

### Core (0–9)

| #  | Name           | Args (a1..a5)                                     | Return                                  |
|----|----------------|---------------------------------------------------|-----------------------------------------|
| 0  | `SYS_EXIT`     | `code`                                            | (does not return)                       |
| 1  | `SYS_WRITE`    | `fd`, `buf`, `len`                                | bytes written / errno                   |
| 2  | `SYS_READ`     | `fd`, `buf`, `len`                                | bytes read / errno                      |
| 3  | `SYS_YIELD`    | —                                                 | 0                                       |
| 4  | `SYS_ALLOC`    | `pages`                                           | phys base / errno                       |
| 5  | `SYS_FREE`     | `phys_base`, `pages`                              | 0 / errno                               |
| 6  | `SYS_GETPID`   | —                                                 | pid                                     |
| 7  | `SYS_KILL`     | `pid`, `signal`                                   | 0 / errno                               |
| 8  | `SYS_WAIT`     | `pid`                                             | exit code / errno                       |
| 9  | `SYS_SLEEP`    | `millis`                                          | 0                                       |

### HelixFS (10–21)

| #  | Name              | Args                                            | Return                  |
|----|-------------------|-------------------------------------------------|-------------------------|
| 10 | `SYS_OPEN`        | `path_ptr`, `path_len`, `flags`                 | fd / errno              |
| 11 | `SYS_CLOSE`       | `fd`                                            | 0 / errno               |
| 12 | `SYS_SEEK`        | `fd`, `offset`, `whence`                        | new pos / errno         |
| 13 | `SYS_STAT`        | `path_ptr`, `path_len`, `out_filestat_ptr`      | 0 / errno               |
| 14 | `SYS_READDIR`     | `path_ptr`, `path_len`, `out_buf`               | entries written / errno |
| 15 | `SYS_MKDIR`       | `path_ptr`, `path_len`                          | 0 / errno               |
| 16 | `SYS_UNLINK`      | `path_ptr`, `path_len`                          | 0 / errno               |
| 17 | `SYS_RENAME`      | `old_ptr`, `old_len`, `new_ptr`, `new_len`      | 0 / errno               |
| 18 | `SYS_TRUNCATE`    | `path_ptr`, `path_len`, `new_size`              | 0 / errno               |
| 19 | `SYS_SYNC`        | —                                               | 0 / errno               |
| 20 | `SYS_SNAPSHOT`    | (TBD)                                           | snapshot id / errno     |
| 21 | `SYS_VERSIONS`    | (TBD)                                           | (TBD)                   |

### System / process / memory (22–31)

| #  | Name             | Notes                                                                       |
|----|------------------|-----------------------------------------------------------------------------|
| 22 | `SYS_CLOCK`      | monotonic ns since boot (TSC)                                               |
| 23 | `SYS_SYSINFO`    | fills `SysInfo` (mem, procs, CPU count, uptime, per-core idle TSC[16])      |
| 24 | `SYS_GETPPID`    | parent pid                                                                  |
| 25 | `SYS_SPAWN`      | `path`, `argv_desc[]`, `argc` — returns child pid                           |
| 26 | `SYS_MMAP`       | kernel picks VA                                                             |
| 27 | `SYS_MUNMAP`     | by VA                                                                       |
| 28 | `SYS_DUP`        | duplicate fd                                                                |
| 29 | `SYS_SYSLOG`     | serial port write — bypasses console                                        |
| 30 | `SYS_GETCWD`     | writes path into buf                                                        |
| 31 | `SYS_CHDIR`      | sets process CWD                                                            |

### Networking — NIC layer (32–41)

| #  | Name              | Notes                                                                  |
|----|-------------------|------------------------------------------------------------------------|
| 32 | `SYS_NIC_INFO`    | fills `NicInfo { mac[8], link_up, present }`                           |
| 33 | `SYS_NIC_TX`      | raw Ethernet frame TX (caller builds full L2 header)                   |
| 34 | `SYS_NIC_RX`      | raw frame RX into buf                                                  |
| 35 | `SYS_NIC_LINK`    | link-up bool                                                           |
| 36 | `SYS_NIC_MAC`     | 6-byte MAC read                                                        |
| 37 | `SYS_NIC_REFILL`  | re-fill RX descriptor ring                                             |
| 38 | `SYS_NET`         | smoltcp TCP/UDP socket ops — subcmd in arg0                            |
| 39 | `SYS_DNS`         | start lookup / poll result / set servers                               |
| 40 | `SYS_NET_CFG`     | get / dhcp / static / hostname / activate; subcmd ≥128 → NIC `ctrl()`  |
| 41 | `SYS_NET_POLL`    | drive smoltcp poll loop (DHCP timers, ARP, TCP retransmits)            |

`SYS_NET` subcmds: 0 socket, 1 connect, 2 send, 3 recv, 4 close, 5 state,
6 listen, 7 accept, 8 shutdown, 9 nodelay, 10 keepalive. `SYS_DNS` subcmds:
0 start, 1 result, 2 set servers. `SYS_NET_CFG` ctrl-subcmds (128+):
PROMISC, MAC_SET, STATS, STATS_RESET, MTU, MULTICAST, VLAN, TX_CSUM, RX_CSUM,
TSO, RX_RING_SIZE, TX_RING_SIZE, IRQ_COALESCE, CAPS.

### Device / mount stubs (42–45)

| #  | Name              | Notes                                                                  |
|----|-------------------|------------------------------------------------------------------------|
| 42 | `SYS_IOCTL`       | reserved                                                               |
| 43 | `SYS_MOUNT`       | stub                                                                   |
| 44 | `SYS_UMOUNT`      | stub                                                                   |
| 45 | `SYS_POLL`        | reserved (libmorpheus async runtime uses futex instead)                |

### Persistence (46–51)

| #  | Name                | Notes                                                  |
|----|---------------------|--------------------------------------------------------|
| 46 | `SYS_PERSIST_PUT`   | key (≤255 B), data (≤4 MiB)                            |
| 47 | `SYS_PERSIST_GET`   | by key into buf                                        |
| 48 | `SYS_PERSIST_DEL`   | by key                                                 |
| 49 | `SYS_PERSIST_LIST`  | enumerate keys                                         |
| 50 | `SYS_PERSIST_INFO`  | fills `PersistInfo { backend_flags, num_keys, used }`  |
| 51 | `SYS_PE_INFO`       | fills `BinaryInfo { format, arch, entry, base, sz }`   |

### Raw hardware (52–62)

| #  | Name                  | Notes                                                |
|----|-----------------------|------------------------------------------------------|
| 52 | `SYS_PORT_IN`         | port, width(1/2/4) — **arch-specific**               |
| 53 | `SYS_PORT_OUT`        | port, width, value — **arch-specific**               |
| 54 | `SYS_PCI_CFG_READ`    | bdf, off, width — **arch-specific**                  |
| 55 | `SYS_PCI_CFG_WRITE`   | bdf, off, width, value — **arch-specific**           |
| 56 | `SYS_DMA_ALLOC`       | pages — physically contiguous, <4 GiB, zeroed        |
| 57 | `SYS_DMA_FREE`        | phys, pages                                          |
| 58 | `SYS_MAP_PHYS`        | phys, pages, flags(bit0=writable, bit1=uncacheable)  |
| 59 | `SYS_VIRT_TO_PHYS`    | translate VA → PA                                    |
| 60 | `SYS_IRQ_ATTACH`      | enable PIC line                                      |
| 61 | `SYS_IRQ_ACK`         | send PIC EOI                                         |
| 62 | `SYS_CACHE_FLUSH`     | CLFLUSH range — pre-DMA-read                         |

### Display + process control (63–68)

| #  | Name                  | Notes                              |
|----|-----------------------|------------------------------------|
| 63 | `SYS_FB_INFO`         | fills `FbInfo { base, w, h, ... }` |
| 64 | `SYS_FB_MAP`          | map back buffer; returns VA        |
| 65 | `SYS_PS`              | process table snapshot             |
| 66 | `SYS_SIGACTION`       | install / default(0) / ignore(1)   |
| 67 | `SYS_SETPRIORITY`     | pid (0=self), prio (0..=255)       |
| 68 | `SYS_GETPRIORITY`     | pid (0=self)                       |

### CPU diagnostics (69–72)

| #  | Name                  | Notes                                                   |
|----|-----------------------|---------------------------------------------------------|
| 69 | `SYS_CPUID`           | leaf, subleaf, out_buf — **arch-specific**              |
| 70 | `SYS_RDTSC`           | TSC + frequency (or raw if arg0=0)                      |
| 71 | `SYS_BOOT_LOG`        | retrieve boot-time serial log                           |
| 72 | `SYS_MEMMAP`          | export E820-style memory map                            |

### Memory protection (73–74)

| #  | Name             | Notes                                                  |
|----|------------------|--------------------------------------------------------|
| 73 | `SYS_SHM_GRANT`  | grant target pid a mapping over src_vaddr region       |
| 74 | `SYS_MPROTECT`   | vaddr, pages, PROT_WRITE|PROT_EXEC bits                |

### Shell / IPC (75–78)

| #  | Name           | Notes                                                          |
|----|----------------|----------------------------------------------------------------|
| 75 | `SYS_PIPE`     | returns `(read_fd, write_fd)`                                  |
| 76 | `SYS_DUP2`     | closes new_fd if open                                          |
| 77 | `SYS_SET_FG`   | set foreground pid (Ctrl+C → SIGINT)                           |
| 78 | `SYS_GETARGS`  | NUL-separated argv blob                                        |

### Sync + threads + signal return (79–83)

| #  | Name                  | Notes                                                  |
|----|-----------------------|--------------------------------------------------------|
| 79 | `SYS_FUTEX`           | addr, op (WAIT=0 / WAKE=1), value                      |
| 80 | `SYS_THREAD_CREATE`   | rip, rsp, arg → tid                                    |
| 81 | `SYS_THREAD_EXIT`     | exit_code                                              |
| 82 | `SYS_THREAD_JOIN`     | tid → exit_code                                        |
| 83 | `SYS_SIGRETURN`       | restore pre-signal context                             |

### Input + framebuffer control (84–90)

| #  | Name                | Notes                                            |
|----|---------------------|--------------------------------------------------|
| 84 | `SYS_MOUSE_READ`    | dx/dy/buttons packed                             |
| 85 | `SYS_FB_LOCK`       | exclusive present                                |
| 86 | `SYS_FB_UNLOCK`     | release                                          |
| 87 | `SYS_FB_IS_LOCKED`  | bool                                             |
| 88 | `SYS_FB_PRESENT`    | flip back→front                                  |
| 89 | `SYS_FB_BLIT`       | rect copy into front                             |
| 90 | `SYS_FB_MARK_DIRTY` | hint to compositor                               |

### Compositor + system control (91–98)

| #  | Name                            | Notes                                                |
|----|---------------------------------|------------------------------------------------------|
| 91 | `SYS_COMPOSITOR_SET`            | register caller as compositor (singleton)            |
| 92 | `SYS_WIN_SURFACE_LIST`          | enumerate `SurfaceEntry[]`                           |
| 93 | `SYS_WIN_SURFACE_MAP`           | map another pid's surface until that pid exits       |
| 94 | `SYS_MOUSE_FORWARD`             | route mouse delta to a pid                           |
| 95 | `SYS_WIN_SURFACE_DIRTY_CLEAR`   | compositor ack                                       |
| 96 | `SYS_TRY_WAIT`                  | non-blocking wait — returns EAGAIN if still alive    |
| 97 | `SYS_FORWARD_INPUT`             | push bytes into target's input ring                  |
| 98 | `SYS_SYSTEM_CONTROL`            | reboot graceful/force, shutdown graceful/force/panic |

Five syscalls are flagged **arch-specific** above: `SYS_CPUID`, `SYS_PORT_IN`,
`SYS_PORT_OUT`, `SYS_PCI_CFG_READ`, `SYS_PCI_CFG_WRITE`. On non-x86 HALs they
return `ENOSYS` rather than being silently unimplemented; this is locked
decision 25 (the exokernel escape hatch is architecture-honest).

## ABI stability guarantees

- All FFI structs are `#[repr(C)]` with `pub` fields. The field order +
  type sequence **is** the contract. There is no accessor-only view.
- Adding a field to `FileStat`, `DirEntry`, `PsEntry`, `SysInfo`,
  `SurfaceEntry`, `PersistInfo`, `BinaryInfo`, `NicInfo`, `NicHwStats`,
  `FbInfo`, `CpuidResult`, or `TscResult` is a breaking ABI change.
- Re-ordering fields is a breaking ABI change.
- Allocating new `SYS_*` numbers at the high end is safe — the dispatcher
  routes unknown numbers to `ENOSYS` and existing binaries keep working.
- Re-numbering an existing `SYS_*` corrupts every compiled binary in the
  filesystem. Don't.
- libmorpheus's `Metadata` is `#[repr(transparent)] struct
  Metadata(pub morpheus_foundation::types::FileStat)`. Layout is preserved
  by the transparent repr while accessor methods (`len()`, `is_dir()`,
  `key()`, `lsn()`, `first_lsn()`, `version_count()`, `flags()`,
  `created_ns()`, `modified_ns()`) provide the ergonomic surface. Same
  pattern for `DirEntry`.

## Static assertions / drift catches

The drift mitigation is structural rather than runtime-checked: both sides
import the same type from the same crate, so a layout change in foundation
that breaks one side breaks the other at compile time.

- `helix/src/types.rs:420` ends with `pub use
  morpheus_foundation::types::{DirEntry, FileStat};`. Helix's `Inode` build
  / `readdir` path materializes these structures directly.
- `libmorpheus/src/fs.rs:352` defines `pub struct
  Metadata(pub morpheus_foundation::types::FileStat)` with `#[repr(transparent)]`.
- `libmorpheus/src/raw.rs:7` does `pub use
  morpheus_foundation::syscall_abi::*;`.
Because helix, the kernel, and libmorpheus all bottom out at the same
foundation definitions, drift would require simultaneously editing one
crate to use a fork of the type — at which point the type identity
check at the `pub use` site fails. Sin #9 dies on first build.

## Userspace consumer survey

Workspace binaries that ship in the runtime image:

| Crate         | Dependencies (Cargo.toml)            | libmorpheus modules consumed                                             |
|---------------|--------------------------------------|---------------------------------------------------------------------------|
| `init`        | `libmorpheus`                        | `entry`, `io`, `process` (`spawn`, `sigaction`, `sigreturn`, `try_wait`) |
| `msh` (shell) | `libmorpheus`                        | `entry`, `env`, `process`, `compositor`, `io`, `fs`                       |
| `compd`       | `libmorpheus`, `channel`             | `entry`, `compositor`, `hw` (`fb_info`/`fb_map`), `process`, `desktop`, `time` |
| `shelld`      | `libmorpheus`                        | `entry`, `compositor`, `hw`, `io`, `process`                              |
| `settings`    | `libmorpheus`                        | `entry`, `hw`, `io`, `desktop`, `persist`                                 |
| `ping`        | (standalone `no_std` lib)            | none — ships its own ICMP packet builder                                  |

Library crates the binaries pull in:

| Crate             | Dependencies                                                       | Role                                                          |
|-------------------|--------------------------------------------------------------------|----------------------------------------------------------------|
| `channel`         | none                                                               | SPSC ring (`Channel<T, N>`) — compd's island IPC               |
| `morpheus-gfx3d`  | none                                                               | software 3D rasterizer used by `tests/spinning-cube`           |
| `morpheus-ui`     | none                                                               | TTY shell + windowed bits — opt-in framework                   |

Verified with `grep -rn "morpheus_foundation\|morpheus_hal\|morpheus_helix\|morpheus_kernel" {init,shell,settings,compd,shelld,ping,gfx3d,channel}/src/`:
zero matches. No userspace binary reaches across the syscall boundary into
a kernel crate. That's the Phase 1 audit invariant landing clean.

## Init process

`init` is PID 1 — the first userspace process the kernel jumps to after
mounting HelixFS. It is small (~57 LOC for the entry, plus an `islands`
supervisor module):

1. `io::println("init: starting MorpheusX Desktop Environment")`.
2. `process::spawn("/bin/compd")` — fail-fast log if missing.
3. `process::spawn("/bin/shelld")` — same.
4. `process::sigaction(SIGCHLD, sigchld_handler)` — installs a no-op handler
   that just calls `sigreturn`; the actual reap happens inside the supervisor
   tick via `SYS_TRY_WAIT`.
5. Loops on `islands::supervisor::tick(&mut state)` + `process::yield_cpu()`.

The supervisor is the reaper for zombie children. If compd or shelld dies it
respawns them (logic lives under `init/src/islands/supervisor.rs`). No
shell-from-init invocation — interactive login happens via shelld's launcher
spawning `/bin/msh` into a compositor surface.

## Channel IPC

`channel` is a 78-LOC, zero-dependency, `no_std`, `no_alloc` SPSC ring buffer:

```rust
pub struct Channel<T, const N: usize> { ... }
impl<T, const N: usize> Channel<T, N> {
    pub const fn new() -> Self;
    pub fn send(&self, msg: T) -> Result<(), T>;
    pub fn recv(&self) -> Option<T>;
}
```

`N` must be a power of two — enforced at compile time via `const _:
() = Self::ASSERT_POWER_OF_2`. Acquire/Release atomics on head/tail; wrapping
subtraction for fullness check. `T: Sync + Send` is asserted by the impl,
relying on the single-core scheduler invariant that producer and consumer
never run concurrently within the same process.

The message-type ABI is per-consumer; `Channel<T, N>` itself is generic.
compd's vocabulary lives in `compd/src/messages.rs` and covers
`VsyncMsg::Tick`, `SurfaceMsg::CompositeList`, `InputMsg::{FocusCycleRequest,
WindowClosed}`, `FocusMsg::FocusChanged`, `MouseSpatialMsg`. These are
internal to compd's island model; no other consumer of `channel` exists in
the workspace today, so the message ABI is currently single-process.

## Key invariants

- **All FFI types are `#[repr(C)]` with `pub` fields.** Layout is the contract.
- **Syscall numbers are never re-numbered.** Adding is safe; reordering is
  forbidden.
- **libmorpheus never imports from `morpheus_{helix, kernel,
  hal-x86_64, hal-api, net-stack, ...}`** — only `morpheus_foundation`.
  Verified by grep across the workspace.
- **Userspace binaries never import from kernel crates.** Verified the same way.
- **Newtype wrappers preserve layout.** `Metadata` and `DirEntry` in
  libmorpheus are `#[repr(transparent)]` over their foundation counterparts.
- **High-`u64` errno encoding is the userland-facing ABI.** Kernel returns
  `0xFFFF_FFFF_FFFF_FFXX` for errors; `is_error(ret)` is the sole check.

## Dependency surface

| Crate                  | Workspace deps                                                       |
|------------------------|-----------------------------------------------------------------------|
| `morpheus-foundation`  | none — true leaf                                                      |
| `morpheus-hal-api`     | none — leaf (own ABI types live here as plain repr structures)        |
| `libmorpheus`          | `morpheus-foundation` only                                            |
| `helix`                | `morpheus-foundation` (re-exports `FileStat`/`DirEntry`)              |
| `morpheus-kernel`      | `morpheus-foundation`, `morpheus-hal-api`, `morpheus-helix`, `morpheus-persistent` (arch-agnostic; **no** `morpheus-hal-x86_64`) |
| `morpheus-hal-x86_64`  | `morpheus-foundation`, `morpheus-hal-api`, `morpheus-x86-asm`, `morpheus-xhci` |
| `morpheus-xhci`        | `morpheus-foundation`, `morpheus-hal-api`, `morpheus-x86-asm`, `morpheus-kernel` |
| `morpheus-net-stack`   | `morpheus-foundation`, `morpheus-hal-x86_64`, NIC/block/virtio, smoltcp |
| Userspace binaries     | `libmorpheus` (+ `channel` for compd; no other workspace crates)      |

The dependency graph fans in to `morpheus-foundation` from every direction
that touches the syscall ABI, and from nowhere else. Foundation can be
rebuilt without rebuilding hal-api; hal-api can be rebuilt without rebuilding
foundation. The two leaves are independent.

## Known intentional changes vs pre-refactor

- **Phase 1.7:** 99 `SYS_*` constants moved from the previous
  `hwinit::syscall::*` to `morpheus_foundation::syscall_abi`. The
  kernel's `syscall` dispatcher (now in `morpheus-kernel/src/syscall/`)
  pulls them from foundation. libmorpheus re-exports them via `raw::*`
  for source compatibility.
- **Phase 1.7:** `FileStat` + `DirEntry` definitions moved from
  `helix::types` + `libmorpheus::fs` (where they were hand-mirrored) to
  `morpheus_foundation::types`. Helix re-exports; libmorpheus wraps in
  newtypes.
- **Phase 1.7.c:** libmorpheus's `Metadata` changed from a `#[repr(C)] struct
  Metadata { pub size: u64, pub is_dir: bool, ... }` to
  `#[repr(transparent)] struct Metadata(pub FileStat)`. Accessor methods
  (`len()`, `is_dir()`, `key()`, ...) preserve the userland API surface;
  this is locked decision 27 (newtype-over-foundation pattern for all
  ergonomic wrappers).
- **Phase 1.7.c follow-up:** userspace consumers `msh` and `ping` were
  rewired from `m.field` to `m.field()` accessor calls.
- **Phase 3.1 Wave 4:** `NetworkError` (and its `Result<T>` alias) moved from
  `morpheus-net-stack` to `morpheus-foundation::error`. Kernel and the
  future userspace TCP/UDP bindings now share one error vocabulary; previous
  callers in `morpheus-block` and friends point at the foundation copy via
  a `pub use morpheus_foundation::error::*`.
- **Pending:** `MAX_CPUS = 64` const (locked decision 21) has not landed.
  Today `libmorpheus::sys::SYSINFO_MAX_CPUS = 16` ceilings the SysInfo
  per-core idle TSC array.

## Cross-references

- **HAL trait surface doc** (`docs/phase3-prep/hal-api-design.md` + the
  separate deep-dive currently in flight) — the kernel side of the
  boundary. `morpheus-hal-api` owns all HAL-side `#[repr]` types
  (`MemoryType`, `DmaRegion`, `PageFlags`, ...).
- **Storage stack doc** — `helix::types` re-exports
  `morpheus_foundation::types::{FileStat, DirEntry}` so `SYS_STAT` and
  `SYS_READDIR` handlers write the canonical layout straight into user
  buffers.
- **Network stack doc** — `morpheus-net-stack` and `morpheus-nic` re-export
  `morpheus_foundation::error::*` for the unified `NetworkError`.
- **Bootloader doc** — the bootloader does not import foundation directly,
  but the eventual handoff target (`init` PID 1) is a userspace binary that
  consumes nothing but libmorpheus, and through it, foundation. The syscall
  ABI is the bootloader's eventual contract.
