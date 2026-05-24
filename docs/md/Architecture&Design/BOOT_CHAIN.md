# MorpheusX Boot Chain

**Source of truth for the complete sequential boot order, from UEFI entry to userland.**

Every step is verified against the actual source. If something boots in a different order than this document says, the document is wrong — fix it here first.

---

## Overview

There is no separate second-stage bootloader. The UEFI binary IS the bootloader AND the kernel. `efi_main` → `enter_baremetal` → `platform_init_selfcontained` → `run_desktop` → PID 1 (`init`). One binary, one chain.

```
UEFI Firmware
  └─► efi_main                          bootloader/src/main.rs
        └─► enter_baremetal             bootloader/src/baremetal.rs
              └─► platform_init_selfcontained   hwinit/src/platform.rs
              └─► run_desktop           bootloader/src/tui/desktop.rs
                    └─► spawn_user_process("init")
                          └─► _start / main     init/src/main.rs  [PID 1]
                                └─► spawn /bin/compd
                                └─► spawn /bin/shelld
                                └─► supervisor loop
```

---

## Stage 1 — UEFI Entry

**File:** `bootloader/src/main.rs`  
**Function:** `efi_main(image_handle, system_table)`  
**Calling convention:** `"efiapi"` (Microsoft x64)

| # | Action | Detail |
|---|--------|--------|
| 1 | Serial proof-of-life | `log_info("UEFI", 900, "efi_main entry")` — first output on COM1 |
| 2 | Arm pre-EBS allocator | `uefi_allocator::set_boot_services(st.boot_services)` — stores `BootServices*` for `allocate_pool`/`free_pool` |
| 3 | Query GOP | `bs.locate_protocol(&GOP_GUID)` → read `GopMode` + `GopModeInfo` → extract `frame_buffer_base`, `size`, `width`, `height`, `pixels_per_scan_line`, `pixel_format` |
| 4 | Pack + hand off | Build `BaremetalEntryConfig { image_handle, system_table, framebuffer }` → `enter_baremetal(config)` — **never returns** |

**Panic handler:** serial dump → `bsod::show_panic_screen(file, line, col)` → `hlt` loop.

---

## Stage 2 — Baremetal Takeover

**File:** `bootloader/src/baremetal.rs`  
**Function:** `enter_baremetal(config: BaremetalEntryConfig) -> !`

This is the UEFI-to-baremetal border crossing. The function is split into two halves: everything before `exit_boot_services` (still UEFI-assisted) and everything after (we own the machine).

### Pre-EBS (UEFI still alive)

| # | Action | Detail |
|---|--------|--------|
| 1 | Serial log | `log_info("BOOT", 901, "enter baremetal")` |
| 2 | Store framebuffer | `FRAMEBUFFER_INFO = config.framebuffer` |
| 3 | Start live console | `start_live_console(&FRAMEBUFFER_INFO)` — mirrors all serial output to framebuffer in real-time |
| 4 | Scan ACPI config table | Walk `st.configuration_table` entries; match `ACPI_20_TABLE_GUID` first, fall back to `ACPI_10_TABLE_GUID`; store physical RSDP address |
| 5 | Read PE image base | `extern "C" { static __ImageBase: u8 }` — linker symbol, needed for buddy reservation post-EBS |
| 6 | Allocate kernel stack | `bs.allocate_pages(AllocateAnyPages, EfiLoaderData, 64 pages)` → 256 KB; `EfiLoaderData` survives EBS |
| 7 | Stage helix.img pre-EBS | `handle_protocol(LOADED_IMAGE_PROTOCOL_GUID)` → `handle_protocol(SIMPLE_FILE_SYSTEM_PROTOCOL_GUID)` → `open_volume` → `open("/morpheus/helix.img")` → seek EOF for size → rewind → `allocate_pages` → read loop (1 MB chunks, short-read tolerant) → store `PRE_EBS_HELIX_BASE/SIZE/SECTOR_SIZE` |
| 8 | Snapshot memory map | `bs.get_memory_map(...)` → fills `MMAP_BUF[65536]`; captures `map_key`, `desc_size`, `desc_ver` |

### The Point of No Return

| # | Action | Detail |
|---|--------|--------|
| 9 | **`exit_boot_services`** | `bs.exit_boot_services(image_handle, map_key)` — **UEFI IS DEAD** |
| 10 | **`cli`** | Immediately kill interrupts — UEFI's IDT pointed into `BootServicesCode` which EBS just freed; OVMF DEBUG scrubs freed pages with `0xAF`; one timer tick into that garbage = `#GP` |
| 11 | Set baremetal flag | `BAREMETAL_MODE.store(true, SeqCst)` |
| 12 | Switch allocator | `uefi_allocator::switch_to_post_ebs()` — transitions from UEFI pool to 4 MB static `.bss` primary heap |
| 13 | Pivot stack | `asm!("mov rsp, {stack_top}")` — switch onto our own `EfiLoaderData` stack |

### Post-EBS Init Sequence

| # | Action | Detail |
|---|--------|--------|
| 14 | Compute PE image size | `pe_image_size(image_base)` → `image_pages` for buddy reservation |
| 15 | **`platform_init_selfcontained`** | Full 13-phase hardware init — see Stage 3 |
| 16 | Register crash hook | `morpheus_hwinit::set_crash_hook(bsod_crash_hook)` |
| 17 | Network activation hook | `network::init_userspace_network_activation(dma_region, tsc_freq)` — passive hook; system stays offline by default |
| 18 | Persistent storage | `storage::init_persistent_storage(dma, tsc_freq)` — see Storage Init below |
| 19 | Init directories | `storage::create_init_directories()` — ensure `/bin`, `/etc`, `/tmp`, `/home`, `/var`, `/dev` exist |
| 20 | Validate framebuffer | Fatal halt if `fb_info.base == 0 \|\| fb_info.width == 0` |
| 21 | Register framebuffer | `morpheus_hwinit::register_framebuffer(FbInfo {...})` — wires `SYS_FB_INFO` / `SYS_FB_MAP` syscalls |
| 22 | Release parked APs | `morpheus_hwinit::cpu::ap_boot::release_parked_aps()` — APs were started in Phase 12 but parked; now they run |
| 23 | **`run_desktop`** | `tui::desktop::run_desktop(&display_info)` — **never returns** — see Stage 4 |

### Storage Init (`storage::init_persistent_storage`)

Fast path (pre-EBS image available):
1. `take_pre_ebs_helix_image()` — claim the staged buffer
2. `MemBlockDevice::new(base, size, sector_size)` → `RAM_HELIX_DEVICE`
3. `helix::vfs::global::replace_root_device(...)` — swap root FS device
4. `root_path_exists("/bin/init")` — validate image is usable
5. Set `PERSISTENT_READY = true`, return

Slow path (PCI probe):
1. Build `BlockDmaConfig` from DMA region offsets
2. `scan_all_block_devices()` — PCI scan for AHCI / VirtIO / SDHCI / USB-MSD
3. Fall back to RAM disk if no devices found
4. For each candidate: map BARs → `create_unified_from_detected` → `select_data_region` → `replace_root_device` → validate `/bin/init`

---

## Stage 3 — Hardware Init

**File:** `hwinit/src/platform.rs`  
**Function:** `platform_init_selfcontained(config: SelfContainedConfig) -> Result<PlatformInit, InitError>`

Called from `enter_baremetal` step 15. Returns `PlatformInit { tsc_freq, dma_region, allocator }`.

### Pre-Phase

| Action | Detail |
|--------|--------|
| `cli` | Belt-and-suspenders — IF=0 before touching memory |
| Clear `CR0.WP` | UEFI marks PT pages read-only; buddy's `list_push` writes `FreeNode` headers at physical addresses — `#PF` without this. Flush TLB after. |

### Phase 1 — Memory

`log_info("BOOT", 101, "phase 1/13: memory")`

| # | Action |
|---|--------|
| 1a | `paging::collect_page_table_pages()` — collect live PML4/PDPT/PD/PT pages |
| 1b | `sgdt` — collect GDT page(s) into `hw_holes` |
| 1c | `sidt` — collect IDT page(s) into `hw_holes` |
| 1d | Collect boot stack pages from `config.stack_base/stack_pages` (RSP-guess fallback) |
| 1e | Insertion-sort + dedup `hw_holes` |
| 1f | **`init_global_registry(..., &hw_holes[..hw_count])`** — import UEFI memory map into buddy allocator; `hw_holes` are punched out so buddy never writes `FreeNode` into live PT/GDT/IDT/stack pages |
| 1g | `global_registry_mut().validate_free_lists()` — catch corruption early |

### Phase 2 — CPU State

`log_info("BOOT", 102, "phase 2/13: cpu state")`

| # | Action |
|---|--------|
| 2a | `global_registry_mut().allocate_pages(AnyPages, LoaderData, 16 pages)` — 64 KB kernel stack |
| 2b | **`init_gdt(kernel_stack_top)`** — load GDT with TSS |
| 2c | **`init_idt()`** — install exception + IRQ handlers |
| 2d | **`enable_sse()`** — set `CR4.OSFXSR` + `CR4.OSXMMEXCPT` |
| 2e | `apic::probe_lapic_base()` — read MSR `0x1B`; firmware can relocate LAPIC |
| 2f | `apic::read_lapic_id()` — LAPIC MMIO identity-mapped by UEFI, safe pre-paging |
| 2g | **`per_cpu::init_bsp(bsp_lapic_id, actual_base)`** — initialize BSP per-CPU data; must happen after GDT, before scheduler/interrupt handlers that use `GS`-relative fields |

### Phase 3 — PIC

`log_info("BOOT", 103, "phase 3/13: pic")`

| # | Action |
|---|--------|
| 3a | **`init_pic()`** — remap PIC1 to vectors `0x20–0x27`, PIC2 to `0x28–0x2F` |

### Phase 4 — Heap

`log_info("BOOT", 104, "phase 4/13: heap")`

| # | Action |
|---|--------|
| 4a | **`init_heap(HEAP_SIZE)`** — 4 MB kernel heap backed by registry pages |

### Phase 5 — TSC

`log_info("BOOT", 105, "phase 5/13: tsc")`

| # | Action |
|---|--------|
| 5a | **`calibrate_tsc_pit()`** — calibrate TSC against PIT channel 2; returns `tsc_freq` in Hz |
| 5b | `CPUID.80000007H:EDX[8]` — check invariant TSC flag; warn only if absent |
| 5c | `scheduler::set_tsc_frequency(tsc_freq)` |

### Phase 6 — DMA

`log_info("BOOT", 106, "phase 6/13: dma")`

| # | Action |
|---|--------|
| 6a | `global_registry_mut().allocate_pages(MaxAddress(0xFFFF_FFFF), AllocatedDma, pages)` — 2 MB below 4 GB (VirtIO/AHCI require 32-bit physical addresses) |
| 6b | `write_bytes(dma_phys, 0, DMA_SIZE)` — zero the region; VirtIO checks `avail->idx` on enable |
| 6c | `DmaRegion::new(dma_phys, dma_phys, DMA_SIZE)` — identity-mapped: VA = PA = bus address |

### Phase 7 — PCI

`log_info("BOOT", 107, "phase 7/13: pci")`

| # | Action |
|---|--------|
| 7a | **`enable_all_pci_devices()`** — scan all bus/device/function; enable memory space + bus mastering on all non-bridge devices |

### Phase 8 — Paging

`log_info("BOOT", 108, "phase 8/13: paging")`

| # | Action |
|---|--------|
| 8a | **`init_kernel_page_table()`** — install kernel page table |
| 8b | **`apic::init_bsp()`** — map LAPIC MMIO as uncacheable (UC); fully enable BSP LAPIC hardware |

### Phase 9 — USB Input

`log_info("BOOT", 109, "phase 9/13: USB input init")`

| # | Action |
|---|--------|
| 9a | **`input::init()`** — initialize unified input subsystem |
| 9b | PCI scan (bus 0–255, dev 0–31): class `0x0C` / subclass `0x03` → `calibrate_tsc_pit()` → `XhciController::new(bar0, tsc_freq)` → `enumerate_and_bind_inputs(&mut controller)` |
| 9c | PS/2 remains fallback if no USB HID devices found |

### Phase 10 — Scheduler

`log_info("BOOT", 110, "phase 10/13: scheduler")`

| # | Action |
|---|--------|
| 10a | **`init_scheduler()`** |

### Phase 11 — Syscalls + Interrupts Live

`log_info("BOOT", 111, "phase 11/13: syscalls")`

| # | Action |
|---|--------|
| 11a | **`init_syscall()`** — configure `SYSCALL`/`SYSRET` MSRs (`STAR`, `LSTAR`, `SFMASK`) |
| 11b | `apic::disable_pic8259()` — mask PIC; all IRQs now flow through LAPIC |
| 11c | **`apic::setup_timer(100)`** — LAPIC timer at 100 Hz; calibrated against PIT channel 2 |
| 11d | `set_interrupt_handler(0x20, irq_timer_isr, 0, 0)` — wire timer ISR to IDT vector `0x20` |
| 11e | **`enable_interrupts()`** — `sti` — **the machine is now preemptive** |

### Phase 10.5 — Reclaim BootServices RAM

`log_info("BOOT", 111, "phase 10.5/13: reclaim boot services ram")`

Runs after `sti`. Non-preemptible — wrapped in `cli`/`sti`.

| # | Action |
|---|--------|
| 10.5a | `disable_interrupts()` |
| 10.5b | `paging::collect_page_table_pages()` + insertion sort |
| 10.5c | `global_registry_mut().reclaim_boot_services(&pt_pages[..pt_count])` — add `BootServicesCode`/`BootServicesData` pages to buddy, excluding live PT pages |
| 10.5d | `reg.validate_free_lists()` |
| 10.5e | `paging::reserve_page_table_pages()` — mark PT pages as reserved in the allocator |
| 10.5f | `reg.validate_free_lists()` — second pass; catches corruption from `carve_block` splits |
| 10.5g | `enable_interrupts()` |

### Phase 11 (FS) — HelixFS

`log_info("BOOT", 112, "phase 11/13: helixfs")`

| # | Action |
|---|--------|
| 11a | `global_registry_mut().allocate_pages(AnyPages, LoaderData, 4096 pages)` — 16 MB |
| 11b | `write_bytes(root_fs_base, 0, ROOT_FS_SIZE)` — zero the region |
| 11c | **`morpheus_helix::vfs::global::init_root_fs(root_fs_base, ROOT_FS_SIZE)`** — mount RAM-backed HelixFS at `/` |
| 11d | `pcpu.kernel_syscall_rsp = kernel_stack_top` — set initial syscall RSP for PID 0 |

### Phase 12 — SMP

`log_info("BOOT", 113, "phase 12/13: smp")`

| # | Action |
|---|--------|
| 12a | `acpi::set_rsdp_phys(config.acpi_rsdp_phys)` |
| 12b | `apic::read_lapic_id()` — get BSP LAPIC ID |
| 12c | `acpi::discover_ap_lapic_ids(bsp_lapic_id)` — scan MADT for enabled AP LAPIC IDs |
| 12d | `per_cpu::set_cpu_count(madt_result.count + 1)` |
| 12e | `disable_interrupts()` |
| 12f | **`ap_boot::start_aps_from_list(&ids[..count])`** — INIT-SIPI-SIPI sequence; APs park in a spin loop until `release_parked_aps()` in Stage 2 step 22 |
| 12g | `enable_interrupts()` |

**Returns:** `Ok(PlatformInit { tsc_freq, dma_region, allocator })`

---

## Stage 4 — Kernel Main Loop

**File:** `bootloader/src/tui/desktop.rs`  
**Function:** `run_desktop(_display_info: &FramebufferInfo) -> !`

| # | Action | Detail |
|---|--------|--------|
| 1 | `Keyboard::new()` | Initialize PS/2 keyboard decoder |
| 2 | `show_boot_log_screen(&mut keyboard)` | Print boot log to framebuffer; `puts("Press any key to launch msh...")` → `keyboard.wait_for_key()` → `clear_live_console_hook()` — framebuffer released from serial mirror |
| 3 | `Mouse::new()` | Initialize PS/2 mouse decoder |
| 4 | **`load_elf_from_fs("init")`** | `vfs_stat("/bin/init")` → `vfs_open` → `vfs_read` → `vfs_close` → return `Vec<u8>` |
| 5 | **`scheduler::spawn_user_process("init", &elf_data, &[], 0, false)`** | Load ELF into new address space; create PID 1 |
| 6 | `drop(elf_data)` | Init is in its own address space; kernel copy no longer needed |
| 7 | **Input loop forever** | Poll `asm_ps2_poll_any()` up to 64 bytes/iter: mouse bytes (`0x03xx`) → `mouse.feed` → `hwinit::mouse::accumulate(dx, dy, buttons)`; keyboard bytes (`0x01xx`) → `keyboard.feed_raw` → Ctrl+C: `SCHEDULER.send_signal(fg_pid, SIGINT)` or `stdin::push(ch)` + `wake_stdin_waiters()`; idle: `mark_kernel_hlt()` + `sti; hlt; cli` |

---

## Stage 5 — PID 1 (init)

**File:** `init/src/main.rs`  
**Entry:** `_start` generated by `entry!(main)` macro from `libmorpheus` (`entry.rs`)  
**Runtime:** Userspace — communicates with kernel exclusively via syscalls

| # | Action | Syscall |
|---|--------|---------|
| 1 | `io::println("init: starting MorpheusX Desktop Environment")` | `SYS_WRITE` |
| 2 | `SupervisorState::new()` | — |
| 3 | **`process::spawn("/bin/compd")`** | `SYS_SPAWN` → kernel loads ELF, creates compositor daemon |
| 4 | **`process::spawn("/bin/shelld")`** | `SYS_SPAWN` → kernel loads ELF, creates shell daemon |
| 5 | `process::sigaction(SIGCHLD, sigchld_handler)` | `SYS_SIGACTION` |
| 6 | **Supervisor loop forever** | `islands::supervisor::tick(&mut state)` + `process::yield_cpu()` |

**`sigchld_handler`:** calls `process::sigreturn()` only — no allocation in signal context (invariant B6). Actual reaping happens in `tick()`.

**`supervisor::tick(state)`:** For each tracked PID (`compd_pid`, `shelld_pid`): `process::try_wait(pid)` (`SYS_TRY_WAIT`). On exit: clear PID slot, increment restart counter. If `restarts < MAX_RESTARTS (5)`: re-spawn via `process::spawn()`. For `compd`: call `compsys::compositor_set()` before re-spawn to reclaim compositor slot (invariant D1). If `MAX_RESTARTS` exceeded: log fatal, give up on that service.

---

## Allocator State Machine

The global allocator transitions through three distinct states across the boot:

| State | Backing | Transition |
|-------|---------|------------|
| Pre-EBS | UEFI `allocate_pool` / `free_pool` via stored `BootServices*` | `set_boot_services()` in `efi_main` |
| Post-EBS primary | 4 MB static `.bss` buffer (`linked_list_allocator::Heap`) | `switch_to_post_ebs()` immediately after `exit_boot_services` |
| Post-EBS overflow | On-demand 16 MB chunks from `MemoryRegistry` | Automatic when primary heap exhausted |

---

## Interrupt Timeline

Interrupts are deliberately dead from `exit_boot_services` until Phase 11 (`sti`). The window is intentional — UEFI's IDT pointed into `BootServicesCode` which EBS freed. OVMF DEBUG scrubs freed pages with `0xAF`. One timer tick into that garbage corrupts buddy `FreeNode` chains and causes `#GP`.

```
exit_boot_services → cli
  Phase 1:  memory registry
  Phase 2:  GDT + IDT + SSE + per-CPU
  Phase 3:  PIC remap
  Phase 4:  heap
  Phase 5:  TSC
  Phase 6:  DMA
  Phase 7:  PCI
  Phase 8:  paging + LAPIC
  Phase 9:  USB
  Phase 10: scheduler
  Phase 11: syscalls + LAPIC timer
            sti  ← INTERRUPTS LIVE
  Phase 10.5: (cli) reclaim BootServices RAM (sti)
  Phase 11(FS): HelixFS
  Phase 12: SMP
```

---

## Key File Index

| File | Role |
|------|------|
| `bootloader/src/main.rs` | UEFI entry point (`efi_main`), GOP query, allocator arm |
| `bootloader/src/baremetal.rs` | Baremetal takeover, EBS, stack pivot, post-EBS init orchestration |
| `bootloader/src/uefi_allocator.rs` | Hybrid global allocator (UEFI pool → static heap → registry overflow) |
| `bootloader/src/storage.rs` | Block device probe (VirtIO/AHCI/RAM), HelixFS device swap |
| `bootloader/src/bsod.rs` | Panic/crash screen renderer (raw framebuffer, no alloc) |
| `bootloader/src/tui/desktop.rs` | Kernel main loop, ELF load, `spawn_user_process`, PS/2 input forwarding |
| `hwinit/src/platform.rs` | 13-phase hardware init orchestrator |
| `hwinit/src/cpu/gdt.rs` | GDT + TSS setup |
| `hwinit/src/cpu/idt.rs` | IDT, exception handlers, `enable_interrupts` / `disable_interrupts` |
| `hwinit/src/cpu/apic.rs` | LAPIC init, timer, PIC disable |
| `hwinit/src/cpu/ap_boot.rs` | INIT-SIPI-SIPI, AP parking, `release_parked_aps` |
| `hwinit/src/cpu/per_cpu.rs` | Per-CPU data, BSP init, GS-relative fields |
| `hwinit/src/cpu/tsc.rs` | TSC calibration against PIT |
| `hwinit/src/process/scheduler.rs` | Scheduler init, `spawn_user_process`, `mark_kernel_hlt` |
| `hwinit/src/memory.rs` | Buddy allocator, `MemoryRegistry`, `init_global_registry` |
| `hwinit/src/paging/` | Kernel page table, PT page collection/reservation |
| `hwinit/src/usb/` | xHCI controller, HID enumeration |
| `init/src/main.rs` | PID 1 — spawn `compd` + `shelld`, supervisor loop |
| `init/src/islands/supervisor.rs` | Child process supervisor, restart logic |
| `entry.rs` | Userspace C runtime — `entry!(main)` macro, `_start`, panic handler |

---

## Forensic Safety Audit (Bring-up / Runtime Initialization)

This section records **observed runtime hazards** from code-path tracing. It is intentionally adversarial and biased toward long-term breakage risk.

### Severity Legend

- **Critical corruption risk**: can corrupt memory/state or hard-lock under realistic timing.
- **Race condition**: concurrent execution can produce inconsistent state or UB.
- **Undefined behavior risk**: aliasing, unsound global mutation, lifetime violations.
- **Architectural debt**: design mismatch likely to fail as system complexity grows.
- **Layering violation**: subsystem boundaries are porous; ownership unclear.
- **Observability gap**: failures are hard to diagnose due to missing stage contracts/logs.

---

## Danger Map (Top Findings)

### 1) Unsynchronized network runtime globals (Critical / Race / UB)

**Files:**
- `bootloader/src/baremetal_ops/network/state.rs`
- `bootloader/src/baremetal_ops/network/activate.rs`
- `hwinit/src/syscall/handler/net.rs`

**Observed behavior:**
- Network activation and runtime socket tables are backed by mutable globals (`USER_NET_STACK`, `USER_TCP_HANDLES`, `USER_UDP_HANDLES`, `USER_DNS_QUERIES`, etc.) with **no lock/atomic protection**.
- Syscalls can run on multiple cores after AP release; net operations can race on handle slot allocation/mutation and stack/device state.

**Why this is dangerous:**
- Handle table corruption, duplicate handle assignment, stale or taken handle reuse.
- Concurrent mutable access to `NetInterface`/driver state through `&'static mut` returns.
- Works only while traffic/process pressure is low; fails under SMP contention.

---

### 2) VFS global singleton returns `&'static mut` from `static mut` (Critical / UB / Layering)

**Files:**
- `helix/src/vfs/global.rs`
- `hwinit/src/syscall/handler/common.rs`

**Observed behavior:**
- `fs_global_mut()` returns `Option<&'static mut FsGlobal>` from `static mut FS_GLOBAL`.
- A manual `VFS_LOCK` (`RawSpinLock`) is used in syscall handlers to serialize usage.
- `replace_root_device()` swaps global FS object wholesale.

**Why this is dangerous:**
- Soundness relies on every caller always respecting external lock discipline.
- `replace_root_device()` has no type-level guard preventing concurrent readers in future call paths.
- Singleton mutability is global and ambient; ownership is implicit and fragile.

---

### 3) Display/compositor global ownership model is atomic-only, not transactional (Race / Architectural debt)

**Files:**
- `hwinit/src/syscall/handler/fb.rs`
- `hwinit/src/syscall/handler/compositor.rs`

**Observed behavior:**
- `COMPOSITOR_PID`, `FB_LOCK_PID`, surface ownership and dirty flags are coordinated with loose atomics and process-table mutations.
- `release_fb_lock_if_holder()` iterates process table and sends kills based on compositor exit path assumptions.

**Why this is dangerous:**
- Ownership transition is not a single state machine.
- Possible stale/partial ownership under future async teardown paths.
- Hard to reason about lock ordering between compositor ops and scheduler/process teardown.

---

### 4) Syscall `SYS_VIRT_TO_PHYS` resolves via kernel page-table manager (Critical correctness bug)

**File:** `hwinit/src/syscall/handler/hw.rs`

**Observed behavior:**
- `sys_virt_to_phys()` uses `paging::kvirt_to_phys(virt)`.
- `kvirt_to_phys` translates through kernel page-table manager, not caller CR3.

**Why this is dangerous:**
- Result can be incorrect for user mappings and can break userspace physical mapping logic.
- Appears to function accidentally only when address identity/mappings overlap.

---

### 5) Boot/runtime responsibility split is inconsistent (Architectural debt / Layering)

**Files:**
- `bootloader/src/baremetal.rs`
- `hwinit/src/platform.rs`
- `bootloader/src/storage.rs`
- `bootloader/src/tui/desktop.rs`

**Observed behavior:**
- Platform init (hwinit) mounts RAM HelixFS.
- Bootloader storage may immediately replace root device.
- APs are started/parked in hwinit but released later by bootloader.
- Userspace transition (`/bin/init` spawn) is in bootloader TUI path.

**Why this is dangerous:**
- No single owner of boot-to-runtime boundary semantics.
- Hidden transitions happen across subsystem edges without explicit contracts.
- Refactor risk is high because sequence correctness is distributed.

---

### 6) CR0.WP policy duplicated and globally disabled early (Architectural debt / Security posture risk)

**Files:**
- `hwinit/src/platform.rs`
- `hwinit/src/paging/mod.rs`

**Observed behavior:**
- Write-protect is cleared in platform prelude and again in paging init.
- System operates with kernel write-protect effectively disabled.

**Why this is dangerous:**
- Duplicate responsibility = duplicate source of truth.
- Long-term hardening and debugging become harder (silent writes to read-only mappings).

---

### 7) Raw spinlocks rely on call-context assumptions (Race / Deadlock risk under growth)

**Files:**
- `hwinit/src/sync.rs`
- `hwinit/src/syscall/handler/common.rs`
- `hwinit/src/pipe.rs`
- `hwinit/src/shutdown/prepare.rs`

**Observed behavior:**
- `RawSpinLock` does not disable interrupts.
- Correctness currently depends on most callers running in syscall context with IF cleared.

**Why this is dangerous:**
- If reused from IRQ-enabled or mixed contexts, deadlock/reentrancy bugs appear.
- Contract is implicit, undocumented at API boundary.

---

### 8) AP parking/release is split-phase and externally controlled (Race window / Architectural debt)

**Files:**
- `hwinit/src/cpu/ap_boot.rs`
- `hwinit/src/platform.rs`
- `bootloader/src/baremetal.rs`

**Observed behavior:**
- APs boot and park during platform init.
- Release occurs later from bootloader after storage/framebuffer orchestration.

**Why this is dangerous:**
- Scheduling concurrency switch is controlled outside SMP owner module.
- Any future bootloader-stage additions post-release run under SMP without explicit audit.

---

### 9) Root device replacement mutates global FS identity mid-boot (Layering / Maintainability)

**Files:**
- `helix/src/vfs/global.rs`
- `bootloader/src/storage.rs`

**Observed behavior:**
- `replace_root_device()` reconstructs mount table and swaps global device/instance.
- Existing invariants depend on “no userspace yet” timing rather than API guarantees.

**Why this is dangerous:**
- Safe by timing, not by design.
- Future parallel bring-up or background services could invalidate assumptions.

---

### 10) Stage numbering/logging contracts drift from execution truth (Observability gap)

**File:** `hwinit/src/platform.rs`

**Observed behavior:**
- Phase labels/checkpoints are inconsistent (`phase 11` reused, syscall checkpoints labeled as phase10 in places).

**Why this is dangerous:**
- Debug traces become misleading during failure triage.
- Refactors can silently reorder steps without log-level detection.

---

## Global Mutable State & Ownership Map (High-Risk Set)

| State | Owner (de-facto) | Init point | Mutation sites | Sync model | Risk |
|---|---|---|---|---|---|
| `FS_GLOBAL`, `FS_INITIALIZED` | Helix VFS global | `init_root_fs*` | `replace_root_device`, VFS ops | External `VFS_LOCK` discipline | UB/lifetime/ownership ambiguity |
| `GLOBAL_REGISTRY` | hwinit memory | phase 1 | alloc/free/reclaim paths | `SpinLock` + IF save/restore | lock-order sensitive |
| `PROCESS_TABLE` | scheduler | `init_scheduler` | scheduler, syscalls, signal paths | `PROCESS_TABLE_LOCK` + ad-hoc direct access | race/alias risk |
| `COMPOSITOR_PID`, `FB_LOCK_PID` | syscall fb/compositor | runtime syscalls | compositor/fb syscalls, teardown | atomics only | split ownership model |
| `NET_STACK_OPS`, `NET_ACTIVATE_FN` | syscall net bridge | bootloader registration | register + runtime dispatch | unsynchronized `static mut` | race/partial state |
| `USER_NET_*` tables | bootloader net state | activation | runtime net ops | unsynchronized `static mut` | critical SMP race |
| `LIVE_PUTC` serial hook | bootloader↔hwinit logging | pre-EBS/live console start | set/clear hook, serial writes | unsafely mutable global + serial lock | transition coupling |
| `AP_SCHEDULER_RELEASED` | ap_boot | AP bring-up | bootloader release | atomic bool | cross-owner sequencing |
| `FB_BACK_PHYS`/shadow globals | syscall nic_fb/fb | first `SYS_FB_MAP` | present/blit/map paths | atomics + registry/paging side-effects | mixed ownership complexity |
| `BOOT_SERVICES` / allocator mode | bootloader allocator | pre-EBS | switch post-EBS | atomics + mutex heaps | transition-sensitive |

---

## Unsafe Ordering Dependencies (Currently Required)

1. `ExitBootServices` → `cli` must happen before any operation that can vector interrupts.
2. `switch_to_post_ebs()` must happen before any allocator use after EBS.
3. Global memory registry must be initialized before heap growth/overflow allocator paths.
4. `init_gdt` before `init_idt`, and both before `sti`.
5. Timer ISR vector must be installed before interrupts are enabled.
6. Scheduler init must precede user process spawn.
7. BootServices reclaim must happen only after CPU/interrupt/paging baseline is stable.
8. PT reservation must follow reclaim to avoid live PT reuse.
9. AP release must occur only after bootloader post-hwinit critical setup completes.
10. Root replacement must happen before first userspace VFS consumers.

Most of these are **implicit contracts**, not encoded in a stage interface.

---

## Hidden Coupling Map

- Bootloader ↔ hwinit logging via live serial framebuffer hook (`LIVE_PUTC`).
- Bootloader storage ↔ Helix global VFS singleton replacement semantics.
- Scheduler teardown ↔ compositor ownership cleanup (`release_fb_lock_if_holder`).
- Network syscall API ↔ bootloader runtime activation callback and mutable state tables.
- Desktop launch path ↔ existence of `/bin/init` in whichever root device happens to be mounted last.
- AP release timing ↔ bootloader orchestration decisions outside SMP subsystem.

---

## Implicit Invariants Required for Boot Success

- Single-writer assumptions on multiple mutable globals remain true by timing, not by type.
- No subsystem re-enters VFS without acquiring `VFS_LOCK`.
- No unexpected userspace/syscall net activity occurs during activation registration transition.
- `process::SCHEDULER.current_*_mut()` unsafely borrowed references are not aliased by concurrent cores in forbidden ways.
- `replace_root_device()` occurs before any long-lived user FD/table expectations.
- Timer ISR does not execute paths that would require locks held by interrupted contexts in the same core.

---

## Legacy / Stale / Contaminating Paths

- `platform_init` legacy entry exists but active chain uses `platform_init_selfcontained`.
- `parse_uefi_memory_map`, `init_heap_with_buffer`, `init_root_fs_raw` are present but not in active canonical boot path.
- `bootloader/src/tui/main_menu.rs` and related legacy TUI files are not part of runtime boot chain.
- `is_boot_disk` helper in storage is not active in current selection path.

These increase cognitive load and can reintroduce stale assumptions during refactor.

---

## Fragile-by-Design Zones

1. **Global singleton FS + mutable mount replacement**.
2. **Network state spread across bootloader and syscall bridge with no synchronization contract**.
3. **Display/compositor ownership distributed across fb/compositor/process teardown code**.
4. **Boot critical sequencing encoded in code order + comments, not in stage contracts**.
5. **Unsafe mutable process access patterns (`&'static mut` views) mixed with lock-based access paths**.

---

## Recommended Direction (Architecture, not implementation)

- Introduce a single boot stage controller with explicit preconditions/postconditions.
- Replace ambient mutable singletons with owned context objects passed between stages.
- Move all runtime net state behind one synchronized kernel-owned object.
- Make VFS global replacement an explicit transition API with quiesce/ownership contract.
- Consolidate display ownership into a single state machine with explicit transitions.
- Define and enforce lock-order policy (documented, audited, testable).
- Convert “works by timing” assumptions into enforceable barriers and invariants.
- Deprecate legacy init paths or mark as non-runtime test-only with hard guards.

---

## Refactor Risk Priority (for future work)

1. **P0**: Network mutable globals + SMP races.
2. **P0**: `SYS_VIRT_TO_PHYS` translation path correctness.
3. **P1**: VFS global singleton and root replacement ownership model.
4. **P1**: Compositor/framebuffer ownership split and teardown coupling.
5. **P1**: Boot sequence ownership split (`bootloader` vs `hwinit`) and AP release coupling.
6. **P2**: Logging/stage contract drift and observability blind spots.
7. **P2**: Legacy path contamination cleanup and source-of-truth consolidation.
