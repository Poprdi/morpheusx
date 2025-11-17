#!/bin/bash
# Rebuild initramfs with complete Arch Linux rootfs
set -e

cd "$(dirname "$0")"
BASE_DIR="$(pwd)"

WORK_DIR="/tmp/morpheus-initrd-build"
INITRD_TARGET="$BASE_DIR/esp/initrds/initramfs-arch.img"
ROOTFS_DIR="$BASE_DIR/esp/rootfs"

echo "=================================="
echo "  Rebuilding Arch Linux Initramfs"
echo "=================================="
echo ""

# Check if rootfs exists
if [ ! -d "$ROOTFS_DIR" ]; then
    echo "Error: Rootfs not found at $ROOTFS_DIR"
    echo "Run ./install-arch.sh first"
    exit 1
fi

echo "This will pack the complete Arch rootfs into initramfs"
echo "Total size: $(sudo du -sh "$ROOTFS_DIR" | cut -f1)"
echo ""

# Clean work directory
rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR"

echo "Copying rootfs to work directory..."
sudo rsync -a --info=progress2 "$ROOTFS_DIR/" "$WORK_DIR/"
echo "✓ Rootfs copied"

# Ensure init exists and is executable
if [ ! -f "$WORK_DIR/usr/lib/systemd/systemd" ]; then
    echo "✗ systemd not found in rootfs"
    exit 1
fi

# Create init symlink to systemd
echo "Setting up init -> systemd..."
sudo ln -sf usr/lib/systemd/systemd "$WORK_DIR/init"
echo "✓ Init configured (systemd)"

# Create the initramfs
echo ""
echo "Packing initramfs (this may take a while)..."
cd "$WORK_DIR"
sudo find . -print0 | sudo cpio --null -o --format=newc 2>/dev/null | gzip -9 > "$INITRD_TARGET"

echo ""
echo "✓ Initramfs created: $INITRD_TARGET"
echo "  Size: $(du -h "$INITRD_TARGET" | cut -f1)"
echo ""
echo "This initramfs contains:"
echo "  - Complete Arch Linux system"
echo "  - systemd init (PID 1)"
echo "  - All installed packages"
echo "  - Full networking stack"
echo "  - Development tools"
echo ""
echo "The system will boot directly from initramfs (no pivot_root needed)"
echo ""

# Cleanup
cd "$BASE_DIR"
sudo rm -rf "$WORK_DIR"

echo "Done!"
echo ""

