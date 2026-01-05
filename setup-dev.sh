#!/bin/bash
# MorpheusX Complete Development Environment Setup
# Distro-agnostic: detects package manager and sets up everything

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  MorpheusX - Complete Development Environment Setup          ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""

# Detect package manager
detect_pkg_manager() {
    if command -v pacman &> /dev/null; then
        echo "arch"
    elif command -v apt &> /dev/null || command -v apt-get &> /dev/null; then
        echo "debian"
    elif command -v dnf &> /dev/null; then
        echo "fedora"
    elif command -v yum &> /dev/null; then
        echo "rhel"
    elif command -v zypper &> /dev/null; then
        echo "suse"
    elif command -v apk &> /dev/null; then
        echo "alpine"
    else
        echo "unknown"
    fi
}

PKG_MGR=$(detect_pkg_manager)

echo "Detected distribution: $PKG_MGR"
echo ""

# Check if running as root
if [ "$EUID" -eq 0 ]; then 
    echo "⚠️  Don't run this as root. It will ask for sudo when needed."
    exit 1
fi

echo "This will:"
echo "  1. Install dependencies (nasm, qemu, ovmf, rust)"
echo "  2. Setup Rust UEFI target"
echo "  3. Download and install Tails OS (1.3GB)"
echo "  4. Create test disk images (50GB + 10GB)"
echo "  5. Build MorpheusX bootloader"
echo "  6. Launch QEMU with everything configured"
echo ""
echo "Total download: ~1.5GB"
echo "Disk space needed: ~5GB"
echo "Time: ~15-20 minutes"
echo ""
read -p "Continue? [Y/n]: " -n 1 -r
echo
if [[ $REPLY =~ ^[Nn]$ ]]; then
    echo "Aborted."
    exit 0
fi

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Step 1/6: Installing system dependencies"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

install_deps() {
    case $PKG_MGR in
        arch)
            echo "Using pacman..."
            PKGS="nasm qemu-full ovmf rust"
            
            # Check which packages are missing
            MISSING=""
            for pkg in $PKGS; do
                if ! pacman -Qi $pkg &> /dev/null; then
                    MISSING="$MISSING $pkg"
                fi
            done
            
            if [ -n "$MISSING" ]; then
                echo "Installing:$MISSING"
                sudo pacman -S --needed --noconfirm $MISSING
            else
                echo "✓ All packages already installed"
            fi
            
            OVMF_PATH="/usr/share/OVMF/OVMF_CODE.fd"
            ;;
            
        debian)
            echo "Using apt..."
            PKGS="nasm qemu-system-x86 ovmf curl rsync parted dosfstools"
            
            # Update package list if not done recently
            if [ ! -f /var/lib/apt/periodic/update-success-stamp ] || \
               [ $(find /var/lib/apt/periodic/update-success-stamp -mtime +7) ]; then
                echo "Updating package list..."
                sudo apt-get update -qq
            fi
            
            # Install missing packages
            DEBIAN_FRONTEND=noninteractive sudo apt-get install -y -qq $PKGS
            
            # Install rust via rustup if not present
            if ! command -v rustc &> /dev/null; then
                echo "Installing Rust via rustup..."
                curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
                source "$HOME/.cargo/env"
            fi
            
            OVMF_PATH="/usr/share/OVMF/OVMF_CODE.fd"
            if [ ! -f "$OVMF_PATH" ]; then
                OVMF_PATH="/usr/share/ovmf/OVMF.fd"
            fi
            ;;
            
        fedora)
            echo "Using dnf..."
            PKGS="nasm qemu-system-x86 edk2-ovmf curl rsync parted dosfstools"
            
            sudo dnf install -y -q $PKGS
            
            # Install rust via rustup if not present
            if ! command -v rustc &> /dev/null; then
                echo "Installing Rust via rustup..."
                curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
                source "$HOME/.cargo/env"
            fi
            
            OVMF_PATH="/usr/share/edk2/ovmf/OVMF_CODE.fd"
            ;;
            
        rhel)
            echo "Using yum..."
            PKGS="nasm qemu-kvm edk2-ovmf curl rsync parted dosfstools"
            
            sudo yum install -y -q $PKGS
            
            if ! command -v rustc &> /dev/null; then
                echo "Installing Rust via rustup..."
                curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
                source "$HOME/.cargo/env"
            fi
            
            OVMF_PATH="/usr/share/edk2/ovmf/OVMF_CODE.fd"
            ;;
            
        suse)
            echo "Using zypper..."
            PKGS="nasm qemu-x86 qemu-ovmf-x86_64 curl rsync parted dosfstools"
            
            sudo zypper install -y $PKGS
            
            if ! command -v rustc &> /dev/null; then
                echo "Installing Rust via rustup..."
                curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
                source "$HOME/.cargo/env"
            fi
            
            OVMF_PATH="/usr/share/qemu/ovmf-x86_64-code.bin"
            ;;
            
        alpine)
            echo "Using apk..."
            PKGS="nasm qemu-system-x86_64 ovmf curl rsync parted dosfstools"
            
            sudo apk add $PKGS
            
            if ! command -v rustc &> /dev/null; then
                echo "Installing Rust via rustup..."
                curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
                source "$HOME/.cargo/env"
            fi
            
            OVMF_PATH="/usr/share/OVMF/OVMF_CODE.fd"
            ;;
            
        *)
            echo "⚠️  Unknown package manager. Please install manually:"
            echo "  - nasm (assembler)"
            echo "  - qemu-system-x86_64 (emulator)"
            echo "  - OVMF firmware"
            echo "  - rust + cargo"
            echo "  - curl, rsync, parted, dosfstools"
            exit 1
            ;;
    esac
}

install_deps

# Verify critical tools exist
echo ""
echo "Verifying installation..."
for cmd in nasm qemu-system-x86_64 rustc cargo; do
    if ! command -v $cmd &> /dev/null; then
        echo "✗ $cmd not found!"
        exit 1
    fi
    echo "✓ $cmd found"
done

# Verify OVMF
if [ ! -f "$OVMF_PATH" ]; then
    echo "⚠️  OVMF not found at $OVMF_PATH"
    echo "Looking for alternative locations..."
    
    for path in \
        /usr/share/OVMF/OVMF_CODE.fd \
        /usr/share/edk2/ovmf/OVMF_CODE.fd \
        /usr/share/edk2-ovmf/OVMF_CODE.fd \
        /usr/share/ovmf/OVMF.fd \
        /usr/share/qemu/ovmf-x86_64-code.bin; do
        if [ -f "$path" ]; then
            OVMF_PATH="$path"
            echo "✓ Found OVMF at $OVMF_PATH"
            break
        fi
    done
    
    if [ ! -f "$OVMF_PATH" ]; then
        echo "✗ OVMF firmware not found. Install ovmf/edk2 package."
        exit 1
    fi
else
    echo "✓ OVMF found at $OVMF_PATH"
fi

# Update run.sh with correct OVMF path
if [ -f "testing/run.sh" ]; then
    echo "Updating run.sh with OVMF path..."
    sed -i "s|/usr/share/OVMF/OVMF_CODE.fd|$OVMF_PATH|g" testing/run.sh
    sed -i "s|/usr/share/edk2/ovmf/OVMF_CODE.fd|$OVMF_PATH|g" testing/run.sh
    echo "✓ run.sh updated"
fi

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Step 2/6: Setting up Rust UEFI target"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

if rustup target list | grep -q "x86_64-unknown-uefi (installed)"; then
    echo "✓ UEFI target already installed"
else
    echo "Installing x86_64-unknown-uefi target..."
    rustup target add x86_64-unknown-uefi
    echo "✓ UEFI target installed"
fi

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Step 3/6: Setting up ESP directory structure"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

cd testing
mkdir -p esp/EFI/BOOT
mkdir -p esp/kernels
mkdir -p esp/initrds
echo "✓ ESP directory structure created"

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Step 4/6: Installing Tails OS (this will take a while)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# Check if Tails already installed
if [ -f "esp/kernels/vmlinuz-tails" ] && [ -f "esp/initrds/initrd-tails.img" ]; then
    echo "✓ Tails OS already installed"
else
    # Run install-tails.sh with auto-yes
    echo "Downloading and extracting Tails OS..."
    echo "This may take 10-15 minutes depending on connection..."
    echo ""
    
    # Run with auto-accept
    yes y | ./install-tails.sh || true
    
    if [ -f "esp/kernels/vmlinuz-tails" ]; then
        echo "✓ Tails OS installed successfully"
    else
        echo "⚠️  Tails installation may have failed. Continuing anyway..."
    fi
fi

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Step 5/6: Creating test disk images"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# Create test disks if they don't exist
if [ ! -f "test-disk-50g.img" ]; then
    echo "Creating 50GB test disk with GPT..."
    yes y | ./create-test-disk.sh || true
    echo "✓ 50GB test disk created"
else
    echo "✓ 50GB test disk already exists"
fi

# Small test disks will be created by run.sh automatically
echo "✓ Test disk setup complete"

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Step 6/6: Building MorpheusX bootloader"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

./build.sh

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  ✓ Development Environment Setup Complete!                   ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""
echo "Environment details:"
echo "  • Bootloader: testing/esp/EFI/BOOT/BOOTX64.EFI"
echo "  • Kernel: testing/esp/kernels/vmlinuz-tails"
echo "  • Initrd: testing/esp/initrds/initrd-tails.img"
echo "  • RootFS: testing/esp/initrds/filesystem.squashfs"
echo "  • Test disks: test-disk-50g.img, test-disk-10g.img (auto-created)"
echo "  • OVMF: $OVMF_PATH"
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Launching QEMU..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "In the bootloader menu:"
echo "  • Use ↑/↓ arrows to select 'Tails OS'"
echo "  • Press ENTER to boot"
echo "  • Press Ctrl+A then X to exit QEMU"
echo ""
echo "Press ENTER to launch QEMU..."
read

# Launch QEMU with ESP image (option 1)
echo 1 | ./run.sh

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║  Setup complete! You can now develop MorpheusX.              ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""
echo "Quick commands:"
echo "  cd testing"
echo "  ./build.sh          # Rebuild bootloader"
echo "  ./run.sh            # Run QEMU"
echo "  ./install-arch.sh   # Install Arch Linux rootfs"
echo ""
echo "For debugging:"
echo "  ./debug.sh          # Connect GDB (run QEMU first)"
echo ""
