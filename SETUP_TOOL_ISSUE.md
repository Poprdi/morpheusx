# Issue: Create Intelligent Development Environment Setup Tool

## Problem Statement

MorpheusX currently has multiple setup scripts scattered across `testing/` that require manual execution in a specific order. New developers face:

- Uncertainty about which scripts to run and in what order
- Manual dependency installation across different Linux distributions
- No clear feedback on what's already configured vs. what needs setup
- Repeated downloads/builds when environment is already ready
- No single entry point for "get me a working dev environment"

The existing `setup-dev.sh` is a basic sequential script, not an intelligent tool that adapts to current system state.

## Goal

Create a professional shell tool (`setup-dev.sh`) in repo root that acts as an **intelligent environment manager**, not just a setup script. It should:

1. **Detect current state** - check what's installed, what's built, what's downloaded
2. **Report status** - clearly show what exists vs. what's missing
3. **Offer choices** - don't force downloads if environment is ready
4. **Be idempotent** - safe to run multiple times
5. **Support all distros** - auto-detect package manager (pacman/apt/dnf/yum/zypper/apk)
6. **Reuse existing scripts** - orchestrate `testing/*.sh` scripts, don't rewrite logic

## Deliverables

### 1. Smart State Detection

Tool must detect and report:
- [ ] System dependencies (nasm, qemu, ovmf, rust)
- [ ] Rust UEFI target installed
- [ ] OVMF firmware location (search multiple paths)
- [ ] Bootloader built (`testing/esp/EFI/BOOT/BOOTX64.EFI` exists + is recent)
- [ ] Distro images installed (Tails/Arch kernels in `testing/esp/kernels/`)
- [ ] Test disks created (`testing/test-disk-*.img`)
- [ ] ESP directory structure ready

### 2. Interactive Modes

Support multiple invocation patterns:

```bash
# Default: check state, show status, offer to complete missing parts
./setup-dev.sh

# Force full setup (ignore existing state)
./setup-dev.sh --force

# Just check status, don't modify anything
./setup-dev.sh --status

# Automated setup for CI (no prompts, sensible defaults)
./setup-dev.sh --auto

# Help text
./setup-dev.sh --help
```

### 3. Intelligent Behavior

**If everything already setup:**
```
✓ Development environment fully configured

Status:
  ✓ Dependencies installed (nasm, qemu, ovmf, rust)
  ✓ UEFI target: x86_64-unknown-uefi
  ✓ Bootloader built (2 hours ago)
  ✓ Tails OS installed (vmlinuz-tails, 1.3GB)
  ✓ Test disks created (50GB, 10GB)
  ✓ OVMF: /usr/share/OVMF/OVMF_CODE.fd

Ready to develop! Use:
  cd testing && ./run.sh
```

**If partial setup:**
```
Development environment partially configured

Status:
  ✓ Dependencies installed
  ✓ UEFI target installed
  ✗ Bootloader not built
  ✗ No distro images installed
  ✓ Test disks ready

Missing components:
  1. Build bootloader (~2 min)
  2. Install OS image (~15 min, 1.3GB download)

Continue setup? [Y/n]:
```

**If nothing setup:**
```
Development environment not configured

This will:
  1. Install dependencies (nasm, qemu, ovmf, rust)
  2. Setup Rust UEFI target
  3. Download Tails OS (1.3GB)
  4. Create test disks (50GB + 10GB sparse)
  5. Build bootloader (2-pass compilation)
  6. Launch QEMU

Total: ~1.5GB download, ~5GB disk, ~15-20 min

Continue? [Y/n]:
```

### 4. Package Manager Support

Must auto-detect and support:
- [x] Arch Linux (pacman)
- [x] Debian/Ubuntu (apt)
- [x] Fedora (dnf)
- [x] RHEL/CentOS (yum)
- [x] openSUSE (zypper)
- [x] Alpine (apk)
- [x] Unknown (print manual instructions)

For each distro, install:
- nasm (assembler)
- qemu-system-x86_64 (or qemu-full/qemu-kvm)
- OVMF/edk2 firmware
- rust (via package manager OR rustup fallback)
- Utilities: curl, rsync, parted, dosfstools, cpio, gzip

### 5. OVMF Path Auto-Fix

Search common locations:
- `/usr/share/OVMF/OVMF_CODE.fd`
- `/usr/share/edk2/ovmf/OVMF_CODE.fd`
- `/usr/share/edk2-ovmf/OVMF_CODE.fd`
- `/usr/share/ovmf/OVMF.fd`
- `/usr/share/qemu/ovmf-x86_64-code.bin`

Auto-patch `testing/run.sh` and `testing/run-persistence-test.sh` with found path.

### 6. Reuse Existing Scripts

Don't rewrite logic - orchestrate:
- `testing/build.sh` - for bootloader compilation
- `testing/install-tails.sh` - for Tails download/extraction
- `testing/install-arch.sh` - for Arch rootfs (optional)
- `testing/create-test-disk.sh` - for 50GB disk
- `testing/run.sh` - for launching QEMU

### 7. Error Handling

- Graceful failures (download timeouts, missing sudo, disk space)
- Clear error messages with actionable fixes
- Cleanup on Ctrl+C
- Non-zero exit codes on failure

### 8. Output Quality

- Color output (green ✓, red ✗, yellow ⚠)
- Progress indicators for long operations
- Minimal verbosity (hide script internals unless `--verbose`)
- Summary at end showing what changed

### 9. Documentation

Update `BUILD.md` with:
- New quick start section referencing `./setup-dev.sh`
- Explanation of different modes (--force, --status, --auto)
- Troubleshooting for common setup failures

## Technical Requirements

- POSIX shell compatible (bash 4+)
- No external dependencies beyond standard GNU tools
- Executable permission (`chmod +x setup-dev.sh`)
- Comments explaining non-obvious logic
- Structured functions (detect_deps, install_deps, check_status, etc.)
- Exit traps for cleanup

## Acceptance Criteria

- [ ] Run on fresh Arch system → full working environment + QEMU launches
- [ ] Run on fresh Ubuntu system → full working environment + QEMU launches
- [ ] Run on already-setup system → reports "ready", suggests `./run.sh`
- [ ] Run with `--force` on setup system → rebuilds everything
- [ ] Run with `--status` → only reports, doesn't modify
- [ ] Run with `--auto` → completes without prompts (for CI)
- [ ] Ctrl+C during download → cleans up temp files
- [ ] OVMF path auto-detected on Fedora/Debian/Arch
- [ ] `testing/run.sh` patched with correct OVMF path
- [ ] BUILD.md updated with new workflow

## Test Cases

```bash
# Fresh system
./setup-dev.sh          # Should complete full setup
cd testing && ./run.sh  # Should boot to Morpheus TUI

# Already setup
./setup-dev.sh          # Should report ready, exit quickly

# Status check
./setup-dev.sh --status # Just print status, exit 0 if ready

# Force rebuild
./setup-dev.sh --force  # Rebuild even if exists

# CI usage
./setup-dev.sh --auto   # Non-interactive, sensible defaults
```

## Non-Goals

- Supporting Windows/macOS (Linux-only)
- GUI interface
- Managing IDE setup (VSCode, etc.)
- Git configuration
- SSH key setup

## Related Files

- `testing/build.sh` - bootloader build orchestration
- `testing/run.sh` - QEMU launcher
- `testing/install-tails.sh` - Tails installer
- `testing/install-arch.sh` - Arch installer
- `testing/create-test-disk.sh` - disk image creator
- `BUILD.md` - documentation to update

## Priority

**High** - This is the first thing new developers encounter. Poor UX here means contributors give up before contributing.
