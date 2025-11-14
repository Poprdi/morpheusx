#!/bin/bash
# Install Arch Linux rootfs for Morpheus bootloader testing
# Downloads compressed Arch rootfs and configures bootloader

set -e

cd "$(dirname "$0")"

ESP_DIR="esp"
KERNELS_DIR="$ESP_DIR/kernels"
INITRD_DIR="$ESP_DIR/initrds"
ROOTFS_DIR="$ESP_DIR/rootfs"
WORK_DIR="/tmp/morpheus-arch-setup"

echo "========================================"
echo "  Morpheus Arch Linux Rootfs Setup"
echo "========================================"
echo ""
echo "This script will:"
echo "  1. Download Arch Linux bootstrap rootfs"
echo "  2. Download kernel and initramfs"
echo "  3. Configure bootloader for Arch Linux"
echo ""
read -p "Continue? [Y/n]: " -n 1 -r
echo
if [[ $REPLY =~ ^[Nn]$ ]]; then
    echo "Aborted."
    exit 0
fi

# Create directories
echo ""
echo "Creating directory structure..."
mkdir -p "$KERNELS_DIR"
mkdir -p "$INITRD_DIR"
mkdir -p "$ROOTFS_DIR"
mkdir -p "$WORK_DIR"
echo "✓ Directories created"

# Check for required tools
echo ""
echo "Checking dependencies..."
if ! command -v curl &> /dev/null; then
    echo "Installing curl..."
    sudo apt-get update -qq
    sudo apt-get install -y curl
fi
echo "✓ Dependencies ready"

# Download Arch Linux bootstrap
echo ""
echo "Downloading Arch Linux bootstrap rootfs..."
BOOTSTRAP_URL="https://geo.mirror.pkgbuild.com/iso/latest/archlinux-bootstrap-x86_64.tar.gz"
BOOTSTRAP_FILE="$WORK_DIR/archlinux-bootstrap.tar.gz"

if [ -f "$BOOTSTRAP_FILE" ]; then
    echo "Using cached bootstrap: $BOOTSTRAP_FILE"
else
    echo "Downloading from: $BOOTSTRAP_URL"
    echo "Size: ~150 MB (compressed)"
    echo ""
    curl -L -o "$BOOTSTRAP_FILE" "$BOOTSTRAP_URL" --progress-bar || {
        echo "✗ Download failed!"
        echo "Trying alternative mirror..."
        BOOTSTRAP_URL="https://america.mirror.pkgbuild.com/iso/latest/archlinux-bootstrap-x86_64.tar.gz"
        curl -L -o "$BOOTSTRAP_FILE" "$BOOTSTRAP_URL" --progress-bar || {
            echo "✗ All download attempts failed!"
            exit 1
        }
    }
    echo ""
    echo "✓ Downloaded bootstrap: $(du -h "$BOOTSTRAP_FILE" | cut -f1)"
fi

# Extract kernel and initramfs from bootstrap
echo ""
echo "Extracting kernel and initramfs from bootstrap..."
cd "$WORK_DIR"

# Extract just the boot files we need
tar -xzf "$BOOTSTRAP_FILE" root.x86_64/boot/vmlinuz-linux root.x86_64/boot/initramfs-linux.img 2>/dev/null || {
    echo "Note: Boot files not in bootstrap, will download separately"
}

# Copy kernel if extracted
if [ -f "$WORK_DIR/root.x86_64/boot/vmlinuz-linux" ]; then
    cp "$WORK_DIR/root.x86_64/boot/vmlinuz-linux" "$KERNELS_DIR/vmlinuz-arch"
    chmod 644 "$KERNELS_DIR/vmlinuz-arch"
    echo "✓ Kernel: $KERNELS_DIR/vmlinuz-arch ($(du -h "$KERNELS_DIR/vmlinuz-arch" | cut -f1))"
else
    # Download kernel from Arch package repository
    echo "Downloading kernel package..."
    KERNEL_URL="https://geo.mirror.pkgbuild.com/core/os/x86_64/"
    KERNEL_PKG=$(curl -s "$KERNEL_URL" | grep -oP 'linux-[0-9]+\.[0-9]+\.[0-9]+-[0-9]+-x86_64\.pkg\.tar\.zst' | head -1)
    
    if [ -z "$KERNEL_PKG" ]; then
        echo "✗ Could not find kernel package"
        exit 1
    fi
    
    curl -L -o kernel.pkg.tar.zst "$KERNEL_URL$KERNEL_PKG" --progress-bar
    
    # Extract kernel from package (using tar, as zstd is in the package)
    if command -v zstd &> /dev/null; then
        zstd -d kernel.pkg.tar.zst -c | tar -x boot/vmlinuz-linux 2>/dev/null || tar -xf kernel.pkg.tar.zst boot/vmlinuz-linux
    else
        # Install zstd
        sudo apt-get install -y zstd
        zstd -d kernel.pkg.tar.zst -c | tar -x boot/vmlinuz-linux
    fi
    
    cp boot/vmlinuz-linux "$KERNELS_DIR/vmlinuz-arch"
    chmod 644 "$KERNELS_DIR/vmlinuz-arch"
    echo "✓ Kernel: $KERNELS_DIR/vmlinuz-arch ($(du -h "$KERNELS_DIR/vmlinuz-arch" | cut -f1))"
fi

# Copy initramfs if extracted
if [ -f "$WORK_DIR/root.x86_64/boot/initramfs-linux.img" ]; then
    cp "$WORK_DIR/root.x86_64/boot/initramfs-linux.img" "$INITRD_DIR/initramfs-arch.img"
    chmod 644 "$INITRD_DIR/initramfs-arch.img"
    echo "✓ Initramfs: $INITRD_DIR/initramfs-arch.img ($(du -h "$INITRD_DIR/initramfs-arch.img" | cut -f1))"
else
    # Use the one from kernel package or create minimal one
    if [ -f "boot/initramfs-linux.img" ]; then
        cp boot/initramfs-linux.img "$INITRD_DIR/initramfs-arch.img"
        chmod 644 "$INITRD_DIR/initramfs-arch.img"
    else
        echo "Warning: No initramfs found, will need to download separately"
    fi
fi

# Extract rootfs to ESP
echo ""
echo "Extracting Arch Linux rootfs..."
cd "$ROOTFS_DIR"
sudo tar -xzf "$BOOTSTRAP_FILE" --strip-components=1 || {
    echo "✗ Failed to extract rootfs"
    exit 1
}
echo "✓ Rootfs extracted to $ROOTFS_DIR"

# Create boot configuration
cd "$(dirname "$0")"
cat > "$ROOTFS_DIR/boot-config.txt" << 'EOF'
Arch Linux Boot Configuration
==============================

Kernel: /kernels/vmlinuz-arch
Initramfs: /initrds/initramfs-arch.img
Rootfs: /rootfs/ (Arch Linux bootstrap)

Recommended kernel cmdline:
  root=/dev/ram0 rw console=ttyS0,115200 debug init=/usr/bin/bash

Alternative (if rootfs on disk):
  root=/dev/sda2 rw console=ttyS0,115200

For shell access:
  init=/usr/bin/bash (drops to bash shell)
  
For normal boot:
  init=/usr/lib/systemd/systemd (if systemd is installed)
EOF

# Create a README
cat > README-ARCH.md << 'EOF'
# Arch Linux Boot Setup

## Files
- **Kernel**: `esp/kernels/vmlinuz-arch`
- **Initramfs**: `esp/initrds/initramfs-arch.img`
- **Rootfs**: `esp/rootfs/` (extracted Arch bootstrap)

## Boot Configuration

The bootloader needs to be updated to reference the Arch kernel and initramfs.

Edit `bootloader/src/tui/distro_launcher.rs` and add:

```rust
KernelEntry {
    name: String::from("Arch Linux"),
    path: String::from("\\kernels\\vmlinuz-arch"),
    cmdline: String::from("root=/dev/ram0 rw console=ttyS0,115200 debug init=/usr/bin/bash"),
    initrd: Some(String::from("\\initrds\\initramfs-arch.img")),
},
```

## Building & Testing

1. Update the bootloader source (see above)
2. Rebuild: `./testing/build.sh`
3. Run in QEMU: `./testing/run.sh`
4. Select "Arch Linux" from the menu

## Expected Behavior

The kernel will boot with the initramfs, then drop to a bash shell.
You'll see kernel messages on the serial console (QEMU output).

## Notes

- The rootfs is currently in `esp/rootfs/` but not automatically mounted
- For a full boot, you'd need to create a disk image and install Arch there
- The initramfs provides a minimal environment
- Use `console=ttyS0,115200` to see output in QEMU's serial console
EOF

echo ""
echo "========================================"
echo "  Arch Linux Setup Complete!"
echo "========================================"
echo ""
echo "Files installed:"
echo "  Kernel:    $(cd "$KERNELS_DIR" && pwd)/vmlinuz-arch"
echo "  Initramfs: $(cd "$INITRD_DIR" && pwd)/initramfs-arch.img"
echo "  Rootfs:    $(cd "$ROOTFS_DIR" && pwd)/"
echo ""
echo "Next steps:"
echo ""
echo "1. Update bootloader configuration:"
echo "   Edit: bootloader/src/tui/distro_launcher.rs"
echo "   Add Arch Linux entry (see README-ARCH.md)"
echo ""
echo "2. Rebuild bootloader:"
echo "   ./testing/build.sh"
echo ""
echo "3. Test in QEMU:"
echo "   ./testing/run.sh"
echo ""
echo "See README-ARCH.md for detailed instructions."
echo ""

