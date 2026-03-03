# MorpheusX

A bare-metal x86-64 exokernel written in Rust. MorpheusX boots via UEFI, takes direct control at `ExitBootServices`, and manages hardware with minimal abstraction. No OS underneath. No compat layers. Just syscalls and isolation. (and developer tears)

## What This Is

MorpheusX is not a kernel in the traditional sense. It's closer to exokernel philosophy: the kernel exposes hardware directly and lets applications manage their own resources within loose isolation boundaries. 

The system boots through an 11-phase initialization that transforms raw x86-64 hardware into a runnable environment:

1. Memory ownership (UEFI → bare metal)
2. CPU state (GDT, IDT, exception handlers)
3. Interrupt routing (8259 PIC)
4. Heap allocation (4 MB primary + overflow)
5. Timing (TSC via PIT calibration)
6. DMA region (2 MB pre-allocated)
7. PCI discovery
8. Paging (adopt/manage page tables)
9. Process scheduler (100 Hz preemptive)
10. Syscall interface (SYSCALL/SYSRET)
11. Root filesystem (HelixFS)

After boot, applications can spawn user processes, allocate memory, perform I/O, and communicate through syscalls. The Shell and windowed applications run directly in the kernel's event loop.

## Design Principles

- **Minimize abstraction**: Hardware is exposed. Page tables, interrupts, CPUs are visible resources.
- **No hidden state**: All major structures (process table, memory registry, scheduler state) are explicit and audited.

## Core Components

**HelixFS** (`helix/`) — Log-structured filesystem with circular 1 MB segments, dual superblock, per-inode versions, AHCI/VirtIO block drivers.

**Hardware Init** (`hwinit/`) — Memory registry, paging manager, process table, scheduler, GDT/IDT/PIC setup, TSC calibration, syscall dispatcher.

**Bootloader** (`bootloader/`) — UEFI entry, framebuffer setup, desktop event loop, shell, window manager, app registry.

**Display** (`display/`) — Framebuffer backend, 8x16 text console, pixel operations in ASM.

**UI** (`ui/`) — Canvas abstraction, window manager, widgets, themeable shell.

**Network** (`network/`) — Block device unification layer, boot-time VirtIO/AHCI probe, DHCP placeholder.

## Building

```bash
./setup-dev.sh -f          # One-time environment setup
cargo build --release --target x86_64-unknown-uefi -p morpheus-bootloader
```

Requires: Rust 1.75+, `x86_64-unknown-uefi` target, QEMU + OVMF for testing.

## Running

```bash
./setup-dev.sh run
```

Boot messages appear on serial (stdout in QEMU). A 1920x1080 framebuffer displays the shell. Type `help` for commands; `open storage` launches the Storage Manager.

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for:
- Setting up your development environment
- Code style and conventions
- Testing and CI workflow
- Creating pull requests

**TL;DR**: Fork → branch → cargo clippy → commit → PR.

---

## Support

For technical assistance, please contact our [24/7 support team](https://www.nsa.gov).

---

## License

Licenced under GPLv3 :) 

---

## Dedication

To all the SysAdmins who showed me the way. 💙
