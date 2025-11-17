# Morpheus Testing Environment

## Quick Start

```bash
# Build the bootloader
./build.sh

# Install a live Linux distribution (optional, for full userland testing)
./install-live-distro.sh

# Run in QEMU
./run.sh
```

## Available Rootfs Options

### 1. Minimal Arch Rootfs (Current)
```bash
./install-arch.sh
```
- Basic Arch Linux bootstrap (~500MB download, ~2GB rootfs)
- **Limitations**: No networking, minimal tools
- Good for: Basic boot testing

### 2. Live Linux Distributions (Recommended)
```bash
./install-live-distro.sh
```
Choose from:
- **Ubuntu 24.04** - Full desktop, easy to use (5.7GB)
- **Debian 12** - Lightweight, reliable (3.1GB)
- **Tails 6.9** - Privacy-focused, fully-featured (1.3GB) ⭐ **Recommended for testing**
- **Fedora 40** - Cutting-edge, complete (2.3GB)
- **Kali Linux** - Pentesting tools, networking (4.1GB)

**Benefits of Live Systems:**
- ✓ Full networking stack (DHCP, NetworkManager, SSH)
- ✓ Complete userland tools (nano, vim, grep, find, etc.)
- ✓ Package managers (apt/dnf/pacman)
- ✓ Hardware drivers and firmware
- ✓ Ready to go - no setup needed

### 3. Tails OS Only (Quick)
```bash
./install-tails.sh
```
- Direct Tails installation
- Smaller download (1.3GB)
- Full-featured privacy-focused OS

## What This Does

1. Builds Morpheus as a UEFI application
2. Copies it to a test ESP (EFI System Partition)
3. Boots QEMU with OVMF firmware
4. OVMF finds and executes Morpheus
5. Morpheus loads the kernel + initrd/rootfs

## Expected Behavior

- Morpheus bootloader menu appears
- Select kernel to boot
- Linux kernel loads and boots
- Full userland environment available (if using live distro)

## Troubleshooting

If QEMU doesn't start:
- Check OVMF path in run.sh
- Ensure qemu-system-x86_64 is installed
- Try with `-nographic` flag for serial console

If Linux doesn't boot:
- Check kernel parameters are correct for your distro
- Ensure initrd and squashfs files are present
- Check serial console output for errors

## File Locations

After running install scripts:
```
esp/
├── EFI/BOOT/BOOTX64.EFI          # Morpheus bootloader
├── kernels/
│   ├── vmlinuz-arch              # Arch kernel
│   ├── vmlinuz-tails             # Tails kernel
│   ├── vmlinuz-ubuntu            # Ubuntu kernel (if installed)
│   └── ...
└── initrds/
    ├── initrd-arch.img           # Arch initrd
    ├── initrd-tails.img          # Tails initrd
    ├── filesystem-tails.squashfs # Tails rootfs
    └── ...
```
