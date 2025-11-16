#!/bin/bash
# Setup script to download and configure initrd for real Linux userspace boot
# This enables testing actual kernel boot to userspace

set -e

cd "$(dirname "$0")"

ESP_DIR="esp"
KERNELS_DIR="$ESP_DIR/kernels"
INITRD_DIR="$ESP_DIR/initrds"

echo "========================================"
echo "  Morpheus Initrd Setup Script"
echo "========================================"
echo ""

# Create initrds directory
mkdir -p "$INITRD_DIR"

# Check if we already have an initrd
if [ -f "$INITRD_DIR/initrd.img" ]; then
    echo "✓ Initrd already exists at $INITRD_DIR/initrd.img"
    echo ""
    read -p "Download fresh initrd? [y/N]: " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo "Using existing initrd."
        exit 0
    fi
fi

echo "Downloading Ubuntu 24.04 (Noble) initrd..."
echo "This initrd matches the kernel already in testing/esp/kernels/"
echo ""

# Download Ubuntu 24.04 netboot initrd
# This is a minimal initrd that works with the Ubuntu kernel
INITRD_URL="http://archive.ubuntu.com/ubuntu/dists/noble/main/installer-amd64/current/legacy-images/netboot/ubuntu-installer/amd64/initrd.gz"

echo "Downloading from: $INITRD_URL"
curl -L -o "$INITRD_DIR/initrd.img.tmp" "$INITRD_URL" 2>&1 | grep -E "%" || true

if [ $? -eq 0 ] && [ -f "$INITRD_DIR/initrd.img.tmp" ]; then
    mv "$INITRD_DIR/initrd.img.tmp" "$INITRD_DIR/initrd.img"
    echo ""
    echo "✓ Downloaded initrd successfully"
    echo "  Location: $INITRD_DIR/initrd.img"
    echo "  Size: $(du -h "$INITRD_DIR/initrd.img" | cut -f1)"
else
    echo ""
    echo "✗ Download failed. Trying alternative source..."
    
    # Try Ubuntu 20.04 as fallback (more widely mirrored)
    INITRD_URL="http://archive.ubuntu.com/ubuntu/dists/focal/main/installer-amd64/current/legacy-images/netboot/ubuntu-installer/amd64/initrd.gz"
    echo "Trying: $INITRD_URL"
    
    curl -L -o "$INITRD_DIR/initrd.img.tmp" "$INITRD_URL" 2>&1 | grep -E "%" || true
    
    if [ $? -eq 0 ] && [ -f "$INITRD_DIR/initrd.img.tmp" ]; then
        mv "$INITRD_DIR/initrd.img.tmp" "$INITRD_DIR/initrd.img"
        echo ""
        echo "✓ Downloaded fallback initrd (Ubuntu 20.04)"
        echo "  Location: $INITRD_DIR/initrd.img"
        echo "  Size: $(du -h "$INITRD_DIR/initrd.img" | cut -f1)"
    else
        echo ""
        echo "✗ Failed to download initrd from both sources."
        echo "Please manually download an initrd.gz and place it at:"
        echo "  $INITRD_DIR/initrd.img"
        exit 1
    fi
fi

echo ""
echo "========================================"
echo "  Setup Complete!"
echo "========================================"
echo ""
echo "The initrd is now available for the bootloader."
echo ""
echo "Next steps:"
echo "  1. Update distro_launcher.rs to point to the initrd"
echo "  2. Rebuild the bootloader: ./testing/build.sh"
echo "  3. Test boot: ./testing/run.sh"
echo ""
echo "For verbose kernel output, use the 'Fedora (verbose)' menu option"
echo "which has 'console=ttyS0 debug' in the kernel command line."
echo ""
