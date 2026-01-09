# MorpheusX

MorpheusX is a UEFI boot and Hardware exokernel like runtime that loads Linux kernels directly from firmware space, treats distributions as disposable layers, and aims at keeping userland state persistent. It is written in Rust with a no_std core. Aswell as Asm firmware and ops. 

## What it does
- Boots Linux kernels from the EFI system partition with custom GPT and FAT32 handling
- Provides a boot-time TUI for selecting images and guiding installs
- Includes a network stack (Bare metal so after EBS "exit_bootservices") for downloading ISO's and updates (work in progress)
- Lays groundwork for persistence: capturing the loaded image, reversing relocations, and writing a bootable copy back to disk

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
