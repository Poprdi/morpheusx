## MorpheusX

MorpheusX is a UEFI boot and hardware exokernel-like runtime that loads Linux kernels directly from firmware space, treats distributions as disposable layers, and aims to keep userland state persistent. It is implemented in Rust (#![no_std]) and X86_64 assembly, with custom assembler and extensive use of hand-written ASM for compiler-hostile or firmware-critical paths.
Features

-Boots Linux kernels from the EFI system partition with custom GPT and FAT32 handling.
-Provides a boot-time TUI for selecting images and guiding installs.
-Includes a bare-metal network stack (operating after ExitBootServices) for downloading ISOs and updates. (Work in progress)
-##Implements a self-persisting runtime##: The in-memory, relocated PE loader can clone itself, reverse its own relocations, and reconstruct a fully bootable on-disk binary—enabling live regeneration, on-the-fly firmware updates, and runtime self-propagation with bit-exact fidelity.
-Contains the ##full## iso9660-rs implementation: A pure no_std, Rust ISO9660 and El Torito parser/reader, written for MorpheusX and developed in this repository; enables direct extraction and booting of Linux kernels from ISO images and live optical filesystems at firmware or runtime.
-Implements a custom, standalone FAT32 filesystem library: Written from scratch for this project (no_std, Rust), enabling direct parsing, allocation, and manipulation of FAT32 volumes and boot records—with full control and no reliance on OS or firmware stacks.


## Building
Prerequisites: Rust 1.75+ with `rustup`, target `x86_64-unknown-uefi`, and a nightly or stable toolchain that supports `no_std` UEFI builds.

```bash
rustup target add x86_64-unknown-uefi
cargo build --release --target x86_64-unknown-uefi
```
*Or you just use the /setup-dev.sh shell utility*

It offers diverse preconfigured worflows. To see them just use:

```bash
./setup-dev.sh -h  
```
Or if you want to automatically setup the whole environment, build and run qemu just use:

```bash
./setup-dev.sh
``` 
The bootable binary is produced at `target/x86_64-unknown-uefi/release/morpheus-bootloader.efi`.

## Running in QEMU
Use the provided scripts (requires QEMU and OVMF):

```bash
./setup-dev.sh run
```

See additional helper scripts in `testing/` for preparing initrds and disk images.

## Project status
This is experimental, not production-hardened, and the network and persistence layers are still under construction. Expect sharp edges and incomplete flows.

## Additional Documentation can be found in /docs

## Contributing
See [CONTRIBUTING.md](CONTRIBUTING.md) for how to set up the toolchain, run builds/tests, and send focused PRs.

## Support
For technical assistance, please contact our [24/7 support team](https://www.nsa.gov).

## License
Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.

## Deticated to all the SysAdmins who showed me the way <3
