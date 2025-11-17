#!/bin/bash

set -e

cd "$(dirname "$0")"
BASE_DIR="$(pwd)"

ESP_DIR="$BASE_DIR/esp"
DISK_IMAGE="test-disk-50g.img"

echo "============================================"
echo "  Creating 50GB Test Disk with Boot Entries"
echo "============================================"
echo ""

if [ -f "$DISK_IMAGE" ]; then
    read -p "Disk image exists. Recreate? [y/N]: " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo "Using existing disk"
        exit 0
    fi
    rm -f "$DISK_IMAGE"
fi

echo "Creating 50GB sparse disk image..."
qemu-img create -f raw "$DISK_IMAGE" 50G

echo "Creating GPT partition table..."
parted -s "$DISK_IMAGE" mklabel gpt

echo "Creating ESP partition (512MB)..."
parted -s "$DISK_IMAGE" mkpart primary fat32 1MiB 513MiB
parted -s "$DISK_IMAGE" set 1 esp on

echo "Creating root partition (remaining space)..."
parted -s "$DISK_IMAGE" mkpart primary ext4 513MiB 100%

echo "Setting up loop device..."
LOOP_DEV=$(sudo losetup -fP --show "$DISK_IMAGE")
echo "Loop device: $LOOP_DEV"

cleanup() {
    echo "Cleaning up..."
    sudo umount /tmp/esp-mount 2>/dev/null || true
    sudo losetup -d "$LOOP_DEV" 2>/dev/null || true
}
trap cleanup EXIT

echo "Formatting ESP partition..."
sudo mkfs.vfat -F 32 -n "ESP" "${LOOP_DEV}p1"

echo "Formatting root partition..."
sudo mkfs.ext4 -L "MORPHEUS_ROOT" "${LOOP_DEV}p2"

echo "Mounting ESP..."
mkdir -p /tmp/esp-mount
sudo mount "${LOOP_DEV}p1" /tmp/esp-mount

echo "Creating directory structure..."
sudo mkdir -p /tmp/esp-mount/EFI/BOOT
sudo mkdir -p /tmp/esp-mount/kernels
sudo mkdir -p /tmp/esp-mount/initrds
sudo mkdir -p /tmp/esp-mount/loader/entries

echo "Copying bootloader..."
if [ -f "$ESP_DIR/EFI/BOOT/BOOTX64.EFI" ]; then
    sudo cp "$ESP_DIR/EFI/BOOT/BOOTX64.EFI" /tmp/esp-mount/EFI/BOOT/
else
    echo "WARNING: Bootloader not built yet"
fi

echo "Copying kernels and initrds..."
if [ -d "$ESP_DIR/kernels" ]; then
    sudo cp -r "$ESP_DIR/kernels/"* /tmp/esp-mount/kernels/ 2>/dev/null || true
fi
if [ -d "$ESP_DIR/initrds" ]; then
    sudo cp -r "$ESP_DIR/initrds/"* /tmp/esp-mount/initrds/ 2>/dev/null || true
fi

echo "Creating boot entries..."
for kernel in /tmp/esp-mount/kernels/vmlinuz-*; do
    if [ ! -f "$kernel" ]; then
        continue
    fi
    
    KERNEL_FILE=$(basename "$kernel")
    DISTRO=$(echo "$KERNEL_FILE" | sed 's/vmlinuz-//')
    
    case "$DISTRO" in
        tails)
            CMDLINE="boot=live live-media-path=/live nopersistence noprompt timezone=Etc/UTC splash=0 console=ttyS0,115200 console=tty0"
            TITLE="Tails OS"
            ;;
        ubuntu)
            CMDLINE="boot=casper quiet splash console=ttyS0,115200 console=tty0"
            TITLE="Ubuntu 24.04"
            ;;
        debian)
            CMDLINE="boot=live quiet console=ttyS0,115200 console=tty0"
            TITLE="Debian 12"
            ;;
        arch)
            CMDLINE="root=/dev/ram0 rw console=ttyS0,115200 console=tty0 debug"
            TITLE="Arch Linux"
            ;;
        fedora)
            CMDLINE="rd.live.image quiet console=ttyS0,115200 console=tty0"
            TITLE="Fedora"
            ;;
        kali)
            CMDLINE="boot=live quiet console=ttyS0,115200 console=tty0"
            TITLE="Kali Linux"
            ;;
        *)
            CMDLINE="console=ttyS0,115200 console=tty0"
            TITLE="$DISTRO"
            ;;
    esac
    
    INITRD_PATH="\\initrds\\initrd-${DISTRO}.img"
    if [ ! -f "/tmp/esp-mount/initrds/initrd-${DISTRO}.img" ]; then
        INITRD_PATH=""
    fi
    
    sudo tee "/tmp/esp-mount/loader/entries/${DISTRO}.conf" > /dev/null <<EOF
title   $TITLE
linux   \\kernels\\$KERNEL_FILE
$([ -n "$INITRD_PATH" ] && echo "initrd  $INITRD_PATH")
options $CMDLINE
EOF
    
    echo "  Created entry: $TITLE"
done

echo "Syncing filesystem..."
sudo sync

echo "Unmounting..."
sudo umount /tmp/esp-mount
sudo losetup -d "$LOOP_DEV"
trap - EXIT

echo ""
echo "============================================"
echo "  ✓ 50GB Test Disk Created Successfully!"
echo "============================================"
echo ""
echo "Disk image: $DISK_IMAGE"
echo "Partitions:"
echo "  1. ESP (512MB, FAT32) - /dev/sda1"
echo "  2. Root (49.5GB, ext4) - /dev/sda2"
echo ""
echo "Boot entries created in /loader/entries/"
echo ""
echo "To use:"
echo "  ./run.sh"
echo "  Select option to boot from this disk"
echo ""
