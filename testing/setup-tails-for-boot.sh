#!/bin/bash
# Setup Tails ISO for bootloader testing
#
# This script:
# 1. Downloads Tails ISO (if not present)
# 2. Extracts kernel/initrd to ESP
# 3. Creates manifest file so scanner can find it
# 4. Creates simulated chunk structure for testing
#
# Note: Full chunked ISO download requires UEFI HTTP protocol support
# which isn't available in standard OVMF. This script provides a
# workaround for testing the boot flow.

set -e

cd "$(dirname "$0")"
BASE_DIR="$(pwd)"

ESP_DIR="$BASE_DIR/esp"
KERNELS_DIR="$ESP_DIR/kernels"
INITRD_DIR="$ESP_DIR/initrds"
MANIFEST_DIR="$ESP_DIR/morpheus/isos"
ISO_DIR="$ESP_DIR/.iso"
WORK_DIR="/tmp/morpheus-tails-setup"

TAILS_VERSION="7.3.1"
TAILS_ISO="tails-amd64-${TAILS_VERSION}.iso"
TAILS_URL="https://download.tails.net/tails/stable/tails-amd64-${TAILS_VERSION}/tails-amd64-${TAILS_VERSION}.iso"

echo "=========================================="
echo "  Tails ISO Boot Setup"
echo "=========================================="
echo ""
echo "This script prepares Tails for booting via Morpheus."
echo ""

# Create directories
mkdir -p "$KERNELS_DIR"
mkdir -p "$INITRD_DIR"
mkdir -p "$MANIFEST_DIR"
mkdir -p "$ISO_DIR"
mkdir -p "$WORK_DIR"

# Check if ISO already exists
ISO_PATH=""
if [ -f "$ISO_DIR/$TAILS_ISO" ]; then
    echo "✓ Found Tails ISO in ESP"
    ISO_PATH="$ISO_DIR/$TAILS_ISO"
elif [ -f "$WORK_DIR/$TAILS_ISO" ]; then
    echo "✓ Found Tails ISO in cache"
    ISO_PATH="$WORK_DIR/$TAILS_ISO"
else
    echo "Downloading Tails ${TAILS_VERSION}..."
    echo "URL: $TAILS_URL"
    echo "This may take a while (~1.3GB)..."
    
    if command -v curl &> /dev/null; then
        curl -L --progress-bar -o "$WORK_DIR/$TAILS_ISO" "$TAILS_URL"
    elif command -v wget &> /dev/null; then
        wget --progress=bar:force -O "$WORK_DIR/$TAILS_ISO" "$TAILS_URL"
    else
        echo "Error: Neither curl nor wget found."
        exit 1
    fi
    
    ISO_PATH="$WORK_DIR/$TAILS_ISO"
fi

# Verify ISO
ISO_SIZE=$(stat -c%s "$ISO_PATH" 2>/dev/null || echo 0)
if [ "$ISO_SIZE" -lt 500000000 ]; then
    echo "Error: ISO appears incomplete (${ISO_SIZE} bytes)"
    exit 1
fi

echo ""
echo "ISO size: $(numfmt --to=iec $ISO_SIZE)"

# Copy ISO to ESP
if [ ! -f "$ISO_DIR/$TAILS_ISO" ]; then
    echo "Copying ISO to ESP..."
    cp "$ISO_PATH" "$ISO_DIR/"
fi

# Mount ISO and extract kernel/initrd
echo ""
echo "Extracting kernel and initrd..."
ISO_MOUNT="$WORK_DIR/iso-mount"
mkdir -p "$ISO_MOUNT"

# Unmount if already mounted
if mountpoint -q "$ISO_MOUNT" 2>/dev/null; then
    sudo umount "$ISO_MOUNT" 2>/dev/null || true
fi

sudo mount -o loop,ro "$ISO_PATH" "$ISO_MOUNT"

if [ -f "$ISO_MOUNT/live/vmlinuz" ]; then
    echo "  Extracting kernel..."
    cp "$ISO_MOUNT/live/vmlinuz" "$KERNELS_DIR/vmlinuz-tails"
    
    echo "  Extracting initrd..."
    cp "$ISO_MOUNT/live/initrd.img" "$INITRD_DIR/initrd-tails.img"
    
    KERNEL_SIZE=$(stat -c%s "$KERNELS_DIR/vmlinuz-tails")
    INITRD_SIZE=$(stat -c%s "$INITRD_DIR/initrd-tails.img")
    echo "  Kernel: $(numfmt --to=iec $KERNEL_SIZE)"
    echo "  Initrd: $(numfmt --to=iec $INITRD_SIZE)"
else
    echo "Error: Cannot find kernel in ISO"
    sudo umount "$ISO_MOUNT"
    exit 1
fi

sudo umount "$ISO_MOUNT"

# Create a simple manifest file
# Note: This is a text-based manifest for the scanner to find
# The actual binary manifest format is created by the downloader
echo ""
echo "Creating manifest entry..."

# Create loader entry for direct boot (without chunking)
ENTRY_DIR="$ESP_DIR/loader/entries"
mkdir -p "$ENTRY_DIR"

cat > "$ENTRY_DIR/tails.conf" << EOF
title   Tails ${TAILS_VERSION} (Live ISO)
linux   \\kernels\\vmlinuz-tails
initrd  \\initrds\\initrd-tails.img
options boot=live nopersistence noprompt timezone=Etc/UTC splash noautologin module=Tails quiet
EOF

echo "✓ Created loader entry: $ENTRY_DIR/tails.conf"

# For chunked ISO testing, we'd need to create actual partitions
# This is handled by the bootloader's downloader in real usage
# For now, the loader entry allows direct kernel/initrd boot

echo ""
echo "=========================================="
echo "  Setup Complete!"
echo "=========================================="
echo ""
echo "Tails is now ready for booting:"
echo "  1. Run ./build.sh to update bootloader"
echo "  2. Run ./run.sh and select boot mode"
echo "  3. Select 'Tails ${TAILS_VERSION} (Live ISO)' from menu"
echo ""
echo "Files created:"
echo "  - $ISO_DIR/$TAILS_ISO"
echo "  - $KERNELS_DIR/vmlinuz-tails"
echo "  - $INITRD_DIR/initrd-tails.img"
echo "  - $ENTRY_DIR/tails.conf"
echo ""
