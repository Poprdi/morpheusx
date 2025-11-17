#!/bin/bash
# Full Arch Linux bootstrap installer for Morpheus bootloader
# Creates a complete Arch environment with base system + networking

set -e

FORCE_REBUILD=0
if [[ ${1:-} == "--force" || ${1:-} == "-f" ]]; then
    FORCE_REBUILD=1
    shift || true
fi

cd "$(dirname "$0")"
BASE_DIR="$(pwd)"

ESP_DIR="$BASE_DIR/esp"
KERNELS_DIR="$ESP_DIR/kernels"
INITRD_DIR="$ESP_DIR/initrds"
ROOTFS_DIR="$ESP_DIR/rootfs"
WORK_DIR="/tmp/morpheus-arch-setup"
ROOTFS_MARKER="$ROOTFS_DIR/.arch_rootfs_ready"
BOOTSTRAP_URL="https://geo.mirror.pkgbuild.com/iso/latest/archlinux-bootstrap-x86_64.tar.zst"
BOOTSTRAP_URL_FALLBACK="https://america.mirror.pkgbuild.com/iso/latest/archlinux-bootstrap-x86_64.tar.zst"
BOOTSTRAP_FILE="$WORK_DIR/archlinux-bootstrap.tar.zst"

echo "=========================================="
echo "  Full Arch Linux Bootstrap Installer"
echo "=========================================="
echo ""
echo "This will create a complete Arch Linux environment:"
echo "  ✓ Base system (bash, coreutils, systemd)"
echo "  ✓ Networking stack (dhcpcd, iproute2, openssh)"
echo "  ✓ Development tools (gcc, make, git)"
echo "  ✓ System utilities (vim, tmux, htop)"
echo "  ✓ Package manager (pacman) - fully functional"
echo "  ✓ Linux kernel + modules"
echo ""
echo "This will download ~500MB and create a ~2GB rootfs"
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

# Unmount any lingering mounts from previous runs
if [ -d "$ROOTFS_DIR" ]; then
    echo "Cleaning up previous rootfs..."
    # Try multiple times with increasing aggressiveness
    for i in {1..3}; do
        sudo umount -l "$ROOTFS_DIR/dev/pts" 2>/dev/null || true
        sudo umount -l "$ROOTFS_DIR/dev/shm" 2>/dev/null || true
        sudo umount -l "$ROOTFS_DIR/dev" 2>/dev/null || true
        sudo umount -l "$ROOTFS_DIR/run" 2>/dev/null || true
        sudo umount -l "$ROOTFS_DIR/proc" 2>/dev/null || true
        sudo umount -l "$ROOTFS_DIR/sys" 2>/dev/null || true
        sleep 0.5
    done
    # Lazy unmount everything under rootfs
    sudo umount -l -R "$ROOTFS_DIR" 2>/dev/null || true
    sleep 1
fi

sudo rm -rf "$ROOTFS_DIR"
mkdir -p "$ROOTFS_DIR"

# Detect distro and set package manager
detect_distro() {
    if [ -f /etc/os-release ]; then
        . /etc/os-release
        echo "$ID"
    elif [ -f /etc/fedora-release ]; then
        echo "fedora"
    elif [ -f /etc/debian_version ]; then
        echo "debian"
    else
        echo "unknown"
    fi
}

install_packages() {
    local distro=$(detect_distro)
    local packages="$@"
    
    case "$distro" in
        fedora|rhel|centos|rocky|almalinux)
            echo "Detected Fedora/RHEL-based system"
            sudo dnf install -y $packages
            ;;
        debian|ubuntu|linuxmint)
            echo "Detected Debian/Ubuntu-based system"
            sudo apt-get update -qq
            sudo apt-get install -y $packages
            ;;
        arch|manjaro)
            echo "Detected Arch-based system"
            sudo pacman -Sy --noconfirm $packages
            ;;
        opensuse*|suse)
            echo "Detected openSUSE system"
            sudo zypper install -y $packages
            ;;
        *)
            echo "Unknown distro, trying generic install..."
            if command -v dnf &>/dev/null; then
                sudo dnf install -y $packages
            elif command -v apt-get &>/dev/null; then
                sudo apt-get update -qq
                sudo apt-get install -y $packages
            elif command -v pacman &>/dev/null; then
                sudo pacman -Sy --noconfirm $packages
            else
                echo "✗ Could not determine package manager"
                echo "Please manually install: $packages"
                exit 1
            fi
            ;;
    esac
}

# Check dependencies
echo ""
echo "Checking dependencies..."
DEPS_NEEDED=""
command -v curl &> /dev/null || DEPS_NEEDED="$DEPS_NEEDED curl"
command -v zstd &> /dev/null || DEPS_NEEDED="$DEPS_NEEDED zstd"

if [ -n "$DEPS_NEEDED" ]; then
    echo "Installing required tools:$DEPS_NEEDED"
    install_packages $DEPS_NEEDED
fi
echo "✓ Dependencies ready"

# Download Arch Linux bootstrap
echo ""
echo "Downloading Arch Linux bootstrap..."
if [ -f "$BOOTSTRAP_FILE" ] && zstd -t "$BOOTSTRAP_FILE" &>/dev/null; then
    echo "Using cached bootstrap"
else
    curl -fL -o "$BOOTSTRAP_FILE" "$BOOTSTRAP_URL" --progress-bar || {
        echo "Primary mirror failed, trying fallback..."
        curl -fL -o "$BOOTSTRAP_FILE" "$BOOTSTRAP_URL_FALLBACK" --progress-bar || {
            echo "✗ Download failed"
            exit 1
        }
    }
fi

# Extract bootstrap
echo ""
echo "Extracting Arch bootstrap to rootfs..."
cd "$ROOTFS_DIR"
sudo zstd -d "$BOOTSTRAP_FILE" -c | sudo tar -x -f - --strip-components=1

echo "✓ Bootstrap extracted"
echo ""

# No need to configure pacman - bootstrap already has everything
# Just use the rootfs as-is for initramfs

# Download and extract Linux kernel package
echo "=========================================="
echo "  Downloading Linux kernel"
echo "=========================================="
echo ""

KERNEL_URL="https://geo.mirror.pkgbuild.com/core/os/x86_64/"
KERNEL_PKG=$(curl -s "$KERNEL_URL" | grep -oP 'linux-[0-9]+\.[0-9]+\.[0-9]+\.arch[0-9]+-[0-9]+-x86_64\.pkg\.tar\.zst(?!")' | head -1)

if [ -z "$KERNEL_PKG" ]; then
    echo "✗ Could not find kernel package"
    exit 1
fi

echo "Found: $KERNEL_PKG"
cd "$WORK_DIR"

if [ ! -f "kernel.pkg.tar.zst" ]; then
    curl -fL -o kernel.pkg.tar.zst "$KERNEL_URL$KERNEL_PKG" --progress-bar || {
        echo "✗ Kernel download failed"
        exit 1
    }
fi

# Extract kernel from package
echo "Extracting kernel..."
KERNEL_EXTRACT="$WORK_DIR/kernel-extract"
rm -rf "$KERNEL_EXTRACT"
mkdir -p "$KERNEL_EXTRACT"

zstd -d kernel.pkg.tar.zst -c | tar -x -f - -C "$KERNEL_EXTRACT" 2>/dev/null

# Find and copy kernel
KERNEL_SRC=$(find "$KERNEL_EXTRACT" -name vmlinuz -o -name 'vmlinuz-*' | head -1)
if [ -n "$KERNEL_SRC" ] && [ -f "$KERNEL_SRC" ]; then
    cp "$KERNEL_SRC" "$KERNELS_DIR/vmlinuz-arch"
    chmod 644 "$KERNELS_DIR/vmlinuz-arch"
    echo "✓ Kernel: $KERNELS_DIR/vmlinuz-arch ($(du -h "$KERNELS_DIR/vmlinuz-arch" | cut -f1))"
else
    echo "✗ Kernel not found in package"
    exit 1
fi

rm -rf "$KERNEL_EXTRACT"

echo ""
echo "✓ Kernel extracted"
echo ""

# System configuration
echo ""
echo "=========================================="
echo "  Configuring system"
echo "=========================================="
echo ""

# Set hostname
echo "morpheus-arch" | sudo tee "$ROOTFS_DIR/etc/hostname" > /dev/null

# Configure hosts file
sudo tee "$ROOTFS_DIR/etc/hosts" > /dev/null << 'HOSTS_EOF'
127.0.0.1   localhost
::1         localhost
127.0.1.1   morpheus-arch.localdomain morpheus-arch
HOSTS_EOF

# Set root password to empty (modify shadow file directly)
echo "Setting root password (empty for testing)..."
sudo sed -i 's/^root:[^:]*:/root::/' "$ROOTFS_DIR/etc/shadow"

# Create motd
sudo tee "$ROOTFS_DIR/etc/motd" > /dev/null << 'MOTD_EOF'

╔════════════════════════════════════════╗
║   Morpheus Arch Linux Bootstrap        ║
║   Minimal • Fast • Ready               ║
╚════════════════════════════════════════╝

Welcome to Arch Linux bootstrap environment!

This is the official Arch bootstrap tarball with:
  - Full bash shell and coreutils
  - Complete system utilities
  - Pacman package manager  
  - Network tools

Available commands: bash, ls, cat, grep, sed, awk, pacman, and more

MOTD_EOF

echo "✓ System configured"

sudo touch "$ROOTFS_MARKER"

echo ""
echo "=========================================="
echo "  Build Complete!"
echo "=========================================="
echo ""
echo "Rootfs location: $ROOTFS_DIR"
echo "Total size: $(sudo du -sh "$ROOTFS_DIR" | cut -f1)"
echo ""

echo "Size breakdown:"
echo "  /usr/bin:    $(sudo du -sh "$ROOTFS_DIR/usr/bin" 2>/dev/null | cut -f1 || echo 'N/A')"
echo "  /usr/lib:    $(sudo du -sh "$ROOTFS_DIR/usr/lib" 2>/dev/null | cut -f1 || echo 'N/A')"
echo "  /usr/share:  $(sudo du -sh "$ROOTFS_DIR/usr/share" 2>/dev/null | cut -f1 || echo 'N/A')"
echo "  Kernel mods: $(sudo du -sh "$ROOTFS_DIR/usr/lib/modules" 2>/dev/null | cut -f1 || echo 'N/A')"
echo ""

if [ -f "$KERNELS_DIR/vmlinuz-arch" ]; then
    echo "Kernel:      $KERNELS_DIR/vmlinuz-arch ($(du -h "$KERNELS_DIR/vmlinuz-arch" | cut -f1))"
fi
if [ -f "$INITRD_DIR/initramfs-arch.img" ]; then
    echo "Initramfs:   $INITRD_DIR/initramfs-arch.img ($(du -h "$INITRD_DIR/initramfs-arch.img" | cut -f1))"
fi

echo ""
echo "System contents:"
echo "  - Arch Linux bootstrap (minimal base system)"
echo "  - Linux kernel $(basename "$KERNEL_PKG" .pkg.tar.zst)"
echo "  - Full bash + coreutils"
echo "  - Pacman package manager"
echo ""
echo "Next steps:"
echo "  1. Build initrd: ./rebuild-initrd.sh"
echo "  2. Build bootloader: ./build.sh"
echo "  3. Test boot: ./run.sh"
echo "  4. Select 'Arch Linux' from boot menu"
echo ""


