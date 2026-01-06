# ISO9660 Implementation Architecture

## Overview

A pure `no_std` Rust implementation of the ISO9660 filesystem with El Torito bootable CD support. This crate is standalone and reusable across projects.

## Design Philosophy

- **Modular:** All files kept under 500 lines for maintainability
- **Pure no_std:** No external dependencies beyond `gpt_disk_io` for block device abstraction
- **Comprehensive:** Supports base ISO9660 + El Torito + optional Rock Ridge/Joliet
- **Well-documented:** Every module and public API has documentation
- **Type-safe:** Strong typing with proper error handling

## Module Structure (20 files)

```
iso9660/
├── Cargo.toml           - Package manifest with optional features
├── README.md            - Usage documentation
└── src/
    ├── lib.rs           - Main entry point, public API exports
    ├── error.rs         - Comprehensive error types (Iso9660Error)
    ├── types.rs         - Core types and constants
    │
    ├── volume/          - Volume descriptor parsing (sector 16+)
    │   ├── mod.rs       - Mount API, descriptor header
    │   ├── primary.rs   - Primary Volume Descriptor parsing
    │   ├── supplementary.rs - Joliet support (optional)
    │   └── boot_record.rs   - El Torito boot record
    │
    ├── directory/       - Directory navigation and iteration
    │   ├── mod.rs       - High-level find_file() API
    │   ├── record.rs    - DirectoryRecord struct (variable-length)
    │   ├── iterator.rs  - DirectoryIterator for sequential reading
    │   ├── path_table.rs - Optional fast path lookup
    │   └── flags.rs     - FileFlags parsing/manipulation
    │
    ├── file/            - File reading and extent management
    │   ├── mod.rs       - read_file() API
    │   ├── reader.rs    - Buffered FileReader
    │   ├── metadata.rs  - FileEntry methods (name, extension)
    │   └── extent.rs    - Extent type (contiguous data regions)
    │
    ├── boot/            - El Torito bootable CD support
    │   ├── mod.rs       - find_boot_image() API
    │   ├── catalog.rs   - Boot catalog parsing
    │   ├── entry.rs     - Boot entry structures
    │   ├── validation.rs - Validation entry + checksum
    │   └── platform.rs  - Platform ID constants (x86, EFI, etc.)
    │
    ├── extensions/      - Optional filesystem extensions
    │   ├── mod.rs       - Extension module exports
    │   └── rock_ridge.rs - POSIX metadata (permissions, symlinks)
    │
    └── utils/           - Shared utilities
        ├── mod.rs       - Utility exports
        ├── datetime.rs  - DateTime7 and DateTime17 parsing
        ├── string.rs    - ISO9660 string encoding handling
        ├── sector.rs    - Sector alignment calculations
        └── checksum.rs  - 16-bit checksum for El Torito

Total: 27 files (~4500 lines estimated when complete)
```

## Public API Surface

All of the following items are re-exported at the crate root and available to external dependencies via `iso9660::ItemName`.

### Functions (5)

```rust
pub fn mount<B: BlockIo>(block_io: &mut B, start_sector: u32) -> Result<VolumeInfo>
pub fn find_file<B: BlockIo>(block_io: &mut B, volume: &VolumeInfo, path: &str) -> Result<FileEntry>
pub fn read_file<B: BlockIo>(block_io: &mut B, file: &FileEntry, buffer: &mut [u8]) -> Result<usize>
pub fn read_file_vec<B: BlockIo>(block_io: &mut B, file: &FileEntry) -> Result<Vec<u8>>
pub fn find_boot_image<B: BlockIo>(block_io: &mut B, volume: &VolumeInfo) -> Result<BootImage>
```

### Advanced Types (8)

```rust
pub struct FileReader<B: BlockIo> { ... }   // Buffered streaming I/O
pub struct DirectoryIterator<B: BlockIo> { ... }   // Sequential directory listing
pub struct VolumeInfo { ... }               // Volume metadata
pub struct FileEntry { ... }                // File metadata
pub struct FileFlags { ... }                // File attribute bitflags
pub struct BootImage { ... }                // Boot catalog entry
pub enum BootMediaType { ... }              // Boot media type
pub enum BootPlatform { ... }               // Boot platform ID
```

### Error & Result

```rust
pub enum Iso9660Error { ... }   // 20+ error variants
pub type Result<T> = std::result::Result<T, Iso9660Error>
```

## Layered Architecture

```
┌────────────────────────────────────────────────────────────────┐
│                      Public API Layer                          │
│  mount() → find_file() → read_file() → find_boot_image()      │
└────────────────────────────────────────────────────────────────┘
                              ▼
┌────────────────────────────────────────────────────────────────┐
│                      Volume Layer                              │
│  Parses volume descriptors (Primary, Supplementary, Boot)     │
│  Detects Rock Ridge/Joliet extensions                         │
└────────────────────────────────────────────────────────────────┘
                              ▼
┌────────────────────────────────────────────────────────────────┐
│                    Directory Layer                             │
│  Navigates directory tree from root                           │
│  Parses directory records (variable-length structures)        │
│  Optional: Path table for fast lookup                         │
└────────────────────────────────────────────────────────────────┘
                              ▼
┌────────────────────────────────────────────────────────────────┐
│                      File Layer                                │
│  Reads file data from extent locations                        │
│  Handles fragmentation (multiple extents)                     │
│  Buffered I/O via FileReader                                  │
└────────────────────────────────────────────────────────────────┘
                              ▼
┌────────────────────────────────────────────────────────────────┐
│                      Boot Layer                                │
│  Parses El Torito boot catalog                                │
│  Extracts bootable images (kernels, floppy images)            │
│  Validates catalog checksums                                  │
└────────────────────────────────────────────────────────────────┘
```

## Key Types

### Core Types (`types.rs`)

- **VolumeInfo** - Parsed volume metadata (root extent, size, extensions)
- **FileEntry** - File/directory metadata (name, size, extent, flags)
- **FileFlags** - Bitfield flags (directory, hidden, extended, etc.)
- **BootImage** - Boot catalog entry (LBA, size, media type, platform)
- **BootMediaType** - No emulation, Floppy 1.2M/1.44M/2.88M, Hard Disk
- **BootPlatform** - x86, PowerPC, Mac, EFI
- **VolumeDescriptorType** - Boot, Primary, Supplementary, Terminator

### Error Type (`error.rs`)

**Iso9660Error** enum with 20+ variants:
- I/O errors: `IoError`, `ReadFailed`
- Format errors: `InvalidSignature`, `InvalidDirectoryRecord`, `InvalidBootCatalog`
- Filesystem errors: `NotFound`, `PathTooLong`, `ExtentOutOfBounds`
- Boot errors: `NoBootRecord`, `NoBootCatalog`, `ChecksumFailed`
- Extension errors: `RockRidgeError`, `JolietError`

## Data Structures

### Volume Descriptors (sector 16+, 2048 bytes each)

```
Boot Record (type 0):
├── identifier: "CD001"
├── boot_system_id: "EL TORITO SPECIFICATION"
└── boot_catalog_lba: u32 (pointer to boot catalog)

Primary VD (type 1):
├── identifier: "CD001"
├── volume_id: [u8; 32]
├── volume_space_size: both-endian u32
├── logical_block_size: both-endian u16 (2048)
├── root_directory_entry: DirectoryRecord (34 bytes)
└── ... (metadata fields)

Supplementary VD (type 2):
└── Same as Primary but with Joliet escape sequences

Terminator (type 255):
└── Ends volume descriptor set
```

### Directory Record (variable length, min 34 bytes)

```
DirectoryRecord:
├── length: u8 (total record length)
├── extent_lba: both-endian u32
├── data_length: both-endian u32
├── recording_datetime: [u8; 7]
├── file_flags: u8
├── file_id_len: u8
├── file_identifier: [u8; file_id_len]
├── padding: (1 byte if file_id_len is even)
└── system_use: (Rock Ridge data if present)
```

### El Torito Boot Catalog

```
Boot Catalog (2048 bytes at boot_catalog_lba):
├── Validation Entry (32 bytes):
│   ├── header_id: 0x01
│   ├── platform_id: u8 (x86=0x00, EFI=0xEF)
│   ├── id_string: [u8; 24]
│   ├── checksum: u16 (makes total sum zero)
│   └── key: [0x55, 0xAA]
│
└── Initial/Default Entry (32 bytes):
    ├── boot_indicator: 0x88 (bootable)
    ├── boot_media_type: u8 (0=no emulation, 4=hard disk)
    ├── load_segment: u16 (0 = default 0x7C0)
    ├── system_type: u8 (partition type)
    ├── sector_count: u16 (virtual 512-byte sectors)
    └── load_rba: u32 (ISO sector, 2048 bytes)
```

## ISO9660 Compliance

### Supported Features

- ✅ Primary Volume Descriptor parsing
- ✅ Directory tree navigation (root to leaf)
- ✅ File reading from extents
- ✅ El Torito boot catalog parsing
- ✅ Both-endian field handling (u32/u16)
- ✅ 7-byte directory datetime
- ✅ 17-byte volume datetime
- ✅ File version stripping (`;1`)
- ✅ d-characters (A-Z, 0-9, _)
- ✅ a-characters (extended ASCII set)

### Optional Extensions (Features)

- `joliet` - Supplementary VD with UCS-2 encoding (long Unicode filenames)
- `rock-ridge` - POSIX metadata (permissions, symlinks, long names)
- `debug` - Extra validation and logging

### Limitations (no_std constraints)

- No regex for pattern matching
- No recursive directory walking (manual iteration)
- No caching (stateless reads)
- Fixed-size buffers (sector-aligned)

## Usage Patterns

### Basic Mount and Read

```rust
use iso9660::{mount, find_file, read_file};

// Mount ISO from block device
let volume = mount(&mut block_io, start_sector)?;

// Find kernel file
let kernel = find_file(&mut block_io, &volume, "/boot/vmlinuz")?;

// Read kernel to buffer
let mut buffer = [0u8; 16 * 1024 * 1024];  // 16 MB
let bytes_read = read_file(&mut block_io, &kernel, &mut buffer)?;
```

### Extract Boot Image

```rust
use iso9660::{mount, find_boot_image};

let volume = mount(&mut block_io, 0)?;
let boot = find_boot_image(&mut block_io, &volume)?;

println!("Boot image at LBA {}, size {} bytes", boot.lba, boot.size);
println!("Media type: {:?}, Platform: {:?}", boot.media_type, boot.platform);
```

### Directory Iteration

```rust
use iso9660::directory::DirectoryIterator;

let root = volume.root_extent_lba;
let mut iter = DirectoryIterator::new(&mut block_io, root, volume.root_extent_len);

for entry in iter {
    let file = entry?;
    if !file.flags.directory {
        println!("File: {}, {} bytes", file.name(), file.size);
    }
}
```

## Implementation Plan

### Phase 1: Core Parsing (Current - Stubs Complete)
- [x] Module structure laid out
- [x] Type definitions complete
- [x] Error handling defined
- [ ] Primary Volume Descriptor parsing
- [ ] Directory record parsing
- [ ] File extent reading

### Phase 2: Boot Support
- [ ] Boot Record VD detection
- [ ] Boot catalog parsing
- [ ] Validation entry checksum
- [ ] Boot entry extraction

### Phase 3: Extensions
- [ ] Joliet (UCS-2 strings)
- [ ] Rock Ridge (POSIX attributes)
- [ ] Path table fast lookup

### Phase 4: Testing & Integration
- [ ] Unit tests for parsing
- [ ] Integration tests with real ISOs
- [ ] Bootloader integration (morpheusx)
- [ ] Example distro ISO support (Ubuntu, Tails, Arch)

## Testing Strategy

### Unit Tests
- Parse known-good volume descriptors
- Directory record edge cases
- Both-endian field conversion
- DateTime parsing (7-byte and 17-byte)
- El Torito checksum validation

### Integration Tests
- Mount real ISO images:
  - Ubuntu Server ISO
  - Tails live ISO
  - Arch Linux ISO
- Extract boot images
- Read initrd/kernel files
- Verify file contents against known hashes

### Bootloader Integration
- Use in morpheusx installer to:
  1. Mount distro ISO from ESP
  2. Extract vmlinuz + initrd
  3. Copy to /kernels/ and /initrds/
  4. Add boot menu entry

## Performance Considerations

### no_std Constraints
- **No heap caching** - Must re-read directory structures
- **Stack buffers** - Sector-aligned (2048 bytes)
- **Sequential I/O** - Minimize seeks with path table

### Optimization Opportunities
- Path table for O(1) directory lookup (vs O(n) tree walk)
- Single-pass directory iteration
- Extent batching for fragmented files
- Buffer pooling (if allocator available)

## Dependencies

- **gpt_disk_io** - BlockIo trait for sector-based I/O
- **gpt_disk_types** - Type definitions (minimal)
- **alloc** - Vec, String (via `extern crate alloc`)
- **core** - Result, Option, iterators

No external parsing libraries (pure implementation).

## Standards Compliance

- **ISO 9660:1988** - Base standard
- **ECMA-119** - Equivalent to ISO 9660
- **El Torito Specification 1.0** - Bootable CD-ROM format
- **Rock Ridge Interchange Protocol** - POSIX extensions
- **Joliet Specification** - Microsoft Unicode extensions

## Files Line Count Estimates

| Module          | Files | Est. Lines | Purpose                          |
|-----------------|-------|------------|----------------------------------|
| volume/         | 4     | ~800       | Volume descriptor parsing        |
| directory/      | 5     | ~1000      | Directory tree navigation        |
| file/           | 4     | ~600       | File reading and metadata        |
| boot/           | 5     | ~800       | El Torito boot catalog           |
| extensions/     | 2     | ~400       | Rock Ridge/Joliet                |
| utils/          | 5     | ~600       | Shared utilities                 |
| Core (lib/err)  | 3     | ~300       | Entry point and error types      |
| **Total**       | **28**| **~4500**  | **Pure no_std implementation**   |

## Future Enhancements

- UDF (Universal Disk Format) support for DVDs
- ISO 9660:1999 (version 2) support
- Write support (ISO creation)
- Hybrid ISO/MBR detection
- Multi-session CD support

---

**Status:** Architecture complete, stubs implemented, ready for parsing logic implementation.

**Next Steps:** Implement Primary Volume Descriptor parsing in `volume/primary.rs`.
