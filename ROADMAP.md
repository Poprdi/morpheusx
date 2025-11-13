# Morpheus Development Roadmap

**From Zero to Bootloader - A Multi-Year Journey**

---

## Phase 0: Foundation (COMPLETE)

**Goal:** Project structure that won't need refactoring

- [x] Create workspace structure
- [x] Define module boundaries
- [x] Document architecture decisions
- [x] Workspace compiles (empty stubs)

**Current Status:** DONE - Ready to code

---

## Phase 1: Hello World Bootloader (Weeks 1-4)

**Goal:** Boot to a screen that says "Morpheus"

### Learning Prerequisites:
- Read [UEFI Specification](https://uefi.org/specs) - Chapter 2 (Overview)
- Read [OSDev UEFI Tutorial](https://wiki.osdev.org/UEFI)
- Understand EFI System Partition (ESP) structure

### Tasks:

1. **Setup QEMU Testing** (Week 1)
   - Install QEMU + OVMF firmware
   - Create test disk image with ESP
   - Boot OVMF and see firmware menu
   - **Milestone:** Can boot OVMF in QEMU

2. **Hello World UEFI App** (Week 2)
   - `bootloader/src/main.rs` - Minimal UEFI entry
   - Use UEFI GOP (Graphics Output Protocol) to clear screen
   - Display "Morpheus" text
   - **Milestone:** See "Morpheus" text in QEMU

3. **Keyboard Input** (Week 3)
   - Implement `bootloader/src/tui/input.rs`
   - Use UEFI SimpleTextInput protocol
   - Detect arrow keys, enter, escape
   - **Milestone:** Can detect keypresses

4. **Basic TUI** (Week 4)
   - Draw a box on screen
   - Show "Press any key" message
   - Respond to keypress
   - **Milestone:** Interactive UEFI application

**Deliverable:** A UEFI app that displays a menu and responds to keyboard

---


### Tasks:

1. **Read/Write GPT** (Week 5) - ✅ COMPLETE
   - `core/src/disk/gpt_ops.rs` - GPT operations
   - Scan, create, modify partitions
   - Find partition by type GUID
   - **Milestone:** Full partition management in QEMU

2. **FAT32 Formatter** (Week 6)
   - `core/src/fs/fat32_format.rs` - Write FAT32 boot sector
   - Initialize FAT tables
   - Create root directory structure
   - **Milestone:** Format partitions as FAT32

3. **FAT32 Reader** (Week 7)
   - `core/src/fs/fat32_read.rs` - Read FAT32 boot sector
   - Parse directory entries
   - Read file contents
   - **Milestone:** Read files from FAT32 partitions

4. **Config File** (Week 8)
   - Create `ESP:/EFI/MORPHEUS/config.toml`
   - `bootloader/src/config.rs` - Parse TOML (minimal parser)
   - Load distro list from config
   - **Milestone:** Boot config loads from disk

**Deliverable:** Bootloader can format partitions, read/write files

**Note:** ext4/btrfs formatting will come later via Linux userspace tools. For now, focus on FAT32 since UEFI natively understands it.

---

## Phase 3: Kernel Loader (Weeks 9-12)

**Goal:** Load a Linux kernel into memory and jump to it

### Learning Prerequisites:
- Linux Boot Protocol (x86_64)
- ELF file format
- Memory management in UEFI

### Tasks:

1. **ELF Parser** (Week 9)
   - `core/src/fs/elf.rs` - Parse ELF headers
   - Load ELF segments into memory
   - Find entry point
   - **Milestone:** Can parse vmlinuz

2. **Boot Protocol** (Week 10)
   - `bootloader/src/boot.rs` - Setup boot params
   - Allocate memory for kernel
   - Prepare command line
   - **Milestone:** Boot params structure ready

3. **Kernel Jump** (Week 11)
   - `bootloader/src/arch/x86_64.rs` - Implement jump
   - Exit UEFI boot services
   - Jump to kernel entry point
   - **Milestone:** Kernel starts (might panic, that's OK)

4. **Initramfs** (Week 12)
   - Load initramfs into memory
   - Pass to kernel via boot params
   - **Milestone:** Kernel boots to shell!

**Deliverable:** Can boot ONE hardcoded Linux kernel

---

## Phase 4: Multi-Distro Menu (Weeks 13-16)

**Goal:** Show menu, let user pick distro, boot it

### Tasks:

1. **Pretty TUI** (Week 13-14)
   - `bootloader/src/tui/menu.rs` - Distro selection menu
   - Highlight selected item
   - Scroll if >10 distros
   - **Milestone:** Pretty menu UI

2. **Multiple Kernels** (Week 15)
   - Config lists multiple distros
   - Load selected distro's kernel
   - Boot whichever user picks
   - **Milestone:** Boot Arch OR Fedora

3. **Error Handling** (Week 16)
   - `core/src/error.rs` - Proper error types
   - Display errors in TUI
   - Fallback to menu if boot fails
   - **Milestone:** Graceful error handling

**Deliverable:** Multi-distro boot menu that works

---

## Phase 5: Ephemeral Root (Weeks 17-24)

**Goal:** Boot with overlayFS - fresh root every time

### Learning Prerequisites:
- OverlayFS concepts
- Initramfs structure
- Squashfs format

### Tasks:

1. **Squashfs Support** (Week 17-18)
   - `core/src/fs/squashfs.rs` - Read-only squashfs
   - Mount squashfs template
   - **Milestone:** Can read from .sqfs file

2. **Custom Initramfs** (Week 19-20)
   - Build custom initramfs with overlayfs tools
   - Mount template as lowerdir
   - Create tmpfs as upperdir
   - Union mount as root
   - **Milestone:** Boot with ephemeral root

3. **Template Creation** (Week 21-22)
   - Script to convert distro → squashfs
   - Test with Arch Linux
   - **Milestone:** Boot Arch from squashfs template

4. **Integration** (Week 23-24)
   - Bootloader passes overlay params to kernel
   - Initramfs sets up overlay
   - Boot into ephemeral Arch
   - **Milestone:** Fresh root every boot!

**Deliverable:** Ephemeral distro system working

---

## Phase 6: Persistent Userland (Weeks 25-32)

**Goal:** Your /home survives across boots

### Tasks:

1. **Partition Detection** (Week 25-26)
   - Detect "Morpheus Persistent" partition by label
   - Mount it in initramfs
   - **Milestone:** Can find persistent partition

2. **Bind Mounts** (Week 27-28)
   - `persistent/` crate implementation
   - Bind mount /persistent/home → /home
   - Bind mount /persistent/.config → /root/.config
   - **Milestone:** Home directory persists

3. **Cross-Distro Sync** (Week 29-30)
   - Dotfiles that work across distros
   - Distro-specific configs in subdirs
   - **Milestone:** Same home on Arch and Fedora

4. **Data Persistence** (Week 31-32)
   - Mount /persistent/data → /opt/data
   - Dev projects persist
   - **Milestone:** Full development environment persists

**Deliverable:** User data survives distro changes

---

## Phase 7: Network & Updates (Weeks 33-48) - YEAR 2

**Goal:** Download and update distro templates

### Tasks:

1. **HTTP Client** (Week 33-40)
   - `network/` crate - Minimal HTTP/1.1
   - TCP stack (use UEFI network protocols)
   - Download files
   - **Milestone:** Can download a file

2. **Distro Registry** (Week 41-44)
   - `registry/` crate - Mirror parsing
   - Detect latest Arch version
   - Parse Fedora metadata
   - **Milestone:** Can query distro versions

3. **Template Updater** (Week 45-48)
   - `updater/` crate - Download new templates
   - Verify checksums
   - Replace old templates
   - **Milestone:** Update Arch template from mirrors

**Deliverable:** Auto-updating distro templates

---

## Phase 8: Redeco Integration (Weeks 49-56)

**Goal:** Persistent network configuration via Redeco

### Tasks:

1. **Redeco Service** (Week 49-52)
   - `redeco-integration/` crate
   - Generate systemd service
   - Start redeco daemon early in boot
   - **Milestone:** Redeco runs on boot

2. **State Persistence** (Week 53-56)
   - Redeco config in /persistent/redeco
   - Network state survives reboots
   - Same IP across distros
   - **Milestone:** Network config persists

**Deliverable:** Redeco fully integrated

---

## Phase 9: Multi-Arch Support (Weeks 57-72) - YEAR 3

**Goal:** Works on x86_64, ARM64, and ARM32

### Tasks:

1. **ARM64 Port** (Week 57-64)
   - `bootloader/src/arch/aarch64.rs`
   - Test on Raspberry Pi 4
   - **Milestone:** Boots on ARM64

2. **ARM32 Port** (Week 65-72)
   - `bootloader/src/arch/armv7.rs`
   - Test on Raspberry Pi 3
   - **Milestone:** Boots on ARM32

**Deliverable:** Universal bootloader

---

## Phase 10: Polish & Release (Weeks 73-80)

**Goal:** Production-ready 1.0 release

### Tasks:

1. **Installer** (Week 73-76)
   - `installer/` crate implementation
   - Partition disk wizard
   - Download initial templates
   - **Milestone:** Easy installation

2. **Documentation** (Week 77-78)
   - User manual
   - Developer docs
   - Video tutorials

3. **Testing** (Week 79-80)
   - Comprehensive test suite
   - Test on real hardware
   - Fix bugs

**Deliverable:** Morpheus 1.0 Release

---

## Quick Start Guide

### Right Now (Today):
```bash
cd morpheus
cargo check --workspace
```

### This Week:
1. Install QEMU: `sudo dnf install qemu-system-x86 edk2-ovmf`
2. Read UEFI basics
3. Start Phase 1, Task 1

### This Month:
- Complete Phase 1 (Hello World)
- Start Phase 2 (Disk Detective)

### This Year:
- Phases 1-6 complete
- Working ephemeral bootloader with persistence

---

## Resources

- **UEFI Spec:** https://uefi.org/specs
- **OSDev Wiki:** https://wiki.osdev.org
- **Linux Boot Protocol:** https://www.kernel.org/doc/html/latest/x86/boot.html
- **Rust UEFI:** https://github.com/rust-osdev

---

**Remember:** This is a marathon, not a sprint. Take your time, learn deeply, build solidly.

Good luck, this is going to be legendary.
