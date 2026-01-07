#!/bin/bash
# Build script for Morpheus bootloader
# Always performs a clean build to avoid stale/cached artifacts

set -e

cd "$(dirname "$0")/.."

echo "========================================"
echo "  MorpheusX Clean Build"
echo "========================================"
echo ""

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

# =============================================================================
# FULL CLEAN BUILD - Remove ALL cached artifacts
# =============================================================================
echo ""
echo "Step 1: Cleaning ALL build artifacts..."
echo "  - Removing target directory entirely"
rm -rf target/

# Also clean any stale fingerprints that might survive cargo clean
echo "  - Build cache cleared"
echo ""

# =============================================================================
# PASS 1: Build bootloader to get binary for reloc extraction
# =============================================================================
echo "Step 2: Building bootloader (pass 1)..."
cargo build --target x86_64-unknown-uefi -p morpheus-bootloader --release

# Extract relocation data from the built binary
echo ""
echo "Step 3: Extracting relocation metadata..."
./tools/extract-reloc-data.sh

# =============================================================================
# PASS 2: Rebuild with correct embedded relocation data
# =============================================================================
echo ""
echo "Step 4: Building bootloader (pass 2 with reloc data)..."
# Clean just the bootloader to force rebuild with new reloc data
cargo clean -p morpheus-bootloader
cargo build --target x86_64-unknown-uefi -p morpheus-bootloader --release

# Rebuild initrd if rootfs exists
if [ -d "testing/esp/rootfs" ]; then
    echo ""
    echo "Step 5: Rebuilding initramfs from rootfs..."
    cd testing
    ./rebuild-initrd.sh
    cd ..
else
    echo ""
    echo "Note: No rootfs found, skipping initrd rebuild"
    echo "Run './testing/install-arch.sh' or './testing/create-minimal-arch.sh' first"
fi

# Copy to test ESP
echo ""
echo "Step 6: Deploying to test ESP..."
cp target/x86_64-unknown-uefi/release/morpheus-bootloader.efi testing/esp/EFI/BOOT/BOOTX64.EFI

echo ""
echo "========================================"
echo "  Build Complete!"
echo "========================================"
echo "✓ Built: testing/esp/EFI/BOOT/BOOTX64.EFI"
echo "✓ Relocation data embedded in binary"
if [ -f "testing/esp/initrds/initramfs-arch.img" ]; then
    echo "✓ Initramfs: testing/esp/initrds/initramfs-arch.img"
fi
echo ""
echo "Run './testing/run.sh' to test in QEMU"
