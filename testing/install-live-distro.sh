#!/bin/bash
# Install a live Linux distribution for Morpheus bootloader
# Supports Ubuntu, Debian, Fedora, and Tails

set -e

cd "$(dirname "$0")"
BASE_DIR="$(pwd)"

ESP_DIR="$BASE_DIR/esp"
KERNELS_DIR="$ESP_DIR/kernels"
INITRD_DIR="$ESP_DIR/initrds"
WORK_DIR="/tmp/morpheus-live-setup"

echo "=========================================="
echo "  Live Linux Distribution Installer"
echo "=========================================="
echo ""
echo "Select a distribution to install:"
echo "  [1] Ubuntu 24.04 Desktop (Full GUI, easy to use) - 5.7GB"
echo "  [2] Debian 12 Live (Lightweight, reliable) - 3.1GB"
echo "  [3] Tails 6.9 (Privacy-focused, fully-featured) - 1.3GB"
echo "  [4] Fedora 40 Workstation (Cutting-edge, complete) - 2.3GB"
echo "  [5] Kali Linux (Pentesting tools, networking) - 4.1GB"
echo ""
read -p "Choice [1-5]: " -n 1 -r
echo ""

case $REPLY in
    1)
        DISTRO_NAME="Ubuntu 24.04"
        ISO_NAME="ubuntu-24.04.1-desktop-amd64.iso"
        ISO_URL="https://releases.ubuntu.com/24.04/${ISO_NAME}"
        KERNEL_PATH="casper/vmlinuz"
        INITRD_PATH="casper/initrd"
        SQUASHFS_PATH="casper/filesystem.squashfs"
        KERNEL_PARAMS="boot=casper quiet splash console=ttyS0,115200"
        ;;
    2)
        DISTRO_NAME="Debian 12"
        ISO_NAME="debian-live-12.8.0-amd64-standard.iso"
        ISO_URL="https://cdimage.debian.org/debian-cd/current-live/amd64/iso-hybrid/${ISO_NAME}"
        KERNEL_PATH="live/vmlinuz-*"
        INITRD_PATH="live/initrd.img-*"
        SQUASHFS_PATH="live/filesystem.squashfs"
        KERNEL_PARAMS="boot=live quiet console=ttyS0,115200"
        ;;
    3)
        DISTRO_NAME="Tails 6.9"
        ISO_NAME="tails-amd64-6.9.iso"
        ISO_URL="https://mirrors.edge.kernel.org/tails/stable/${ISO_NAME}"
        ISO_URL_FALLBACK="https://mirrors.wikimedia.org/tails/stable/${ISO_NAME}"
        KERNEL_PATH="live/vmlinuz"
        INITRD_PATH="live/initrd.img"
        SQUASHFS_PATH="live/filesystem.squashfs"
        KERNEL_PARAMS="boot=live live-media-path=/live nopersistence noprompt timezone=Etc/UTC splash=0 console=ttyS0,115200"
        ;;
    4)
        DISTRO_NAME="Fedora 40"
        ISO_NAME="Fedora-Workstation-Live-x86_64-40-1.14.iso"
        ISO_URL="https://download.fedoraproject.org/pub/fedora/linux/releases/40/Workstation/x86_64/iso/${ISO_NAME}"
        KERNEL_PATH="isolinux/vmlinuz"
        INITRD_PATH="isolinux/initrd.img"
        SQUASHFS_PATH="LiveOS/squashfs.img"
        KERNEL_PARAMS="root=live:CDLABEL=Fedora-WS-Live-40-1-14 rd.live.image quiet console=ttyS0,115200"
        ;;
    5)
        DISTRO_NAME="Kali Linux"
        ISO_NAME="kali-linux-2024.4-live-amd64.iso"
        ISO_URL="https://cdimage.kali.org/kali-2024.4/${ISO_NAME}"
        KERNEL_PATH="live/vmlinuz"
        INITRD_PATH="live/initrd.img"
        SQUASHFS_PATH="live/filesystem.squashfs"
        KERNEL_PARAMS="boot=live quiet console=ttyS0,115200"
        ;;
    *)
        echo "Invalid choice"
        exit 1
        ;;
esac

echo ""
echo "=========================================="
echo "  Installing: $DISTRO_NAME"
echo "=========================================="
echo ""

# Create directories
echo "Setting up workspace..."
mkdir -p "$KERNELS_DIR"
mkdir -p "$INITRD_DIR"
mkdir -p "$WORK_DIR"

# Download ISO
echo ""
echo "Downloading $DISTRO_NAME ISO..."
echo "This may take a while depending on the size..."
if [ ! -f "$WORK_DIR/$ISO_NAME" ]; then
    if ! wget -c -O "$WORK_DIR/$ISO_NAME" "$ISO_URL" 2>/dev/null; then
        if [ -n "$ISO_URL_FALLBACK" ]; then
            echo "Primary mirror failed, trying fallback..."
            wget -c -O "$WORK_DIR/$ISO_NAME" "$ISO_URL_FALLBACK"
        else
            echo "Download failed"
            exit 1
        fi
    fi
else
    echo "ISO already downloaded, using cached version"
fi

# Mount ISO and extract kernel/initrd
echo ""
echo "Extracting kernel and initrd from ISO..."
ISO_MOUNT="$WORK_DIR/iso-mount"
mkdir -p "$ISO_MOUNT"

# Mount the ISO
sudo mount -o loop,ro "$WORK_DIR/$ISO_NAME" "$ISO_MOUNT"

# Extract kernel (handle wildcards)
KERNEL_FILE=$(ls "$ISO_MOUNT/$KERNEL_PATH" 2>/dev/null | head -n1)
if [ -n "$KERNEL_FILE" ] && [ -f "$KERNEL_FILE" ]; then
    echo "  Copying kernel..."
    DISTRO_SLUG=$(echo "$DISTRO_NAME" | tr '[:upper:]' '[:lower:]' | tr ' ' '-' | cut -d'-' -f1)
    cp "$KERNEL_FILE" "$KERNELS_DIR/vmlinuz-${DISTRO_SLUG}"
else
    echo "Error: Could not find kernel at $KERNEL_PATH"
    sudo umount "$ISO_MOUNT"
    exit 1
fi

# Extract initrd (handle wildcards)
INITRD_FILE=$(ls "$ISO_MOUNT/$INITRD_PATH" 2>/dev/null | head -n1)
if [ -n "$INITRD_FILE" ] && [ -f "$INITRD_FILE" ]; then
    echo "  Copying initrd..."
    cp "$INITRD_FILE" "$INITRD_DIR/initrd-${DISTRO_SLUG}.img"
else
    echo "Error: Could not find initrd at $INITRD_PATH"
    sudo umount "$ISO_MOUNT"
    exit 1
fi

# Extract squashfs if available
SQUASHFS_FILE=$(ls "$ISO_MOUNT/$SQUASHFS_PATH" 2>/dev/null | head -n1)
if [ -n "$SQUASHFS_FILE" ] && [ -f "$SQUASHFS_FILE" ]; then
    echo "  Copying SquashFS filesystem (this may take a moment)..."
    cp "$SQUASHFS_FILE" "$INITRD_DIR/filesystem-${DISTRO_SLUG}.squashfs"
fi

# Unmount ISO
sudo umount "$ISO_MOUNT"

echo ""
echo "=========================================="
echo "  ✓ $DISTRO_NAME Installation Complete!"
echo "=========================================="
echo ""
echo "Installed files:"
echo "  • Kernel: $KERNELS_DIR/vmlinuz-${DISTRO_SLUG}"
echo "  • Initrd: $INITRD_DIR/initrd-${DISTRO_SLUG}.img"
if [ -n "$SQUASHFS_FILE" ] && [ -f "$SQUASHFS_FILE" ]; then
    echo "  • RootFS: $INITRD_DIR/filesystem-${DISTRO_SLUG}.squashfs"
fi
echo ""
echo "Kernel boot parameters:"
echo "  $KERNEL_PARAMS"
echo ""
echo "Next steps:"
echo "  1. Update your bootloader configuration to use these files"
echo "  2. Set the kernel parameters above"
echo "  3. Run ./build.sh && ./run.sh"
echo ""
echo "Note: These live systems include:"
echo "  ✓ Full networking (DHCP, NetworkManager)"
echo "  ✓ Text editors (nano, vim)"
echo "  ✓ Package managers (apt/dnf/pacman)"
echo "  ✓ All hardware drivers and firmware"
echo "  ✓ Complete userland tools"
echo ""
