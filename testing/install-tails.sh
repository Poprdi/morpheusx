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
ISO_TARGET_DIR="$ESP_DIR/.iso"

echo "=========================================="
echo "  Tails OS Live System Installer"
echo "=========================================="
echo ""
echo "This will download and extract Tails OS ${TAILS_VERSION:-7.3.1} live system:"
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
mkdir -p "$ISO_TARGET_DIR"
mkdir -p "$WORK_DIR"

# Download Tails ISO
# Using version 7.3.1 (Dec 2025) - current stable release
TAILS_VERSION="7.3.1"
TAILS_ISO="tails-amd64-${TAILS_VERSION}.iso"
# Tails official download structure: https://download.tails.net/tails/stable/
TAILS_URL="https://download.tails.net/tails/stable/tails-amd64-${TAILS_VERSION}/tails-amd64-${TAILS_VERSION}.iso"
TAILS_URL_FALLBACK="https://mirrors.edge.kernel.org/tails/stable/tails-amd64-${TAILS_VERSION}/tails-amd64-${TAILS_VERSION}.iso"

echo ""
echo "Downloading Tails ${TAILS_VERSION}..."
echo "This may take a while (1.3GB)..."
echo "URL: $TAILS_URL"

MIN_ISO_SIZE=$((500 * 1024 * 1024))

download_iso() {
    rm -f "$WORK_DIR/$TAILS_ISO"
    if command -v curl &> /dev/null; then
        if ! curl -L --fail --progress-bar -o "$WORK_DIR/$TAILS_ISO" "$TAILS_URL"; then
            echo "Primary mirror failed, trying fallback..."
            echo "Fallback URL: $TAILS_URL_FALLBACK"
            curl -L --fail --progress-bar -o "$WORK_DIR/$TAILS_ISO" "$TAILS_URL_FALLBACK"
        fi
    elif command -v wget &> /dev/null; then
        if ! wget --progress=bar:force --max-redirect=5 -O "$WORK_DIR/$TAILS_ISO" "$TAILS_URL"; then
            echo "Primary mirror failed, trying fallback..."
            wget --progress=bar:force --max-redirect=5 -O "$WORK_DIR/$TAILS_ISO" "$TAILS_URL_FALLBACK"
        fi
    else
        echo "Error: Neither curl nor wget found."
        exit 1
    fi
}

# Check if we already have the ISO
SKIP_DOWNLOAD=false

# Check final destination first (ESP .iso/ directory)
if [ -f "$ISO_TARGET_DIR/$TAILS_ISO" ]; then
    ISO_SIZE=$(stat -c%s "$ISO_TARGET_DIR/$TAILS_ISO" 2>/dev/null || echo 0)
    if [ "$ISO_SIZE" -ge "$MIN_ISO_SIZE" ]; then
        echo "ISO already in ESP .iso/ directory ($(numfmt --to=iec $ISO_SIZE))"
        # Check if running interactively (user can respond to prompts)
        if [ -t 0 ]; then
            read -p "Re-download? [y/N]: " -n 1 -r
            echo
            if [[ ! $REPLY =~ ^[Yy]$ ]]; then
                SKIP_DOWNLOAD=true
                echo "Using existing ISO from ESP"
                # Copy to work dir for mounting
                cp "$ISO_TARGET_DIR/$TAILS_ISO" "$WORK_DIR/$TAILS_ISO"
            fi
        else
            SKIP_DOWNLOAD=true
            echo "Using existing ISO from ESP (non-interactive mode)"
            # Copy to work dir for mounting
            cp "$ISO_TARGET_DIR/$TAILS_ISO" "$WORK_DIR/$TAILS_ISO"
        fi
    fi
# Check cache directory second
elif [ -f "$WORK_DIR/$TAILS_ISO" ]; then
    ISO_SIZE=$(stat -c%s "$WORK_DIR/$TAILS_ISO" 2>/dev/null || echo 0)
    if [ "$ISO_SIZE" -ge "$MIN_ISO_SIZE" ]; then
        echo "ISO already cached in temp ($(numfmt --to=iec $ISO_SIZE))"
        # Check if running interactively (user can respond to prompts)
        if [ -t 0 ]; then
            read -p "Re-download? [y/N]: " -n 1 -r
            echo
            if [[ ! $REPLY =~ ^[Yy]$ ]]; then
                SKIP_DOWNLOAD=true
                echo "Using cached ISO"
            fi
        else
            SKIP_DOWNLOAD=true
            echo "Using cached ISO (non-interactive mode)"
        fi
    else
        echo "Cached ISO is corrupted (too small: ${ISO_SIZE} bytes). Re-downloading..."
    fi
fi

if [ "$SKIP_DOWNLOAD" = false ]; then
    download_iso
fi

ISO_SIZE=$(stat -c%s "$WORK_DIR/$TAILS_ISO" 2>/dev/null || echo 0)
if [ "$ISO_SIZE" -lt "$MIN_ISO_SIZE" ]; then
    echo "Error: Downloaded ISO is too small (${ISO_SIZE} bytes). Download failed."
    rm -f "$WORK_DIR/$TAILS_ISO"
    exit 1
fi

# Copy ISO to ESP .iso/ for bootloader ISO boot
echo ""
echo "Copying ISO to ESP .iso/ directory for bootloader discovery..."
cp "$WORK_DIR/$TAILS_ISO" "$ISO_TARGET_DIR/"
echo "  Added: $ISO_TARGET_DIR/$TAILS_ISO"

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
echo "  • ISO:    $ISO_TARGET_DIR/$TAILS_ISO"
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
