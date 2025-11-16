#!/bin/bash
# Build script for Morpheus bootloader

set -e

cd "$(dirname "$0")/.."

echo "Building Morpheus bootloader for x86_64 UEFI..."

# Prompt to clean QEMU disk images
if [ -f testing/test-disk.img ] || [ -f testing/test-disk-10g.img ]; then
    echo ""
    echo "QEMU disk images exist. Delete them for a fresh start?"
    echo "  [y] Yes - delete disks (fresh install, lose any test data)"
    echo "  [n] No - keep disks (preserve partitions and data)"
    read -p "Delete disk images? [y/N]: " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        rm -f testing/test-disk.img testing/test-disk-10g.img
        echo "✓ Disk images deleted"
    else
        echo "✓ Keeping existing disk images"
    fi
fi

# Install rust target if not present
rustup target add x86_64-unknown-uefi 2>/dev/null || true

# Clean build cache to ensure fresh build
echo "Cleaning build cache..."
cargo clean

# Build (first pass to get binary for reloc extraction)
echo "Building bootloader (pass 1)..."
cargo build --target x86_64-unknown-uefi -p morpheus-bootloader --release

# Extract relocation data from the built binary
echo "Extracting relocation metadata..."
./tools/extract-reloc-data.sh

# Rebuild with correct embedded relocation data
echo "Building bootloader (pass 2 with correct reloc data)..."
cargo build --target x86_64-unknown-uefi -p morpheus-bootloader --release

# Copy to test ESP
echo "Deploying to test ESP..."
cp target/x86_64-unknown-uefi/release/morpheus-bootloader.efi testing/esp/EFI/BOOT/BOOTX64.EFI

echo ""
echo "✓ Built successfully: testing/esp/EFI/BOOT/BOOTX64.EFI"
echo "✓ Relocation data is hardcoded in the binary"
echo "Run './testing/run.sh' to test in QEMU"
