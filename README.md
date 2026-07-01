# MorpheusX

An "exokernel" written in Rust. MorpheusX boots via UEFI, and is and upcomming OS that aims to provide maximal control over hardware while delivering sensible abstractions for convenience.

Yes it actually boots on real hardware. It is a work in progress but is already capable of booting all the way into userland. It successfully brings up AP's and manages real silicon. 

In theory it should be able to run on any x86_64 UEFI-compatible system, manage USB devices, and ahci devices aswell as Virtio devices.

This Repo contains only the KERNEL - I have moved the userland to a private repository for the time being, but you are free to roll your own.
My next steps are Porting the Rust PAL and upstreaming it so this kernel becomes an actuall build target and the ecosystem becomes available. 

## Tested on: Fujitsu D3674-B13 -- ThinkPad T450s

## Serial console

COM1, **115200 8N1, no flow control**. The kernel programs the UART itself, so
the baud is fixed regardless of firmware.

## Building

```bash
./setup-dev.sh             # For auto setup
./setup-dev.sh -f          # for force rebuilding
./setup-dev.sh -h          # for help
```

Requires: Rust 1.75+, `x86_64-unknown-uefi` target, QEMU + OVMF for testing.

## Running

```bash
./setup-dev.sh run  # for running in QEMU
```

For running on real hardware, you will need to flash the UEFI binary to a USB drive and boot from it. For this you may use:

```bash
./setup-dev.sh flash /dev/sdX # where /dev/sdX is the target USB drive
```

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for:
- Setting up your development environment
- Code style and conventions
- Testing and CI workflow
- Creating pull requests

**TL;DR**: Fork → branch → cargo clippy → commit → PR.

---

## License

Licenced under GPLv3  

---

## Dedication

To all the SysAdmins who showed me the way. 💙
