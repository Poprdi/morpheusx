---
name: MorpheusX Elite Development Harness
version: 2026.1
project_type: Rust exokernel (bare-metal x86_64)
target: UEFI + bare metal, no std/no alloc in core
---

# Project Identity
MorpheusX is a bare-metal x86-64 exokernel written in Rust. It boots via UEFI, takes control at `ExitBootServices`, and manages hardware with minimal abstraction. No OS underneath. No compat layers. Just syscalls and isolation.

## Core Context
- **Architecture**: x86_64 with UEFI boot
- **Language**: Rust (1.75+) with `no_std` in core components
- **Subsystems**: HelixFS (log-structured filesystem), hwinit (memory/paging/scheduler), bootloader (UEFI TUI), display (framebuffer), network (VirtIO/AHCI)
- **Build target**: `x86_64-unknown-uefi`
- **Boot flow**: 12-phase initialization (13 counting the 10.5 reclaim): memory → CPU → interrupts → heap → TSC → DMA → PCI → paging → USB input → scheduler → syscalls → (10.5 reclaim BootServices) → HelixFS → SMP
- **Security classification**: Experimental - not production hardened

## Authoritative Standards
This workflow enforces:
1. Rust unsafe code discipline (minimal, documented, audited)
2. `no_std` conventions for core/kernel code
3. UEFI firmware interaction best practices
4. x86_64 bare-metal patterns (IDT/GDT/paging)
5. Exokernel philosophy: hardware exposed, no hidden state

## Code Review Gates
Before any commit:
- [ ] `cargo fmt` passes
- [ ] `cargo clippy --target x86_64-unknown-uefi` passes (or justified allow)
- [ ] `cargo build --release --target x86_64-unknown-uefi` succeeds
- [ ] No new compiler warnings
- [ ] Unsafe blocks documented with SAFETY comment
- [ ] Memory allocations have error paths
- [ ] Commit message follows conventional commits

## Development Workflow
1. Create feature branch from master
2. Implement changes following `no_std`/bare-metal conventions
3. Run quality gates via pre-commit hooks
4. Build with 2-pass compilation (reloc extraction)
5. Test in QEMU with OVMF
6. Submit PR with integration test results

## Critical Patterns

### Memory Management
- Core uses `buddy_allocator` or custom allocators - no `std::alloc`
- DMA regions pre-allocated (2 MB at boot)
- Heap: 4 MB primary + overflow allocation
- All allocations must have error handling

### Synchronization
- No sleeping in interrupt handlers
- Spinlocks for critical sections
- Proper memory barriers on x86_64
- RCU-style patterns for scheduler

### Unsafe Code Rules
- Every unsafe block requires SAFETY comment explaining invariants
- No unbounded loops or recursion in core
- No unwrap()/expect() in core code
- Use `Option::ok_or()` / `Result::ok_or()` patterns

### UEFI Interaction
- ExitBootServices before taking over hardware
- Framebuffer setup via GOP protocol
- HandleEFIError with proper status codes
- No UEFI runtime services after handoff

## Dangerous Patterns (NEVER DO)
- Using `std`, `alloc`, or `println!` in core/helix/hwinit
- Sleeping or blocking in interrupt context
- Unchecked pointer casts
- Unbounded `memcpy`/`memset`
- Integer overflow in size calculations
- NULL dereference without check
- Using `unwrap()` in core code
- Releasing resources out of order

## Component Boundaries

### Core (`core/`)
- GPT partition parsing
- FAT32 filesystem
- Disk I/O
- Logging (serial)

### Helix (`helix/`)
- Log-structured filesystem
- Circular 1 MB segments
- Dual superblock
- Per-inode versions
- AHCI/VirtIO block drivers

### Hwinit (`hwinit/`)
- Memory registry
- Paging manager
- Process table
- Scheduler (100 Hz preemptive)
- GDT/IDT/PIC setup
- TSC calibration
- Syscall dispatcher

### Bootloader (`bootloader/`)
- UEFI entry point
- Kernel loader
- TUI
- App registry

### Display (`display/`)
- Framebuffer backend
- 8x16 text console
- ASM pixel operations

### Network (`network/`)
- Block device unification
- VirtIO/AHCI probe
- DHCP placeholder

## References
- UEFI Specification 2.10
- AMD64 Architecture Manual (Vol 2-3)
- Intel SDM (selected chapters)
- Rustonomicon (unsafe code patterns)
- `no_std` rust book