# Changelog

## 1.0.2 - 2026-01-06
- Expanded public API surface: added 13+ publicly exported items
- Added comprehensive rustdoc examples to all core functions (mount, find_file, read_file, find_boot_image)
- Enhanced README with detailed API overview (high-level functions + advanced APIs)
- Updated ARCHITECTURE.md with complete public API surface documentation
- Improved El Torito boot support documentation with field access examples
- Added typical usage patterns showing three different reading approaches

## 1.0.1 - 2026-01-06
- First public release of `iso9660-rs`
- Completed El Torito boot catalog parsing and checksum validation
- Implemented buffered `FileReader` with seek/read/EOF handling
- Added GitHub Actions CI (tests, all-features, no_std UEFI builds, clippy, docs)
- Polished README with install instructions and API overview
- Added MIT and Apache-2.0 licenses; crate metadata ready for crates.io
