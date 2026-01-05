# Contributing to MorpheusX

Thanks for your interest in improving MorpheusX! This project is experimental and low-level (UEFI, `no_std`). Please keep changes small and well-documented.

## Quick start
- Fork the repo and create a feature branch.
- Run the setup script: `./setup-dev.sh` (handles dependencies, toolchain, and test images).
- Build: `cargo build --release --target x86_64-unknown-uefi`.
- Run in QEMU/OVMF: `cd testing && ./run.sh`.

## Expectations for pull requests
- Keep PRs focused and describe the user-visible behavior change.
- Include reasoning for architectural changes or new unsafe code.
- Add tests when possible (unit tests for pure logic; use `testing/` scripts for integration).
- Ensure builds succeed for the UEFI target before submitting.

## Style and hygiene
- Prefer clear, small modules over large functions; document unsafe blocks briefly.
- Run `cargo fmt` before sending a PR.
- Run `cargo clippy --target x86_64-unknown-uefi` when applicable; allow lints only with justification.

## Reporting issues
- Include firmware/virtualization details (machine model, UEFI version, QEMU/OVMF versions) and reproduction steps.
- Attach logs or screenshots from the TUI/serial console when relevant.

## Security and release scope
This code is not production-hardened. Do not treat it as a secure boot chain. Avoid submitting features that imply security guarantees without accompanying design notes and tests.
