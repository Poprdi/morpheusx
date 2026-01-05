# iso9660

A `no_std` ISO9660 filesystem implementation in Rust with El Torito bootable CD support.

## Features

- **Pure `no_std`** - Works in bare metal, UEFI, embedded environments
- **ISO9660 Level 1/2/3** - Full standard support
- **El Torito** - Bootable CD/DVD parsing for kernel extraction
- **Rock Ridge** - POSIX extensions (optional feature)
- **Joliet** - Long filename support (optional feature)
- **Zero-copy** - Efficient parsing directly from block device

## Use Cases

- UEFI bootloaders that boot from ISO files on ESP
- Embedded systems booting from CD-ROM
- ISO file inspection/extraction in `no_std` contexts
- Extracting kernels and initrds from live ISOs

## Example

```rust
use iso9660::{mount, find_file, read_file, find_boot_image};

// Mount ISO from block device
let volume = mount(&mut block_io, start_sector)?;

// Find and read a file
let file = find_file(&mut block_io, &volume, "/boot/vmlinuz")?;
let kernel = read_file(&mut block_io, &volume, &file)?;

// Extract bootable image via El Torito
let boot = find_boot_image(&mut block_io, &volume)?;
```

## Architecture

```
iso9660/
├── volume/        # Volume descriptor parsing
├── directory/     # Directory record navigation
├── file/          # File reading from extents
├── boot/          # El Torito boot catalog
├── extensions/    # Rock Ridge, Joliet
└── utils/         # Datetime, string conversion, checksums
```

## ISO9660 Spec Reference

Based on ECMA-119 and ISO 9660:1988 standards.

## License

Licensed under either of:
- Apache License, Version 2.0 ([LICENSE-APACHE](../LICENSE-APACHE))
- MIT license ([LICENSE-MIT](../LICENSE-MIT))

at your option.
