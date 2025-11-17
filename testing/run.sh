#!/bin/bash
# Run Morpheus in QEMU with OVMF firmware

set -e

cd "$(dirname "$0")"

# Check if bootloader is built
if [ ! -f esp/EFI/BOOT/BOOTX64.EFI ]; then
    echo "Error: Bootloader not built yet. Run ./build.sh first"
    exit 1
fi

# Always rebuild ESP disk image to ensure latest bootloader is used
echo "Creating ESP disk image from esp/ directory..."
# Calculate size needed (add 50MB overhead for FAT32 structures)
ESP_SIZE=$(du -sb esp | awk '{print int(($1 / 1024 / 1024) + 50)}')
echo "ESP directory size: ${ESP_SIZE}MB (with overhead)"

# Remove old image if exists
rm -f esp.img

# Create disk image
dd if=/dev/zero of=esp.img bs=1M count=$ESP_SIZE status=none

# Format as FAT32
mkfs.vfat -F 32 -n "ESP" esp.img >/dev/null

# Mount and copy contents (EXCLUDE rootfs - it's packed into initramfs)
mkdir -p /tmp/esp-mount
sudo mount -o loop esp.img /tmp/esp-mount
sudo rsync -a --exclude='rootfs' esp/ /tmp/esp-mount/ || true  # Ignore symlink errors from FAT32
sudo umount /tmp/esp-mount
rmdir /tmp/esp-mount

echo "✓ ESP image created: esp.img (${ESP_SIZE}MB)"

echo "Starting QEMU with OVMF..."
echo "Press Ctrl+A then X to exit QEMU"
echo ""

# Create test disk images if they don't exist
if [ ! -f test-disk.img ]; then
    echo "Creating small test disk (100MB) with GPT..."
    dd if=/dev/zero of=test-disk.img bs=1M count=100 status=none
    parted -s test-disk.img mklabel gpt
    parted -s test-disk.img mkpart primary fat32 1MiB 50MiB
    parted -s test-disk.img set 1 esp on
    parted -s test-disk.img mkpart primary ext4 50MiB 99MiB
    echo "Small test disk created"
fi

if [ ! -f test-disk-10g.img ]; then
    echo "Creating large test disk (10GB) - empty for partition testing..."
    qemu-img create -f raw test-disk-10g.img 10G
    echo "Large test disk created (no partition table - test creating GPT from bootloader)"
fi

# Prompt for boot mode
echo ""
echo "Select boot mode:"
echo "  [1] Normal - ESP + both test disks (default)"
echo "  [2] Persistence test - ONLY 10GB disk (tests if bootloader installed)"
echo "  [3] ESP only - For development/testing"
read -p "Choice [1/2/3]: " -n 1 -r
echo ""

case $REPLY in
    2)
        echo "Booting ONLY test-disk-10g.img (persistence test)..."
        echo "If Morpheus boots, installation succeeded!"
        echo ""
        qemu-system-x86_64 \
            -s \
            -bios /usr/share/edk2/ovmf/OVMF_CODE.fd \
            -drive format=raw,file=test-disk-10g.img \
            -net none \
            -m 4096M \
            -serial stdio
        ;;
    3)
        echo "Booting from ESP directory only..."
        echo ""
        qemu-system-x86_64 \
            -s \
            -bios /usr/share/edk2/ovmf/OVMF_CODE.fd \
            -drive format=raw,file=esp.img \
            -net none \
            -m 4096M \
            -serial stdio
        ;;
    *)
        echo "Booting with ESP + both test disks..."
        echo ""
        qemu-system-x86_64 \
            -s \
            -bios /usr/share/edk2/ovmf/OVMF_CODE.fd \
            -drive format=raw,file=esp.img \
            -drive format=raw,file=test-disk.img \
            -drive format=raw,file=test-disk-10g.img \
            -net none \
            -m 4096M \
            -serial stdio
        ;;
esac
