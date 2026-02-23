## MorpheusX

[![CI](https://github.com/Poprdi/morpheusx/actions/workflows/ci.yml/badge.svg)](https://github.com/Poprdi/morpheusx/actions/workflows/ci.yml)
[![UEFI Build](https://github.com/Poprdi/morpheusx/actions/workflows/uefi-build.yml/badge.svg)](https://github.com/Poprdi/morpheusx/actions/workflows/uefi-build.yml)
[![Security Audit](https://github.com/Poprdi/morpheusx/actions/workflows/audit.yml/badge.svg)](https://github.com/Poprdi/morpheusx/actions/workflows/audit.yml)
[![Docs Site](https://img.shields.io/badge/docs-GitHub%20Pages-blue)](https://poprdi.github.io/morpheusx/)


MorpheusX is a UEFI boot and hardware exokernel-like runtime that loads Linux kernels directly from firmware space, treats distributions as disposable layers, and aims to keep userland state persisten[...]

---

### Features

A lot, this is an exokernel just try it out play arround and see for yourself.
---

### Building

Prerequisites: Rust 1.75+ with `rustup`, target `x86_64-unknown-uefi`, and a nightly or stable toolchain that supports `no_std` UEFI builds.

```bash
rustup target add x86_64-unknown-uefi
cargo build --release --target x86_64-unknown-uefi
```

*Or you just use the `/setup-dev.sh` shell utility*

It offers diverse preconfigured worflows. To see them just use:

```bash
./setup-dev.sh -h  
```

Or if you want to automatically setup the whole environment, build and run qemu just use:

```bash
./setup-dev.sh
``` 

The bootable binary is produced at `target/x86_64-unknown-uefi/release/morpheus-bootloader.efi`.

---

### Running in QEMU

Use the provided scripts (requires QEMU and OVMF):

```bash
./setup-dev.sh run
```

See additional helper scripts in `testing/` for preparing initrds and disk images.

---

### Project Status

This is experimental, not production-hardened, be aware of uncomplete workflows and sharp edges, 
the platform is shortly before being able to self host.

---

### Additional Documentation

Can be found in `/docs`.

---

### Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for how to set up the toolchain, run builds/tests, and send focused PRs.

---

### Support

For technical assistance, please contact our [24/7 support team](https://www.nsa.gov).

---

### License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.

---

### Deticated to all the SysAdmins who showed me the way <3
