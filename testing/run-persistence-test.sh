#!/bin/bash
# Test Morpheus persistence - boot ONLY from the 10GB disk
# This verifies the bootloader was actually installed to the ESP

set -e

cd "$(dirname "$0")"

if [ ! -f test-disk-10g.img ]; then
    echo "Error: test-disk-10g.img not found"
    echo "Run the installer first to create and install Morpheus"
    exit 1
fi

echo "========================================="
echo "   MORPHEUS PERSISTENCE TEST"
echo "========================================="
echo ""
echo "Booting ONLY from test-disk-10g.img"
echo "No ESP directory mounted - testing real persistence"
echo ""
echo "If Morpheus boots, installation succeeded!"
echo "Press Ctrl+A then X to exit QEMU"
echo ""

# Boot ONLY the 10GB disk (no ESP directory, no other disks)
qemu-system-x86_64 \
    -bios /usr/share/edk2/ovmf/OVMF_CODE.fd \
    -drive format=raw,file=test-disk-10g.img \
    -net none \
    -m 256M \
    -serial stdio
