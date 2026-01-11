# CI/CD & Regression Harness Session Ledger

> Maintained throughout the CI/CD integration session. Updates after each chunk.

## Current Phase: Chunk G - Stabilization

### Last Updated: 2026-01-11

---

## Session Status

| Phase | Status | Artifacts |
|-------|--------|-----------|
| Chunk A: Repo Discovery | ✅ Complete | This document |
| Chunk B: CI/CD Design | ✅ Complete | Design below |
| Chunk C: Regression Plan | ⏳ Pending | — |
| Chunk D: CI Workflows | ✅ Complete | `.github/workflows/ci.yml` |
| Chunk E: E2E Workflows | ✅ Complete | `.github/workflows/uefi-e2e.yml`, `uefi-build.yml` |
| Chunk F: Scripts | ✅ Complete | `scripts/qemu-e2e.sh`, `scripts/gen-fixtures.sh`, `scripts/ci-build-uefi.sh` |
| Chunk G: Stabilization | 🔄 In Progress | `deny.toml`, README updates |

---

## Repository Discovery (Chunk A Summary)

### Workspace Structure

```
morpheusx/                          # Cargo workspace (resolver=2)
├── bootloader/                     # UEFI entry point (x86_64-unknown-uefi target)
├── core/                           # GPT, FAT32, disk I/O, logging (no_std)
├── persistent/                     # PE/COFF parsing, relocation reversal (no_std)
├── updater/                        # Self-update logic (no_std)
├── network/                        # Bare-metal networking + smoltcp (no_std)
├── iso9660/                        # ISO filesystem parser (no_std, published crate)
└── dma-pool/                       # DMA buffer management (no_std)
```

### Target Architecture

- **Primary**: `x86_64-unknown-uefi` (UEFI bootloader)
- **Host tests**: `x86_64-unknown-linux-gnu` (unit tests)
- **Toolchain**: Rust 1.75+ minimum (workspace.rust-version)
- **Profile**: `opt-level="z"`, `lto=true`, `panic="abort"`, stripped

### Build Process (2-Pass)

1. **Pass 1**: Build bootloader → produces `.efi`
2. **Extract**: `tools/extract-reloc-data.sh` parses PE, generates `persistent/src/pe/embedded_reloc_data.rs`
3. **Pass 2**: Rebuild with embedded reloc data
4. **Output**: `testing/esp/EFI/BOOT/BOOTX64.EFI`

### Test Inventory

#### Unit Tests (inline `#[cfg(test)]`)

| Crate | File | Coverage |
|-------|------|----------|
| `iso9660` | `src/utils/checksum.rs` | Checksum algorithms |
| `network` | `src/url/parser.rs` | URL parsing (~35 tests) |
| `network` | `src/transfer/chunked.rs` | HTTP chunked encoding (~20 tests) |
| `network` | `src/device/registers.rs` | VirtIO registers |
| `network` | `src/client/native.rs` | HTTP client stubs |

#### Integration Tests (`tests/`)

| Crate | File | Tests |
|-------|------|-------|
| `iso9660` | `block_io_tests.rs` | Block device abstraction |
| `iso9660` | `volume_tests.rs` | Volume mounting (~6 tests) |
| `iso9660` | `directory_tests.rs` | Directory parsing |
| `iso9660` | `file_tests.rs` | File lookup/read |
| `iso9660` | `boot_tests.rs` | El Torito boot catalog |
| `iso9660` | `integration_tests.rs` | Real ISO parsing (optional) |

#### E2E / Manual Scripts (`testing/`)

| Script | Purpose |
|--------|---------|
| `build.sh` | Full 2-pass clean build |
| `test-network.sh` | QEMU + VirtIO-net + HTTP server |
| `test-boot.exp` | Expect script for automated serial assertion |
| `install-{arch,tails,live-distro}.sh` | Download distros for testing |
| `create-minimal-initrd.sh` | Generate minimal busybox initrd |

#### Test Runner

- `iso9660/run-tests.sh`: Phases unit→integration→extended (real ISO)

### Dependencies (External)

- **NASM**: Assembles `trampoline32.asm` in bootloader
- **QEMU + OVMF**: E2E testing
- **genisoimage/mkisofs**: ISO fixture generation
- **mtools/dosfstools**: FAT image creation
- **expect**: Automated serial output assertion

### Notable Paths

- `testing/esp/`: FAT32 ESP image layout
- `tools/extract-*.sh`: PE parsing scripts for relocation extraction
- `persistent/src/pe/embedded_reloc_data.rs`: Generated file (2-pass build)

---

## CI/CD Design (Chunk B)

### Design Principles

1. **Deterministic**: Pinned toolchains, cacheable, reproducible
2. **Fast feedback**: Lint → Build → Test cascade with fail-fast
3. **No vendor lock-in**: GitHub Actions only, no external SaaS
4. **Transparent**: All scripts checked in, explicit preconditions
5. **Security-conscious**: cargo-deny, cargo-audit in scheduled runs

### Job Matrix

```
┌─────────────────────────────────────────────────────────────────┐
│                         CI Pipeline                              │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────┐                                               │
│  │    Lint      │  rustfmt --check, clippy -D warnings          │
│  └──────┬───────┘                                               │
│         │                                                        │
│  ┌──────▼───────┐                                               │
│  │ Build (host) │  cargo build --all-features                   │
│  └──────┬───────┘                                               │
│         │                                                        │
│  ┌──────▼───────┐                                               │
│  │ Test (host)  │  cargo test --all-features                    │
│  └──────┬───────┘                                               │
│         │                                                        │
│  ┌──────▼───────┐                                               │
│  │ Build (UEFI) │  x86_64-unknown-uefi, 2-pass                  │
│  └──────┬───────┘                                               │
│         │                                                        │
│  ┌──────▼───────┐                                               │
│  │ E2E (QEMU)   │  Boot .efi via OVMF, assert serial token      │
│  └──────────────┘                                               │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Workflow Files

| Workflow | Trigger | Jobs |
|----------|---------|------|
| `ci.yml` | push/PR to `main` | lint, build-host, test-host |
| `uefi-build.yml` | push/PR to `main` | build-uefi (2-pass), upload artifact |
| `uefi-e2e.yml` | push/PR to `main` | build-uefi → qemu-e2e (optional gating) |
| `audit.yml` | schedule (weekly) | cargo-deny, cargo-audit |
| `release.yml` | tag `v*.*.*` | build-uefi, create release, attach .efi |

### Caching Strategy

```yaml
- uses: actions/cache@v4
  with:
    path: |
      ~/.cargo/registry
      ~/.cargo/git
      target
    key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
    restore-keys: |
      ${{ runner.os }}-cargo-
```

### UEFI Build Steps

```bash
# 1. Install target
rustup target add x86_64-unknown-uefi

# 2. Install NASM
apt-get install -y nasm

# 3. Pass 1: Build
cargo build --target x86_64-unknown-uefi -p morpheus-bootloader --release

# 4. Extract relocations
./tools/extract-reloc-data.sh

# 5. Pass 2: Rebuild with embedded relocs
cargo clean -p morpheus-bootloader
cargo build --target x86_64-unknown-uefi -p morpheus-bootloader --release

# 6. Output artifact
# target/x86_64-unknown-uefi/release/morpheus-bootloader.efi
```

### E2E Test Contract

The bootloader must emit a deterministic serial token on successful boot:

```
MORPHEUSX_BOOT_OK
```

This enables CI gating without complex parsing. Token emission should be:
- Feature-gated (`#[cfg(feature = "ci_boot_token")]`)
- Emitted via serial port (COM1, 0x3F8) after reaching stable state
- Bounded: test times out after 60s

### Scripts to Create

| Script | Purpose |
|--------|---------|
| `scripts/qemu-e2e.sh` | Boot .efi via OVMF, capture serial, assert token |
| `scripts/gen-fixtures.sh` | Generate ISO/FAT fixtures (no binary blobs) |
| `scripts/ci-build-uefi.sh` | Wrapper for 2-pass UEFI build |

---

## Assumptions

1. Repository owner/name: `Poprdi/morpheusx` (from iso9660 Cargo.toml)
2. CI runners: GitHub-hosted Ubuntu (`ubuntu-latest`)
3. OVMF available via `apt-get install ovmf`
4. No existing `.github/workflows/` (only `.github/agents/` empty)
5. Release artifacts: single `.efi` file + optional source tarball

---

## Decisions Log

| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-01-11 | Use GitHub Actions only | No external SaaS, transparent |
| 2026-01-11 | 2-pass build in CI | Required for reloc embedding |
| 2026-01-11 | Serial token for E2E | Simple, deterministic assertion |
| 2026-01-11 | Separate UEFI build workflow | Isolate slow build from fast lint/test |
| 2026-01-11 | Weekly audit schedule | Security without blocking PRs |

---

## TODOs

- [x] Implement `scripts/qemu-e2e.sh`
- [x] Implement `scripts/gen-fixtures.sh`
- [x] Implement `scripts/ci-build-uefi.sh`
- [x] Create `.github/workflows/ci.yml`
- [x] Create `.github/workflows/uefi-build.yml`
- [x] Create `.github/workflows/uefi-e2e.yml`
- [x] Create `.github/workflows/audit.yml`
- [x] Create `.github/workflows/release.yml`
- [x] Create `deny.toml` for cargo-deny
- [ ] Add CI boot token feature flag to bootloader
- [ ] Update README with CI badges and E2E requirements
- [ ] Test workflows in GitHub Actions
