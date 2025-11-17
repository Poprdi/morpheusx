#!/bin/bash
# Install Tails OS live system for Morpheus bootloader
# This gives us a full-featured Linux with networking, GUI, and all tools

set -e

cd "$(dirname "$0")"
BASE_DIR="$(pwd)"

ESP_DIR="$BASE_DIR/esp"
KERNELS_DIR="$ESP_DIR/kernels"
INITRD_DIR="$ESP_DIR/initrds"
WORK_DIR="/tmp/morpheus-tails-setup"

echo "=========================================="
echo "  Tails OS Live System Installer"
echo "=========================================="
echo ""
echo "This will download and extract Tails OS 7.2 live system:"
echo "  ✓ Full Debian-based userland"
echo "  ✓ Complete networking stack (Tor ready)"
echo "  ✓ Desktop environment (GNOME)"
echo "  ✓ Full suite of tools (nano, vim, browsers, etc.)"
echo "  ✓ Hardware drivers and firmware"
echo ""
echo "Download size: ~1.3GB"
echo "Extracted size: ~2GB"
echo ""
read -p "Continue? [Y/n]: " -n 1 -r
echo
if [[ $REPLY =~ ^[Nn]$ ]]; then
    echo "Aborted."
    exit 0
fi

# Create directories
echo ""
echo "Setting up workspace..."
mkdir -p "$KERNELS_DIR"
mkdir -p "$INITRD_DIR"
mkdir -p "$WORK_DIR"

# Download Tails ISO
# Using version 7.2 (Nov 2025) - current stable release
TAILS_VERSION="7.2"
TAILS_ISO="tails-amd64-${TAILS_VERSION}.iso"
# Tails official download structure: https://download.tails.net/tails/stable/
TAILS_URL="https://download.tails.net/tails/stable/tails-amd64-${TAILS_VERSION}/tails-amd64-${TAILS_VERSION}.iso"
TAILS_URL_FALLBACK="https://mirrors.wikimedia.org/tails/stable/tails-amd64-${TAILS_VERSION}/tails-amd64-${TAILS_VERSION}.iso"

echo ""
echo "Downloading Tails ${TAILS_VERSION}..."
echo "This may take a while (1.3GB)..."
echo "URL: $TAILS_URL"
if [ ! -f "$WORK_DIR/$TAILS_ISO" ]; then
    # Try with curl first (better redirect handling)
    if command -v curl &> /dev/null; then
        if ! curl -L --progress-bar -C - -o "$WORK_DIR/$TAILS_ISO" "$TAILS_URL"; then
            echo "Primary mirror failed, trying fallback..."
            echo "Fallback URL: $TAILS_URL_FALLBACK"
            curl -L --progress-bar -C - -o "$WORK_DIR/$TAILS_ISO" "$TAILS_URL_FALLBACK"
        fi
    # Fallback to wget with proper redirect handling
    elif command -v wget &> /dev/null; then
        if ! wget --progress=bar:force --max-redirect=5 -c -O "$WORK_DIR/$TAILS_ISO" "$TAILS_URL"; then
            echo "Primary mirror failed, trying fallback..."
            echo "Fallback URL: $TAILS_URL_FALLBACK"
            wget --progress=bar:force --max-redirect=5 -c -O "$WORK_DIR/$TAILS_ISO" "$TAILS_URL_FALLBACK"
        fi
    else
        echo "Error: Neither curl nor wget found. Please install one of them."
        exit 1
    fi
else
    echo "ISO already downloaded, using cached version"
fi

# Mount ISO and extract kernel/initrd
echo ""
echo "Extracting kernel and initrd from Tails ISO..."
ISO_MOUNT="$WORK_DIR/iso-mount"
mkdir -p "$ISO_MOUNT"

# Unmount if already mounted (from previous run)
if mountpoint -q "$ISO_MOUNT" 2>/dev/null; then
    echo "  Unmounting previously mounted ISO..."
    sudo umount "$ISO_MOUNT" 2>/dev/null || true
fi

# Mount the ISO
echo "  Mounting ISO..."
sudo mount -o loop,ro "$WORK_DIR/$TAILS_ISO" "$ISO_MOUNT"

# Tails stores kernel and initrd in /live/ directory
if [ -f "$ISO_MOUNT/live/vmlinuz" ]; then
    echo "  Copying kernel (vmlinuz)..."
    rm -f "$KERNELS_DIR/vmlinuz-tails"
    cp "$ISO_MOUNT/live/vmlinuz" "$KERNELS_DIR/vmlinuz-tails"
    
    echo "  Copying initrd (initrd.img)..."
    rm -f "$INITRD_DIR/initrd-tails.img"
    cp "$ISO_MOUNT/live/initrd.img" "$INITRD_DIR/initrd-tails.img"
    
    # Get kernel version if possible
    if [ -d "$ISO_MOUNT/live/filesystem.squashfs" ] || [ -f "$ISO_MOUNT/live/filesystem.squashfs" ]; then
        echo "  Found SquashFS filesystem"
    fi
else
    echo "Error: Could not find Tails kernel/initrd in expected location"
    sudo umount "$ISO_MOUNT"
    exit 1
fi

# Copy the entire SquashFS filesystem (Tails uses this as the root filesystem)
if [ -f "$ISO_MOUNT/live/filesystem.squashfs" ]; then
    echo "  Copying SquashFS root filesystem (this may take a moment)..."
    rm -f "$INITRD_DIR/filesystem.squashfs"
    cp "$ISO_MOUNT/live/filesystem.squashfs" "$INITRD_DIR/filesystem.squashfs"
fi

# Unmount ISO
sudo umount "$ISO_MOUNT"

echo ""
echo "=========================================="
echo "  ✓ Tails OS Installation Complete!"
echo "=========================================="
echo ""
echo "Installed files:"
echo "  • Kernel: $KERNELS_DIR/vmlinuz-tails"
echo "  • Initrd: $INITRD_DIR/initrd-tails.img"
echo "  • RootFS: $INITRD_DIR/filesystem.squashfs"
echo ""
echo "Kernel boot parameters needed:"
echo "  boot=live"
echo "  live-media-path=/live"
echo "  nopersistence"
echo "  noprompt"
echo "  timezone=Etc/UTC"
echo "  splash=0"
echo "  console=ttyS0,115200"
echo ""
echo "Next steps:"
echo "  1. Update your bootloader to load these files"
echo "  2. Pass the kernel parameters above"
echo "  3. Run ./build.sh && ./run.sh"
echo ""
