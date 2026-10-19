# MorpheusX syscall handler audit (read-only), 2026-10-19

Scope: every handler reachable from `syscall_dispatch` (morpheus-kernel/src/syscall/mod.rs:72-250),
plus the kernel paths they call (pipe.rs, io/readiness.rs, schedular/{wait,wake,tick,lifecycle,spawn}.rs,
storage/{mod,fs_api}.rs, sync.rs, HAL idt.rs). Paths below are relative to `morpheus-kernel/src/`
unless stated. No files edited, no builds, no git mutation.

Caveat: the working tree was being modified under me (git status: pipe.rs, schedular/{lifecycle,spawn,
state,tick}.rs, syscall/handler/{proc,socket}.rs dirty). Line numbers for those files are from the
state I read; expect drift of a few lines. All other files were clean.

Explicitly skipped (orchestrator already on them): (1) the exit/terminate -> pipe_close_* -> set_ready ->
PROCESS_TABLE_LOCK recursion family; (2) `do_tcp_recv` EIO-before-EOF.

---

## P0 — memory-unsafety / isolation / lock cycles

### P0-1  validate_user_buf proves canonical range only; every handler then derefs raw. Kernel #PF = machine crash.
- `common.rs:19-27` checks `[ptr, ptr+len) <= 0x8000_0000_0000`, nothing about *mapped*. The only mapping check
  anywhere in the syscall layer is `sys_thread_create` (sync.rs:148-153, `pml4_translate` on the stack page).
- Every handler then does `slice::from_raw_parts[_mut]` / `ptr::read` / `ptr::write` / `*ptr` on the user address
  (e.g. core.rs:42,57,87,101,164; fd.rs:27,49; ipc.rs:188,256,278; proc.rs:107,142,188,378; fs.rs:164,205,284,333,
  517,569,609,703,716; epoll.rs:205,300; socket.rs:196,199,226,241,243,788,815,872,877,910,919; net.rs:292,320,452;
  nic_io.rs:143,150,169-172,212; persist.rs:113,187,282,353,488; hw.rs:76; sysinfo.rs:25,114,134,159,181;
  compositor.rs:68,193; fb.rs:64; clock.rs:29,45; sync.rs:27,67; mem.rs none).
- A CPL0 fault is NOT recoverable: `morpheus-hal-x86_64/src/cpu/idt.rs:463` computes `is_user_mode = cs&3==3`;
  the PID-terminate branch (`:667-684`) is taken only for user-mode faults; a kernel-mode fault falls through to
  `CRASH_HOOK` (BSOD) and halt/reset (`:686-700`).
- Scenario: any unprivileged process does `read(0, 0x1000, 1)` (canonical, unmapped) -> `stdin::read(buf)` writes
  -> #PF in ring 0 -> whole machine BSODs. Same with `getcwd(0x7fff_0000_0000, 64)`, `clock_gettime(0, 0x10)`, etc.
  Worse variants fault while holding a lock: `sys_futex` reads `*addr` under PROCESS_TABLE_LOCK (sync.rs:64-67);
  `replay_file_actions` reads the action array under PROCESS_TABLE_LOCK (proc.rs:185-188); `sys_win_surface_list`
  / `sys_forward_input` touch user memory under PROCESS_TABLE_LOCK (compositor.rs:72-98, 195-214); `epoll::scan`
  writes user memory under EPOLL_TABLE (epoll.rs:277-305); `sys_fs_read/write`, `persist_*` hand the user slice
  to the backend under STORAGE_LOCK (fs.rs:165-171, 207-220).
- Fix sketch (smallest): add `user_range_mapped(ptr,len) -> bool` next to validate_user_buf: for each 4K page in
  the range, `hal().paging().pml4_translate(Pml4Handle(current cr3), page)` must be Some (and user-accessible;
  for writes also writable — if the HAL exposes flags). Call it from validate_user_buf (or a `validate_user_rw`
  variant) so all ~90 call sites inherit it. Longer term: exception-table `copy_from_user`/`copy_to_user` with a
  #PF fixup, which also closes the TOCTOU against a sibling thread's concurrent munmap.

### P0-2  sys_fs_open stores user `flags` verbatim; O_PIPE_READ/O_PIPE_WRITE bits (0x100/0x200) let a process read/write/close ANOTHER process's pipe.
- fs.rs:21 `let flags = flags as u32;` -> fs.rs:51 `state.flags = flags;` with `kind = Regular`, `mount_id = <real mount id>`.
  Same in spawn file-action open: proc.rs:329 `state.flags = oflags`.
- Dispatch on the *flag bits*, not `kind`: sys_read core.rs:85 (`desc.flags & O_PIPE_READ`), sys_write core.rs:40,
  sys_fs_close fs.rs:93-107, child_fd_close proc.rs:221, terminate_process_inner lifecycle.rs (pipe fd sweep),
  fd0_available nic_io.rs:127, sys_dup2 ipc.rs:225-230. `is_socket()` is safe (fs_api.rs:109 checks `kind`).
- Scenario: `fd = open("/anything", O_READ | 0x100)` -> `read(fd, buf, n)` -> `pipe_read(pipe[mount_id])`
  (mount_id of the root mount is a small integer, a valid pipe index 0..15) -> steals bytes from a pipe owned by
  another process (e.g. a parent's stdout capture). `open(.., O_WRITE|0x200)` + `write` injects into it.
  `close(fd)` -> `pipe_close_reader(mount_id)` decrements a foreign refcount -> the victim's writer gets EPIPE /
  the pipe is freed while in use. The real file handle opened by `m.fs.open` leaks and `open_fds` is never decremented.
  No kernel memory unsafety (pipe idx bounds-checked via `get_mut`), but a cross-process isolation break.
- Fix sketch: in sys_fs_open and child_fd_open mask `flags &= !(O_PIPE_READ|O_PIPE_WRITE|O_SOCKET)` before storing;
  better, make every pipe dispatch test `desc.kind == FdKind::Pipe` (sys_pipe already sets kind) and drop the flag test.

### P0-3  Lock-order cycles: none new found. Observed partial order is consistent (see map below).
Only the known recursion family (skipped) exists. Stale comment: fs_api.rs:137 says the OFD slab lock is "never
nested under PROCESS_TABLE_LOCK", but replay_file_actions -> child_fd_close/child_fd_dup2 -> fd_table.free/set
-> ofd::decref/incref (proc.rs:223,241,275) and spawn_user_process -> inherit_fds_minus_cloexec -> dup -> ofd::incref
(spawn.rs:192-291) do nest it. No inverse (OFD TABLE is a leaf; SpinLock disables IRQs) so it is not a deadlock,
but the invariant text is wrong.

---

## P1 — errno / semantics lies that std will trip on

### P1-1  write() to a full pipe returns 0 -> std `WriteZero` (core.rs:43-48, pipe.rs Pipe::write)
`pipe_write` returns the partial count, 0 when the 4095-byte ring is full; no blocking, no EAGAIN. Scenario: child
writes 8 KiB to stdout (a pipe) before the parent drains: first write 4095, second returns 0 -> `write_all` errors
"failed to write whole buffer". Fix: if `n == 0 && len > 0`: O_NONBLOCK on the OFD -> `EAGAIN`; else park with
`readiness::wait_ready(pipe_token(idx), EPOLLOUT|EPOLLERR, 0)` and retry; return EPIPE if readers hit 0 meanwhile.

### P1-2  dup / F_DUPFD / F_DUPFD_CLOEXEC of a pipe or socket fd do not refcount the backend; closing either copy kills both.
- `sys_dup` fd.rs:10-17 and `sys_fcntl` F_DUPFD* fd.rs:127-136 -> `FdTable::dup_from` (fs_api.rs:496-515) copies the
  FdState (pipe index in mount_id, socket cookie) and only increfs the OFD. Only `sys_dup2` bumps pipe refcounts
  (ipc.rs:224-230); nothing refcounts a socket's smoltcp handle.
- `sys_fs_close` on either fd: pipe -> `pipe_close_reader/writer` (fs.rs:100-105) -> peer sees EOF/EPIPE while an fd is
  still open; socket -> `socket_close_backend` (fs.rs:86; socket.rs:982-994) closes the smoltcp handle and
  `unregister`s the readiness token -> the surviving fd's next op runs on a dead/reused handle.
- Scenario: `let s2 = stream.try_clone()?; drop(s2);` (std uses F_DUPFD_CLOEXEC) -> `stream` is now closed. Same for
  `ChildStdout::try_clone`.
- Fix sketch: in `dup_from`/`dup2` (or the three handlers) call `pipe_add_reader/writer` for `FdKind::Pipe`; for
  sockets, `ensure_ofd` at `sys_socket`/`finish_accept` and have `FdTable::free` report "last OFD ref dropped"; only
  then run `socket_close_backend`/`pipe_close_*`.

### P1-3  SYS_BIND on a UDP socket is a lie: the port is cached, never bound in the stack.
- socket.rs:330-347 stores local_port and returns 0. `NetStackOps` (net.rs:28-63) has no `udp_bind`; the stack has
  one (`morpheus-net-stack/src/stack/interface.rs:513`) that is never exposed. `udp_send_to` auto-binds an ephemeral
  port (interface.rs:528-536) and `udp_recv_from` on an unbound socket returns `Ok((0, 0.0.0.0, 0))` (interface.rs:549-562).
- Scenario: `UdpSocket::bind("0.0.0.0:5353")?.recv_from(&mut b)` never receives anything. Whether it parks forever
  (do_udp_recv treats rc<0 as would-block, socket.rs:732-746) or spuriously returns `Ok(0)` from 0.0.0.0:0 depends on
  how `net_udp_recv_from_impl` (bootloader glue, not read) maps that `(0,..)` — unverified which, both are wrong.
- Fix sketch: add `udp_bind: Option<unsafe fn(i64, u16) -> i64>` to NetStackOps + glue; call it from sys_bind for
  SOCK_DGRAM (map failure -> EADDRINUSE). `udp_recv_from` glue should return -1 (would-block) for Err, not 0.

### P1-4  SMP lost wakeup in blocking pipe/stdin reads (ipc.rs:298-316, core.rs:105-149)
Check-then-block without PROCESS_TABLE_LOCK: `pipe_read()==0` ... `mark_pipe_waiter; state = Blocked(PipeRead)`;
`wake_pipe_readers` (wake.rs:25-41) only flips tasks already marked; the tick sweep (tick.rs:300-345) re-checks only
Sleep/FutexWait/IoReady, never PipeRead/StdinRead/InputRead. Two cores: reader A sees empty; writer B writes and
wakes (no waiter yet); A marks + blocks -> stuck until the *next* write or last-writer close (exit sweep in
lifecycle.rs does wake PipeRead on writer==0). Request/response over a pipe pair between two live processes hangs.
Single-core is safe (writer is a syscall, cannot interleave). Fix: do the re-check + Blocked transition under
PROCESS_TABLE_LOCK (as `wait_ready` does, readiness.rs:264-284), or replace the loop with
`readiness::wait_ready(pipe_token(idx), EPOLLIN|EPOLLHUP, 0)` which already has the lost-wakeup discipline.

### P1-5  read(0) returns 0 (EOF) when a signal is pending (core.rs:120-122, 139-141)
POSIX/std expect `EINTR` (std retries `Interrupted`). Scenario: parent with SIGCHLD pending reads stdin -> gets EOF,
`read_line` returns Ok(0), REPLs exit. Fix: return `EINTR`.

### P1-6  write()/read() with len > 1 MiB -> EINVAL (core.rs:30,75; also sys_keyboard_read:158)
std `fs::write(path, 2 MiB)` / `File::write` of a big buffer gets `InvalidInput` instead of a short write.
Fix: `let len = len.min(1 << 20)` (short write/read), never EINVAL.

### P1-7  poll(): every fd >= 3 is reported POLLIN|POLLOUT unconditionally (nic_io.rs:230-240)
Pipes/sockets are "ready" when empty/full; only fd 0 is re-checked during the timeout loop (:255-260). If the PAL's
`Command::output` (read2) or anything else polls pipes it spins/returns wrong. Fix: resolve fd -> token via
`epoll::fd_token` and use `readiness::ready_mask`; regular files stay always-ready; park with
`wait_net_wake`/`wait_ready` instead of 10 ms `sys_sleep` chunks.

### P1-8  Path syscalls return EINVAL for bad pointer AND for len > 255 (common.rs:29-37 via resolve_user_path)
Affects open/stat/mkdir/unlink/rename/truncate/rmdir/readdir/versions/chdir/spawn/mount/umount (fs.rs:17,266,301,
345,366,389-395,431,487,786; fd.rs:57; proc.rs:380,395; persist.rs:90,155,225). std maps EINVAL -> InvalidInput;
a 300-char name should be `ENAMETOOLONG` (InvalidFilename), a bad pointer `EFAULT`. Fix: make `user_path` return
`Result<&str, u64>` with EFAULT / ENAMETOOLONG / EINVAL(utf8) and propagate.

---

## P2 — cosmetic / resource / minor errno

- sys_pipe: fd-table exhaustion -> ENOMEM (ipc.rs:160,173,185) should be EMFILE; the `pipe_alloc`'d slot leaks on
  that path (never `pipe_close_*`'d). Doc says "two u64" but writes `[u32;2]` (:188-190); validation (8 B) matches the code.
- sys_spawn: any `spawn_user_process` error -> ENOMEM (proc.rs:495), so a non-ELF file reports OutOfMemory (want ENOEXEC);
  `m.fs.open` failure -> ENOENT (:453) instead of mapped errno; `file_size > 4 MiB` -> EINVAL (:421) (want E2BIG/ENOMEM).
- sys_thread_create: spawn failure -> ENOMEM (sync.rs:158); POSIX pthread_create is EAGAIN.
- sys_fs_seek: accepts pipe/socket/epoll fds and returns an offset (fs.rs:229-263); should be ESPIPE.
- sys_fs_read via sys_read on an Epoll-kind fd falls into the storage path with mount_id 0 / empty path (fs.rs:157-171);
  should be EINVAL. (Socket and Null kinds are handled.)
- child_fd_close (proc.rs:206-243): Socket/Epoll/Null kinds take the storage path -> backend handle / epoll instance
  leak and a spurious `open_fds` decrement on mount 0 (if that id exists — unverified). Also the whole replay runs FS
  I/O under PROCESS_TABLE_LOCK (child_fd_open :321-337), stalling ticks on all cores.
- sys_getcwd: no ERANGE on truncation (fd.rs:48-51), returns the full length; depends on PAL retry logic.
- sys_getargs: bad pointer -> returns 0 (ipc.rs:253-255) instead of EFAULT.
- sys_setsockopt/getsockopt: bad pointers -> EINVAL (socket.rs:785-786, 869-874, 907-913) instead of EFAULT.
  `SO_RCVTIMEO`: `(tv.tv_sec as u64) * 1000` (socket.rs:816) overflows for negative/huge tv_sec — wraps in release
  (no overflow-checks in profile.release, Cargo.toml:64-67), panics in a dev build.
- read_sockaddr (socket.rs:196): materialises `&SockAddrStorage` (128 B) after validating only 16 B; only sa_family is
  read in practice. Use `*(addr as *const u16)`.
- sys_connect on a non-blocking socket returns EINPROGRESS on every call while SYN_SENT (socket.rs:533-534); Linux
  returns EALREADY after the first. std uses poll+SO_ERROR so harmless today.
- do_udp_recv: any bridge negative (including BRIDGE_ABSENT) is treated as would-block (socket.rs:730-746) -> a
  blocking recv on an absent op parks forever instead of ENOSYS.
- epoll: EPOLL_CTL_ADD with the 64-watch instance full -> ENOMEM (epoll.rs:230); Linux uses ENOSPC. A closed fd is not
  auto-removed from other instances' watch sets (`destroy_for` only drops the instance's own set, epoll.rs:126-137);
  if smoltcp reuses the handle the stale watch reports the new socket's readiness with the old `data`.
- sys_ioctl: unknown cmd -> EINVAL (nic_io.rs:176); should be ENOTTY. FIONBIO on fd 0 toggles the per-process
  `stdin_nonblock`, not the OFD O_NONBLOCK, so a dup2'd-pipe stdin ignores it (sys_read's pipe path tests
  `desc.flags & O_NONBLOCK`, core.rs:88).
- sys_map_phys: `phys + i*4096` can wrap for phys near 2^64 (hw.rs:204-205); partial-map failure leaves the already
  mapped PTEs (:206-212). Exokernel-by-design otherwise (any phys mappable).
- sys_virt_to_phys translates through the *kernel* PML4 (hw.rs:231) so user VAs mostly yield EINVAL — partial.
- sys_fb_map: two processes racing the first map double-allocate back/shadow (fb.rs:181-185, no lock).
- sys_sleep / sys_nanosleep / futex timeouts: TSC uncalibrated -> immediate return / block-forever (core.rs:227-230,
  clock.rs:58-60, sync.rs:32-34). Documented, boot-time only.
- sys_sysinfo / sys_fs_stat / sys_fs_fstat / sys_nic_info / sys_cpuid / sys_rdtsc / sys_pe_info / sys_persist_info /
  sys_fb_info write structs with `ptr::write` / `*p =` without an alignment check (fine on x86, UB by the book).
- sys_write/sys_syslog to fd 1/2: `stdout::push` + serial; if `len` is 1 MiB this holds the CPU for a long time with
  interrupts off (syscall context) — not a bug per se.

---

## Lock-order map (as observed in handler-reachable code)

```
PROCESS_TABLE_LOCK (IsrSafeRawSpinLock, IRQ-off)
  -> STORAGE_LOCK        proc.rs:185-241 (replay_file_actions -> child_fd_close/open), wait.rs:375 reap comment
  -> PIPE_LOCK           lifecycle.rs terminate sweep -> pipe_writers()/close_end(); proc.rs:225-230 (*_quiet)
  -> OFD TABLE (SpinLock) proc.rs:223,241,275 / spawn.rs inherit (fd_table.free/set/dup)
  -> ADDRESS_SPACE_LOCKS wait.rs reap (import at :2; not traced further)
STORAGE_LOCK            leaf w.r.t. the above (grep: no PTL/set_ready/wake under storage/, helix)
PIPE_LOCK               leaf; readiness publish happens after unlock (pipe.rs publish_readiness, *_quiet + DEFER_*)
EPOLL_TABLE (SpinLock)  leaf; ready_mask is lock-free
REG_LOCK (readiness)    leaf
net-state SpinLock      inside the bootloader glue; net_drive releases it before refresh_socket_readiness -> set_ready -> PTL
address-space locks     leaf (mem.rs, hw.rs, ipc.rs shm_grant ordered by leader pid)
```
No A->B / B->A pair found outside the known recursion family. `set_ready`/`wake_tokens` (readiness.rs:197-213)
take PTL; every handler-side caller (socket.rs:520,530,576,683,727,958,981,1054; pipe.rs; fs.rs:105; core.rs:47)
runs with no lock held. `net_wake()` from a future ISR takes only PTL (IRQ-safe lock) — OK.

## Prior-audit (2026-06) notes — status
- SYS_MOUNT -> ENOSYS: **NO LONGER TRUE.** fs.rs:624-658 calls `storage::mount` (storage/mod.rs:237) which does real
  direct + staged mounts (EEXIST/ENODEV/EBUSY mapped). SYS_UMOUNT (fs.rs:661-670 -> storage/mod.rs:526) is real too,
  with MNT_FORCE revoking fds via slab generation. Update the memory note.
- SYS_IOCTL only FIONREAD/FIONBIO/TIOCGWINSZ on fd 0..2: still true (nic_io.rs:138-178).
- NET_CFG GET zero-fills when `cfg_get` is None: still true (net.rs:451-454).
- Net family lazy-activated (ENODEV before ACTIVATE): still true — sys_net/sys_dns/sys_net_poll gate on
  `net_stack_present()` (net.rs:112,354,721); sys_net_cfg gates everything except GET/ACTIVATE (:471); sys_socket
  (socket.rs:250) returns ENODEV.

---

## Full table

Legend: V = user pointer/len validation; L = lock discipline; E = errno fidelity; class: REAL / PARTIAL / STUB.
"ptr-mapped" = inherits P0-1 (validated range, unchecked mapping). Files are syscall/handler/ unless noted.

| nr | name | where | class | verdict / note |
|---|---|---|---|---|
| 0 | EXIT | core.rs:23 | REAL | -> exit_process (known family) |
| 1 | WRITE | core.rs:29 | REAL | V ok (ptr-mapped); E: len>1MiB EINVAL (P1-6); pipe full -> 0 (P1-1); pipe dispatch on flag bits (P0-2); fd1/2 serial ok |
| 2 | READ | core.rs:74 | REAL | V ok (ptr-mapped); E: len>1MiB EINVAL (P1-6); signal-pending -> 0 not EINTR (P1-5); SMP lost wake (P1-4); flag-bit dispatch (P0-2); epoll fd -> storage (P2) |
| 3 | YIELD | core.rs:180 | REAL | halt_wait_irq; fine |
| 4 | ALLOC | mod.rs:255 | REAL | raw phys alloc, pages<=1024; exokernel by design |
| 5 | FREE | mod.rs:270 | REAL | frees ANY phys base (no ownership check) — exokernel by design, note |
| 6 | GETPID | core.rs:187 | REAL | leader pid; ok |
| 7 | KILL | core.rs:191 | REAL | E: every send_signal error -> ESRCH (incl. "already terminated") ok; L: known family |
| 8 | WAIT | core.rs:206 / wait.rs:270 | REAL | V ok (aligned, ptr-mapped); E: ECHILD/EINVAL(P_PGID)/0 WNOHANG ok; L: PTL only |
| 9 | SLEEP | core.rs:222 | REAL | TSC=0 -> immediate 0 (P2) |
| 10 | OPEN | fs.rs:16 | REAL | V: path EINVAL (P1-8); **flags unmasked (P0-2)**; fd slot not reserved by alloc() so no leak on early return (verified fs_api.rs:295) |
| 11 | CLOSE | fs.rs:77 | REAL | dispatch by flag bits for pipes (P0-2); dup'd socket/pipe close kills peer (P1-2); L: close_writer -> set_ready -> PTL, no lock held: ok |
| 12 | SEEK | fs.rs:229 | REAL | accepts pipe/socket/epoll fds (P2 ESPIPE) |
| 13 | STAT | fs.rs:265 | REAL | V ok (stat_buf validated, ptr-mapped); path EINVAL (P1-8) |
| 14 | READDIR | fs.rs:300 | REAL | V ok: validates only the written span, saturating_mul; path EINVAL (P1-8) |
| 15 | MKDIR | fs.rs:344 | REAL | path EINVAL (P1-8) |
| 16 | UNLINK | fs.rs:365 | REAL | EISDIR on dir, good; path EINVAL (P1-8) |
| 17 | RENAME | fs.rs:388 | REAL | EXDEV cross-mount; path EINVAL (P1-8) |
| 18 | TRUNCATE | fs.rs:430 | REAL | path EINVAL (P1-8) |
| 19 | SYNC | fs.rs:448 | REAL | all mounts; always 0 (sync errors swallowed — P2) |
| 20 | SNAPSHOT | fs.rs:461 | REAL | root mount only; name via user_path (EINVAL) |
| 21 | VERSIONS | fs.rs:484 | REAL | V ok (saturating_mul, validated span) |
| 22 | CLOCK | proc.rs:26 | REAL | monotonic ns; 0 pre-calibration |
| 23 | SYSINFO | proc.rs:45 | REAL | V ok (ptr-mapped, unaligned write) |
| 24 | GETPPID | proc.rs:83 | REAL | ok |
| 25 | SPAWN | proc.rs:371 | REAL | V: args aligned+validated; argv/envp pairs validated per entry (ptr-mapped); file_actions stride*count checked_mul + validated, read under PTL (P0-1 under lock); E: ENOMEM for any spawn error (P2), ENOENT on open error; L: PTL -> STORAGE_LOCK in replay (consistent); known family for close actions; `child_chdir` EINVAL on bad ptr |
| 26 | MMAP | mem.rs:45 | REAL | no user deref; MAP_FIXED overflow via checked_add; ok |
| 27 | MUNMAP | mem.rs:162 | REAL | ok; EINVAL on partial tile |
| 28 | DUP | fd.rs:10 | PARTIAL | **no pipe/socket backend refcount (P1-2)** |
| 29 | SYSLOG | fd.rs:20 | REAL | V ok |
| 30 | GETCWD | fd.rs:42 | REAL | V ok; no ERANGE (P2) |
| 31 | CHDIR | fd.rs:54 | REAL | path EINVAL (P1-8); storage guard dropped on early return (Drop impl verified storage/mod.rs:109) |
| 32 | NIC_INFO | nic_io.rs:11 | REAL | V ok |
| 33 | NIC_TX | nic_io.rs:37 | REAL | V ok; 14..=9000 |
| 34 | NIC_RX | nic_io.rs:57 | REAL | V ok; no upper bound on len (NIC copies min? unverified; ptr-mapped) |
| 35 | NIC_LINK | nic_io.rs:75 | REAL | ok |
| 36 | NIC_MAC | nic_io.rs:82 | REAL | V ok (6) |
| 37 | NIC_REFILL | nic_io.rs:95 | REAL | ok |
| 38 | NET | net.rs:111 | REAL (raw) | V ok on every buf (incl. UdpSendDesc/UdpRecvDesc nested ptr); E: blanket EIO for stack errors (raw API, acceptable); lazy ENODEV confirmed |
| 39 | DNS | net.rs:353 | REAL | V ok; name 1..253 |
| 40 | NET_CFG | net.rs:435 | REAL | GET zero-fill confirmed; ACTIVATE rc map; >=128 -> nic_ctrl raw |
| 41 | NET_POLL | net.rs:720 | REAL | V ok |
| 42 | IOCTL | nic_io.rs:138 | PARTIAL | only 3 cmds on fd 0..2; unknown -> EINVAL (want ENOTTY); FIONBIO vs pipe-stdin mismatch (P2); V ok (arg validated before each deref) |
| 43 | MOUNT | fs.rs:624 | REAL | **no longer ENOSYS**; mp via user_path (EINVAL); flags masked to RDONLY|STAGED |
| 44 | UMOUNT | fs.rs:661 | REAL | real; "/" -> EBUSY; MNT_FORCE honoured |
| 45 | POLL | nic_io.rs:195 | PARTIAL | V ok (nfds bounded before mul); **fd>=3 always ready (P1-7)**; busy 10 ms sleep loop |
| 46 | PERSIST_PUT | persist.rs:79 | REAL | V ok; key EINVAL; data slice used under STORAGE_LOCK (ptr-mapped) |
| 47 | PERSIST_GET | persist.rs:147 | REAL | V ok; buf_len==0 -> size |
| 48 | PERSIST_DEL | persist.rs:220 | REAL | ok |
| 49 | PERSIST_LIST | persist.rs:253 | REAL | V ok; missing dir -> 0 |
| 50 | PERSIST_INFO | persist.rs:310 | REAL | V ok |
| 51 | PE_INFO | persist.rs:358 | REAL | V ok; path EINVAL |
| 52 | PORT_IN | hw.rs:29 | REAL | exokernel raw; ENOSYS w/o hook |
| 53 | PORT_OUT | hw.rs:42 | REAL | same |
| 54 | PCI_CFG_READ | hw.rs:83 | REAL | raw; offset<=4095 |
| 55 | PCI_CFG_WRITE | hw.rs:100 | REAL | raw |
| 56 | DMA_ALLOC | hw.rs:127 | REAL | <4 GiB, zeroed via identity map |
| 57 | DMA_FREE | hw.rs:148 | REAL | frees any phys (exokernel) |
| 58 | MAP_PHYS | hw.rs:163 | REAL | any phys mappable (design); wrap + partial-unmap leak (P2) |
| 59 | VIRT_TO_PHYS | hw.rs:227 | PARTIAL | kernel-PML4 translation only (P2) |
| 60 | IRQ_ATTACH | hw.rs:238 | REAL | PIC unmask irq<=15 |
| 61 | IRQ_ACK | hw.rs:246 | REAL | PIC EOI |
| 62 | CACHE_FLUSH | hw.rs:255 | STUB (honest) | no-op on x86, documented |
| 63 | FB_INFO | fb.rs:57 | REAL | V ok |
| 64 | FB_MAP | fb.rs:159 | REAL | double-alloc race (P2) |
| 65 | PS | sysinfo.rs:11 | REAL | V ok (validates max_count*size, writes min(max,64)); ppid always 0 (documented) |
| 66 | SIGACTION | sysinfo.rs:55 | REAL | index bounded (from_u8 max 19 < 32) |
| 67 | SETPRIORITY | sysinfo.rs:72 | REAL | E: bad pid -> EINVAL (want ESRCH) P2 |
| 68 | GETPRIORITY | sysinfo.rs:87 | REAL | same P2 |
| 69 | CPUID | sysinfo.rs:101 | REAL | V ok; ENOSYS on all-zero |
| 70 | RDTSC | sysinfo.rs:127 | REAL | V ok (optional ptr) |
| 71 | BOOT_LOG | sysinfo.rs:149 | STUB (honest) | boot_log() is "" per comment; V ok |
| 72 | MEMMAP | sysinfo.rs:165 | REAL | V ok (saturating_mul) |
| 73 | SHM_GRANT | ipc.rs:18 | REAL | no user deref; two addr-space locks ordered by leader pid: ok |
| 74 | MPROTECT | mem.rs:221 | REAL | ok; ENOMEM when no VMA (POSIX) |
| 75 | PIPE | ipc.rs:142 | REAL | V ok (8 B); ENOMEM-for-EMFILE + pipe slot leak (P2) |
| 76 | DUP2 | ipc.rs:195 | REAL | bumps pipe refs (good), not socket refs (P1-2); L: sys_fs_close inside, no lock held: ok |
| 77 | SET_FG | ipc.rs:236 | REAL | no validation of pid (any value accepted) P2 |
| 78 | GETARGS | ipc.rs:243 | REAL | returns 0 on EFAULT (P2) |
| 79 | FUTEX | sync.rs:48 | REAL | V: addr aligned+validated but **deref under PTL** (P0-1 under lock); timeout ptr aligned+validated; E ok (EAGAIN/ETIMEDOUT) |
| 80 | THREAD_CREATE | sync.rs:116 | REAL | only handler that checks mapping (stack page); ctid validated; E: ENOMEM (want EAGAIN) P2 |
| 81 | THREAD_EXIT | sync.rs:163 | REAL | -> exit_process |
| 82 | THREAD_JOIN | sync.rs:167 / wait.rs:180 | REAL | E: ECHILD if already reaped, ESRCH otherwise; returns raw exit code (no -errno collision for codes >= 2^64-4095 — unverified, cosmetic) |
| 83 | SIGRETURN | sync.rs:172 | REAL | EINVAL outside handler |
| 84 | MOUSE_READ | sync.rs:185 | REAL | ok |
| 85 | FB_LOCK | fb.rs:110 | REAL | atomics, non-atomic check-then-store race (P2) |
| 86 | FB_UNLOCK | fb.rs:125 | REAL | ok |
| 87 | FB_IS_LOCKED | mod.rs:198 / fb.rs:140 | REAL | ok |
| 88 | FB_PRESENT | fb.rs:306 | REAL | ok |
| 89 | FB_BLIT | fb.rs:340 | REAL | ok |
| 90 | FB_MARK_DIRTY | mod.rs:201 | REAL | ok |
| 91 | COMPOSITOR_SET | compositor.rs:14 | REAL | EBUSY if other owner |
| 92 | WIN_SURFACE_LIST | compositor.rs:27 | REAL | V ok (saturating_mul) but **user write under PTL** (P0-1 under lock) |
| 93 | WIN_SURFACE_MAP | compositor.rs:103 | REAL | EPERM non-compositor; L: PTL released before sys_map_phys: ok |
| 94 | MOUSE_FORWARD | compositor.rs:129 | REAL | ok |
| 95 | WIN_SURFACE_DIRTY_CLEAR | compositor.rs:154 | REAL | ok |
| 96 | TRY_WAIT | compositor.rs:174 / wait.rs:229 | REAL | EAGAIN when not exited; ok |
| 97 | FORWARD_INPUT | compositor.rs:179 | REAL | V ok but **user read under PTL** (P0-1 under lock) |
| 98 | SYSTEM_CONTROL | core.rs:239 | REAL | EBUSY single-owner; graceful drain via send_signal (no lock held) |
| 99 | KEYBOARD_READ | core.rs:157 | REAL | V ok; len>1MiB EINVAL (P1-6 minor) |
| 100 | SET_THREAD_POINTER | proc.rs:34 | REAL | range check; ok |
| 101 | GETRANDOM | hw.rs:58 | REAL | V ok; ENOSYS w/o RDRAND |
| 102 | VOLUMES | fs.rs:528 | REAL | V ok |
| 103 | MOUNTS | fs.rs:587 | REAL | V ok |
| 104 | CLOCK_GETTIME | clock.rs:20 | REAL | V ok; EINVAL unknown clock |
| 105 | NANOSLEEP | clock.rs:37 | REAL | V ok; EINVAL bad spec; rem untouched (documented) |
| 106 | FSTAT | fs.rs:675 | REAL | V ok; synthesises mode for non-regular fds |
| 107 | THREAD_DETACH | sync.rs:208 | REAL | PTL only; ok |
| 108 | GETTID | sync.rs:235 | REAL | ok |
| 109 | SOCKET | socket.rs:249 | REAL | ENODEV pre-activation; EAFNOSUPPORT/EPROTONOSUPPORT ok; EMFILE ok |
| 110 | BIND | socket.rs:330 | **PARTIAL (lies for UDP)** | P1-3; TCP: port cached for listen (ok) |
| 111 | LISTEN | socket.rs:350 | REAL | EADDRINUSE on stack reject; unbound -> port 0 (P2) |
| 112 | ACCEPT | socket.rs:377 | REAL | V ok (write_sockaddr_in validates addrlen then addr); rcvtimeo -> EAGAIN ok; L: park_one_slice -> wait_ready -> PTL, no lock held |
| 113 | CONNECT | socket.rs:470 | REAL | V ok; ECONNREFUSED/EINPROGRESS/ETIMEDOUT; repeated EINPROGRESS (P2) |
| 114 | SENDTO | socket.rs:544 | REAL | V ok (len>0); ENOTCONN/EPIPE/EAGAIN ok; EMSGSIZE udp |
| 115 | RECVFROM | socket.rs:633 | REAL | V ok; TCP EOF path = known (2); UDP rc<0 == would-block (P2) |
| 116 | GETSOCKNAME | socket.rs:751 | REAL | V ok |
| 117 | GETPEERNAME | socket.rs:763 | REAL | ENOTCONN; V ok |
| 118 | SETSOCKOPT | socket.rs:778 | PARTIAL | REUSEADDR/BUF sizes accepted-and-ignored (documented); EINVAL-for-EFAULT; tv_sec overflow (P2) |
| 119 | GETSOCKOPT | socket.rs:862 | PARTIAL | SO_ERROR real (positive 111); KEEPALIVE/NODELAY always 0 (not tracked) P2 |
| 120 | SHUTDOWN | socket.rs:938 | REAL | ok |
| 121 | EPOLL_CREATE | epoll.rs:141 | REAL | EMFILE on exhaustion; ok |
| 122 | EPOLL_CTL | epoll.rs:176 | REAL | V ok (event validated for ADD/MOD); EPERM for regular files (Linux); ENOMEM-for-ENOSPC (P2) |
| 123 | EPOLL_WAIT | epoll.rs:344 | REAL | V ok (maxevents<=1024, saturating_mul); user write under EPOLL_TABLE lock (P0-1); lost-wakeup ordering reasoned correctly; timeout via tick IoReady sweep |
| 124 | GETENV | ipc.rs:265 | REAL | V ok |
| 125 | FSYNC | fs.rs:726 | REAL | non-regular -> 0 (honest-ish) |
| 126 | FTRUNCATE | fs.rs:752 | REAL | EINVAL non-writable (Linux) |
| 127 | FCNTL | fd.rs:94 | PARTIAL | F_DUPFD* no backend refcount (P1-2); F_SETFL only O_NONBLOCK (documented) |
| 128 | RMDIR | fs.rs:785 | REAL | ENOTDIR/ENOTEMPTY ok; path EINVAL (P1-8) |
| 129 | REPARENT | proc.rs:525 | REAL | PTL only; ok |
| other | unknown nr | mod.rs:246 | — | ENOSYS honest |

## Unverified (said so rather than guessed)
- Whether `net_udp_recv_from_impl` (bootloader glue) maps the stack's `Ok((0,0.0.0.0,0))` to 0 or -1 (decides
  hang vs spurious Ok(0) in P1-3).
- Whether mount id 0 exists (decides if child_fd_close's storage path on socket/epoll fds decrements a real
  mount's open_fds).
- Whether `inherit_fds_minus_cloexec` (spawn.rs:147) bumps pipe refcounts for inherited pipe ends (not read in full).
- NIC `rx` op bounds its copy to `buf_len` (sys_nic_rx passes len straight through).
