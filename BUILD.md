# MorpheusX Build & Dev Guide

## Prerequisites

```bash
# Arch
sudo pacman -S nasm qemu-full ovmf rust

# Add UEFI target
rustup target add x86_64-unknown-uefi
```

OVMF path: `/usr/share/OVMF/OVMF_CODE.fd` (verify exists or update `testing/run.sh`)

## Quick Build

```bash
/setup-dev.sh -f
```

Build does 2-pass compilation:
1. Initial build -> extract PE .reloc section
2. Rebuild with embedded reloc data (for unrelocating runtime image)

Output: `testing/esp/EFI/BOOT/BOOTX64.EFI`

## Build Internals

**Workspace structure** (workspace in root `Cargo.toml`):
- `bootloader/` - UEFI entry, kernel loader, TUI, installer
- `core/` - GPT, FAT32, disk I/O, logging
- `persistent/` - PE/COFF parsing, relocation reversal
- `updater/` - self-update logic
- `network/` - Bare metal networking (WIP)
**Build requirements**:
- `nasm` - assembles `trampoline32.asm` (bootloader/build.rs)
- `ar` - creates static lib from .obj
- Rust 1.75+ with `x86_64-unknown-uefi` target

**Profile**: `opt-level="z"`, `lto=true`, `panic="abort"`, stripped

## Testing Scripts (`testing/`)

**Distro installers**:
- `install-arch.sh` - downloads Arch bootstrap (~500MB), creates 2GB rootfs
- `install-tails.sh` - downloads Tails 7.2 ISO (1.3GB), extracts kernel/initrd/squashfs
- `install-live-distro.sh` - menu: Ubuntu/Debian/Tails/Fedora/Kali, extracts live system
- `quickstart-tails.sh` - one-shot: download Tails → build → run

**Initrd management**:
- `create-minimal-initrd.sh` - minimal test initrd with busybox
- `rebuild-initrd.sh` - packs `esp/rootfs/` into initramfs (for Arch)
- `setup-initrd.sh` - downloads Ubuntu netboot initrd

**Other**:
- `test-boot.exp` - expect script (automated testing)

## Relocation Tool (`tools/`)

**Why**: UEFI discards .reloc after applying fixups. Need original relocs to unrelocate image for persistence.

- `extract-reloc-data.sh` - parses PE header, extracts .reloc section, generates `persistent/src/pe/embedded_reloc_data.rs`
- `extract-image-base.sh` - gets ImageBase from PE optional header
- `analyze-relocs.sh` - debug helper

Called automatically by `build.sh` between passes.

## Debugging

```bash
./debug.sh          # GDB helper, connects to QEMU :1234
```

Start QEMU first with `./testing/run.sh`, then run debug.sh. Includes .gdbinit config.

QEMU runs with `-s` (gdbserver on :1234).

## ESP Layout

After install scripts:
```
testing/esp/
├── EFI/BOOT/BOOTX64.EFI     # morpheus bootloader
├── kernels/
│   ├── vmlinuz-arch
│   ├── vmlinuz-tails
│   └── vmlinuz-ubuntu
├── initrds/
│   ├── initrd-arch.img
│   ├── initrd-tails.img
│   ├── filesystem.squashfs  # Tails rootfs
│   └── initramfs-arch.img
└── rootfs/                 # Arch bootstrap (if installed)
```

## Common Issues

**Mount fails (install-tails.sh)**: needs sudo for loop mount
**nasm not found**: install nasm before building
**OVMF path wrong**: update path in `run.sh` line 79/94/108
**ESP too small**: run.sh calculates size from esp/ dir, adds 50MB overhead

## Kernel Boot Params

**Tails**: `boot=live live-media-path=/live nopersistence noprompt timezone=Etc/UTC splash=0 console=ttyS0,115200`
**Arch** (from initrd): `root=/dev/ram0 rw console=ttyS0,115200`
**Ubuntu**: `boot=casper quiet splash console=ttyS0,115200`

## Dev Notes

- No network in minimal Arch rootfs
- Live ISOs preferred for full userland (Tails recommended: 1.3GB, complete)
- Bootloader uses custom GPT/FAT32 (no std, no alloc in core)
- Kernel handoff via EFI stub protocol (not GRUB multiboot)
- Serial console on ttyS0 for debugging
- Workspace members build as libs, bootloader is bin target
- Reloc data hardcoded at compile time (see persistent/src/pe/)

## Clean Build

```bash
cargo clean
rm -f testing/test-disk*.img testing/esp.img
cd testing && ./build.sh
```

Deletes QEMU disks for fresh GPT/partition testing.

## One-Liner Test

```bash
cd testing && ./quickstart-tails.sh
```

Downloads Tails, builds, runs. ~10min on decent connection.

## Script Summary

| Script | Purpose |
|--------|---------|
| build.sh | 2-pass build with reloc extraction |
| install-arch.sh | Arch bootstrap → rootfs → initramfs |
| install-tails.sh | Download Tails ISO, extract kernel/initrd |
| install-live-distro.sh | Menu installer for 5 distros |
| quickstart-tails.sh | Automated Tails setup + build + run |
| create-minimal-initrd.sh | Busybox test initrd |
| rebuild-initrd.sh | Pack rootfs/ into initramfs |
| setup-initrd.sh | Download Ubuntu netboot initrd |
| create-test-disk.sh | Create 50GB GPT disk with ESP |
| run-persistence-test.sh | Boot only 10GB disk (persistence check) |
| extract-reloc-data.sh | Parse PE, extract .reloc, gen Rust code |
| debug.sh | Connect GDB to QEMU :1234 |

## Architecture Flow

1. OVMF firmware loads BOOTX64.EFI from ESP
2. Morpheus scans GPT, mounts FAT32 ESP
3. TUI presents kernel list from /kernels/
4. User selects kernel
5. Load kernel + initrd into memory
6. Setup EFI handoff protocol
7. Jump to kernel entry
8. Kernel boots, runs init from initrd
9. (Optional) Pivot to real root or stay in initramfs
