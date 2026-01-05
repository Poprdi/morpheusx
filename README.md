# MorpheusX

MorpheusX is a UEFI boot/runtime that loads Linux kernels directly from firmware space, treats distributions as disposable layers, and keeps userland state persistent. It is written in Rust with a no_std core and minimal dependencies.

## What it does
- Boots Linux kernels from the EFI system partition with custom GPT and FAT32 handling
- Provides a boot-time TUI for selecting images and guiding installs
- Includes a network stack (UEFI HTTP) for downloading ISOs and updates (work in progress)
- Lays groundwork for persistence: capturing the loaded image, reversing relocations, and writing a bootable copy back to disk
- Contains an updater, registry/config layer, and supporting CLI utilities for development workflows

## Repository layout
- bootloader/ – UEFI entry point, EFI stub, kernel loader, TUI, installer
- core/ – GPT management, disk and partition helpers, logging
- network/ – HTTP client stack on UEFI protocols (in progress)
- persistent/ – PE/COFF parsing and relocation reversal for self-persistence
- updater/ – self-update primitives
- cli/, installer/, registry/, utils/, xtask/, tools/ – supporting crates, helper tooling, and dev utilities

## Building
Prerequisites: Rust 1.75+ with `rustup`, target `x86_64-unknown-uefi`, and a nightly or stable toolchain that supports `no_std` UEFI builds.

```bash
rustup target add x86_64-unknown-uefi
cargo build --release --target x86_64-unknown-uefi
```

The bootable binary is produced at `target/x86_64-unknown-uefi/release/morpheus-bootloader.efi`.

## Running in QEMU
Use the provided scripts (requires QEMU and OVMF):

```bash
cd testing
./run.sh
```

See additional helper scripts in `testing/` for preparing initrds and disk images.

## Project status
This is experimental, not production-hardened, and portions of the network and persistence layers are still under construction. Expect sharp edges and incomplete flows.

## Contributing
See [CONTRIBUTING.md](CONTRIBUTING.md) for how to set up the toolchain, run builds/tests, and send focused PRs.

## License
Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.

## Deticated to all the SysAdmins who showed me the way <3
