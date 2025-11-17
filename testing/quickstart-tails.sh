#!/bin/bash
# Quick Start: Boot Tails OS with Morpheus
# This is the fastest way to get a full Linux userland running

set -e

cd "$(dirname "$0")"

echo "╔════════════════════════════════════════════════════════════╗"
echo "║  Morpheus + Tails OS - Quick Start                         ║"
echo "╚════════════════════════════════════════════════════════════╝"
echo ""
echo "This will:"
echo "  1. Download Tails OS live system (1.3GB)"
echo "  2. Extract kernel and initrd"
echo "  3. Build Morpheus bootloader"
echo "  4. Boot in QEMU"
echo ""
echo "Total time: ~10-15 minutes (depending on download speed)"
echo ""

# Step 1: Install Tails
if [ ! -f esp/kernels/vmlinuz-tails ] || [ ! -f esp/initrds/initrd-tails.img ]; then
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "Step 1/3: Installing Tails OS"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    ./install-live-distro.sh <<EOF
3
EOF
else
    echo "✓ Tails OS already installed, skipping download"
fi

# Step 2: Build bootloader
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Step 2/3: Building Morpheus bootloader"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
./build.sh

# Step 3: Launch
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Step 3/3: Launching in QEMU"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "In the bootloader menu:"
echo "  • Use UP/DOWN arrows to select 'Tails OS (Full Featured)'"
echo "  • Press ENTER to boot"
echo ""
echo "Expected boot time: 30-60 seconds"
echo ""
echo "Press Enter to continue..."
read

./run.sh <<EOF
1
EOF
