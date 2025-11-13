# Morpheus - Ephemeral Linux Bootloader

A next-generation UEFI bootloader that enables daily distro-hopping with persistent userland. Boot into a fresh OS every day while keeping your dev environment intact.

## Features

- **Ephemeral Root Filesystems** - Fresh OS on every boot, zero cruft accumulation
- **Persistent Userland** - Your home, configs, and data survive across distros
- **Network Integration** - Redeco L2 protocol for consistent networking
- **Multi-Architecture** - x86_64, aarch64, armv7 support
- **Auto-Updates** - Fetch latest distro images from mirrors
- **Zero Dependencies** - Pure Rust with minimal inline assembly
- **OverlayFS Magic** - Union mounts for maximum flexibility

## Architecture

```
[UEFI Firmware]
      ↓
[Morpheus Bootloader]
      ↓
[TUI Menu - Pick Your Distro]
      ↓
[Mount Templates + Persistent Layer]
      ↓
[Jump to Linux Kernel]
      ↓
[Your Chosen Distro Boots]
```

## Project Structure

- `bootloader/` - Main UEFI application
- `core/` - Low-level disk/fs/mount operations
- `persistent/` - Persistent layer management
- `redeco-integration/` - Network daemon integration
- `updater/` - Template update system
- `registry/` - Distro metadata and sources
- `network/` - HTTP client for downloads
- `installer/` - Initial system setup
- `cli/` - Runtime management tool
- `utils/` - Shared utilities

## Building

```bash
# Build for x86_64 UEFI
cargo build --release --target x86_64-unknown-uefi -p morpheus-bootloader

# Build for ARM64 UEFI
cargo build --release --target aarch64-unknown-uefi -p morpheus-bootloader
```

## Installation

See `docs/INSTALLATION.md`

*Dedicated to all the sysadmins who showed me the way.*
