# iso9660-rs

[![Crates.io](https://img.shields.io/crates/v/iso9660-rs.svg)](https://crates.io/crates/iso9660-rs)
[![Documentation](https://docs.rs/iso9660-rs/badge.svg)](https://docs.rs/iso9660-rs)
[![License](https://img.shields.io/crates/l/iso9660-rs.svg)](LICENSE-MIT)

A pure `no_std` ISO9660 filesystem implementation in Rust with El Torito bootable CD support.

## Features

- **Pure `no_std`** - Works in bare metal, UEFI bootloaders, and embedded environments
- **ISO9660 Level 1/2/3** - Full ECMA-119 standard support
- **El Torito** - Bootable CD/DVD parsing for kernel extraction from live ISOs
- **Rock Ridge** - POSIX extensions for permissions and symlinks (optional feature)
- **Joliet** - Long Unicode filename support (optional feature)
- **Zero-copy parsing** - Efficient direct parsing from block devices
- **Minimal dependencies** - Only `gpt_disk_io` for block device abstraction

## Use Cases

- **UEFI bootloaders** that boot Linux from ISO files on ESP
- **Embedded systems** booting from CD-ROM or ISO images
- **Hypervisors/VMs** mounting ISOs for guest boot
- **Recovery tools** reading from optical media
- **ISO inspection** in `no_std` contexts

## Installation

```toml
[dependencies]
iso9660-rs = "1.0.1"
```

For optional extensions:
```toml
[dependencies]
iso9660-rs = { version = "1.0.1", features = ["rock-ridge", "joliet"] }
```

## Quick Start

```rust
use iso9660::{mount, find_file, read_file, find_boot_image};

// Mount ISO from block device
let volume = mount(&mut block_io, 0)?;

// Find and read a file
let file = find_file(&mut block_io, &volume, "/boot/vmlinuz")?;
let mut buffer = vec![0u8; file.size as usize];
read_file(&mut block_io, &file, &mut buffer)?;

// Extract bootable image via El Torito
let boot = find_boot_image(&mut block_io, &volume)?;
println!("Boot image at sector {}, {} bytes", boot.load_rba, boot.sector_count * 512);
```

## API Overview

### High-Level Functions

| Function | Purpose |
|----------|---------|
| `mount(block_io, start_sector)` → `VolumeInfo` | Parse volume descriptors and mount ISO |
| `find_file(block_io, volume, path)` → `FileEntry` | Navigate directory tree to locate file by path |
| `read_file(block_io, file, buffer)` → `usize` | Read file contents into provided buffer |
| `read_file_vec(block_io, file)` → `Vec<u8>` | Read entire file into heap-allocated vector |
| `find_boot_image(block_io, volume)` → `BootImage` | Extract El Torito bootable image entry |

### Advanced APIs

| Type | Purpose |
|------|---------|
| `FileReader<B>` | Buffered file reader with `seek()`, `read()`, `position()`, `is_eof()` |
| `DirectoryIterator<B>` | Manual directory traversal for sequential listing |
| `VolumeInfo` | Volume descriptor details (publisher, volume name, creation date) |
| `FileEntry` | File metadata (name, size, location, flags, datetime) |
| `FileFlags` | File attribute flags (is_directory, is_file, is_hidden, etc.) |
| `BootImage` | Boot catalog entry (load_rba, sector_count, platform, media_type) |
| `BootMediaType` | Boot media type enum (NoEmulation, Floppy, HardDisk, CDROM) |
| `BootPlatform` | Boot platform ID enum (x86, EFI, PowerPC, Mac) |
| `Iso9660Error` | Comprehensive error types with error context |
| `Result<T>` | Standard result type alias |

### Typical Usage Pattern

```rust
use iso9660::{mount, find_file, read_file, FileReader};

// 1. Mount ISO
let volume = mount(&mut block_io, 0)?;

// 2. Find file by path
let file = find_file(&mut block_io, &volume, "/boot/vmlinuz")?;

// 3. Option A: Read entire file into vector
let data = iso9660::read_file_vec(&mut block_io, &file)?;

// Option B: Stream read with buffer
let mut buf = [0u8; 4096];
let bytes_read = read_file(&mut block_io, &file, &mut buf)?;

// Option C: Use FileReader for advanced control
let mut reader = FileReader::new(&file);
reader.seek(512)?;  // Skip first sector
let pos = reader.position();
```

## Architecture

```
iso9660/
├── volume/        # Volume descriptor parsing (Primary, Supplementary, Boot)
├── directory/     # Directory record navigation and iteration
├── file/          # File reading from extents
├── boot/          # El Torito boot catalog parsing
├── extensions/    # Rock Ridge, Joliet (optional)
└── utils/         # Datetime, string conversion, checksums
```

## El Torito Boot Support

Extract bootable images from live ISO files - essential for UEFI bootloaders booting Tails, Ubuntu, etc.:

```rust
use iso9660::find_boot_image;

// Find boot image
let boot = find_boot_image(&mut block_io, &volume)?;

// Access boot image metadata
println!("Boot platform: {:?}", boot.platform);      // x86, EFI, PowerPC, Mac
println!("Media type: {:?}", boot.media_type);        // NoEmulation, Floppy, HardDisk, CDROM
println!("Boot location: sector {}", boot.load_rba);  // Sector number
println!("Boot size: {} bytes", boot.sector_count * 512);  // Size in 512-byte sectors
```

## Spec Compliance

Based on **ECMA-119** (ISO 9660:1988) and **El Torito** (1995) specifications.

### Supported
- ✅ Primary Volume Descriptor
- ✅ Directory tree navigation
- ✅ Both-endian field handling
- ✅ File version stripping (`;1`)
- ✅ El Torito validation + initial entry
- ✅ 7-byte and 17-byte datetime formats

### Optional (feature flags)
- 🔧 Rock Ridge POSIX extensions (`rock-ridge`)
- 🔧 Joliet Unicode filenames (`joliet`)

## Minimum Supported Rust Version

Rust 1.70 or later.

## License

Licensed under either of:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

## Contributing

Contributions welcome! This crate aims to be a reliable foundation for low-level systems work. Please keep changes focused and include tests where possible.
