# MorpheusX

A UEFI "bootloader" (more like a minimal bare metall os or runtime) written in pure Rust enabling ephemeral operating systems with persistent userland state. Distro-hop daily while maintaining your development environment, configurations, and data across reboots.

## Overview

MorpheusX implements a bare-metal bootloader that decouples the OS layer from user data. Boot into Arch today, Fedora tomorrow, NixOS next week - your userland persists regardless of the underlying distribution. The bootloader handles kernel loading, filesystem operations, and state management entirely in UEFI space without external dependencies.

## Vision

Traditional systems tie user data to the OS installation. MorpheusX inverts this model by treating operating systems as ephemeral, interchangeable layers while preserving userland state across different distributions. This enables:

- Daily distribution switching without data migration
- Isolated testing environments with persistent development tools
- Rapid OS recovery by simply booting a different image
- Exploration of multiple distributions simultaneously

The architecture demonstrates low-level systems programming techniques including direct UEFI protocol manipulation, custom filesystem drivers, GPT partition management, and cross-architecture bootloader design.

## Architecture

```
morpheusx/
├── bootloader/     UEFI application entry point, EFI stub, kernel loading
├── core/           GPT operations, disk management, logging infrastructure  
├── network/        HTTP client for ISO downloads (UEFI protocol-based)
├── persistent/     State capture and restoration across boots
├── registry/       Configuration management
└── updater/        Self-update mechanisms
```

## Technical Details

- **Target**: x86_64-unknown-uefi (ARM64 support planned)
- **Language**: Rust (no_std, bare metal)
- **Dependencies**: Zero runtime dependencies, minimal build-time deps
- **Protocols**: Direct UEFI protocol bindings (no wrapper libraries)
- **Build**: LTO enabled, size-optimized for EFI system partition constraints

The bootloader implements custom parsers for GPT, FAT32, and PE/COFF formats. Network stack uses raw UEFI HTTP protocols. All disk I/O happens through EFI_BLOCK_IO_PROTOCOL without OS driver dependencies.

## Build

```bash
cargo build --release --target x86_64-unknown-uefi
```

Output: `target/x86_64-unknown-uefi/release/morpheus-bootloader.efi`

## Testing

QEMU with OVMF firmware:
```bash
cd testing && ./run.sh
```

## Status

Early development. Core bootloader and disk management functional. Network and persistence layers in progress.

*Dedicated to all the sysadmins who showed me the way <3.*
