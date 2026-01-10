#!/bin/bash
# =============================================================================
# MorpheusX Network Stack Test Harness
# =============================================================================
# Tests ISO download over HTTP using VirtIO-net in QEMU
#
# This script:
# 1. Starts a local HTTP server serving a test ISO
# 2. Boots QEMU with VirtIO-net + user networking
# 3. The bootloader should:
#    - Exit UEFI Boot Services
#    - Initialize VirtIO-net driver
#    - Obtain IP via DHCP (QEMU's built-in DHCP server)
#    - Download the ISO via HTTP
#    - Write the ISO to VirtIO-blk disk
#
# Usage: ./test-network.sh [--no-gui] [--create-iso SIZE_MB]
# =============================================================================

set -e

cd "$(dirname "$0")"

# Configuration
HTTP_PORT=8000
ISO_SIZE_MB=${ISO_SIZE_MB:-50}  # Default 50MB test ISO
TEST_ISO="test-iso.img"
HTTP_SERVER_PID=""
QEMU_EXTRA_ARGS=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --no-gui)
            QEMU_EXTRA_ARGS="$QEMU_EXTRA_ARGS -nographic"
            shift
            ;;
        --create-iso)
            ISO_SIZE_MB=$2
            shift 2
            ;;
        --help|-h)
            echo "Usage: $0 [--no-gui] [--create-iso SIZE_MB]"
            echo ""
            echo "Options:"
            echo "  --no-gui        Run QEMU without GUI (serial console only)"
            echo "  --create-iso N  Create test ISO of N MB (default: 50)"
            echo ""
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_ok() { echo -e "${GREEN}[OK]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_err() { echo -e "${RED}[ERROR]${NC} $1"; }

# Cleanup function
cleanup() {
    log_info "Cleaning up..."
    if [ -n "$HTTP_SERVER_PID" ] && kill -0 "$HTTP_SERVER_PID" 2>/dev/null; then
        kill "$HTTP_SERVER_PID" 2>/dev/null || true
        log_ok "Stopped HTTP server (PID $HTTP_SERVER_PID)"
    fi
}
trap cleanup EXIT

# =============================================================================
# STEP 1: Check prerequisites
# =============================================================================
echo ""
echo "========================================"
echo "  MorpheusX Network Stack Test"
echo "========================================"
echo ""

log_info "Checking prerequisites..."

if [ ! -f esp/EFI/BOOT/BOOTX64.EFI ]; then
    log_err "Bootloader not built. Run ./build.sh first"
    exit 1
fi
log_ok "Bootloader found"

if ! command -v qemu-system-x86_64 &> /dev/null; then
    log_err "QEMU not installed"
    exit 1
fi
log_ok "QEMU available"

if ! command -v python3 &> /dev/null; then
    log_err "Python3 not installed (needed for HTTP server)"
    exit 1
fi
log_ok "Python3 available"

# =============================================================================
# STEP 2: Create test ISO image
# =============================================================================
echo ""
log_info "Creating test ISO image (${ISO_SIZE_MB}MB)..."

if [ -f "$TEST_ISO" ]; then
    log_warn "Test ISO already exists, recreating..."
fi

# Create ISO with recognizable pattern for verification
# Pattern: "MORPHEUS_TEST_ISO" header + sequential data
dd if=/dev/zero of="$TEST_ISO" bs=1M count="$ISO_SIZE_MB" status=none

# Write magic header at start
echo -n "MORPHEUS_TEST_ISO_v1.0" | dd of="$TEST_ISO" bs=1 count=22 conv=notrunc status=none

# Write size marker
printf '%08x' "$ISO_SIZE_MB" | dd of="$TEST_ISO" bs=1 seek=32 count=8 conv=notrunc status=none

# Calculate and write checksum of first 1MB (for verification)
CHECKSUM=$(dd if="$TEST_ISO" bs=1M count=1 status=none | sha256sum | cut -d' ' -f1)
echo -n "$CHECKSUM" | dd of="$TEST_ISO" bs=1 seek=64 count=64 conv=notrunc status=none

log_ok "Created test ISO: $TEST_ISO (${ISO_SIZE_MB}MB)"
log_info "SHA256: $CHECKSUM"

# =============================================================================
# STEP 3: Create target disk for ISO storage
# =============================================================================
echo ""
log_info "Creating target disk for ISO storage..."

TARGET_DISK="network-test-disk.img"
TARGET_DISK_SIZE=$((ISO_SIZE_MB + 100))  # ISO size + overhead

if [ ! -f "$TARGET_DISK" ] || [ "$TARGET_DISK_SIZE" -gt "$(stat -c%s "$TARGET_DISK" 2>/dev/null | awk '{print int($1/1024/1024)}')" ]; then
    dd if=/dev/zero of="$TARGET_DISK" bs=1M count="$TARGET_DISK_SIZE" status=none
    # Create GPT with single partition
    parted -s "$TARGET_DISK" mklabel gpt
    parted -s "$TARGET_DISK" mkpart primary 1MiB 100%
    log_ok "Created target disk: $TARGET_DISK (${TARGET_DISK_SIZE}MB)"
else
    log_ok "Using existing target disk: $TARGET_DISK"
fi

# =============================================================================
# STEP 4: Create ESP image with bootloader
# =============================================================================
echo ""
log_info "Creating ESP disk image..."

ESP_SIZE=$(du -sb esp | awk '{print int(($1 / 1024 / 1024) + 50)}')
rm -f esp.img
dd if=/dev/zero of=esp.img bs=1M count=$ESP_SIZE status=none
mkfs.vfat -F 32 -n "ESP" esp.img >/dev/null

mkdir -p /tmp/esp-mount
sudo mount -o loop esp.img /tmp/esp-mount
sudo rsync -a --exclude='rootfs' esp/ /tmp/esp-mount/ 2>/dev/null || true
sudo umount /tmp/esp-mount
rmdir /tmp/esp-mount

log_ok "Created ESP image: esp.img (${ESP_SIZE}MB)"

# =============================================================================
# STEP 5: Start HTTP server
# =============================================================================
echo ""
log_info "Starting HTTP server on port $HTTP_PORT..."

# Create simple Python HTTP server
python3 -m http.server $HTTP_PORT --directory . &
HTTP_SERVER_PID=$!
sleep 1

if ! kill -0 "$HTTP_SERVER_PID" 2>/dev/null; then
    log_err "Failed to start HTTP server"
    exit 1
fi

log_ok "HTTP server started (PID $HTTP_SERVER_PID)"
log_info "ISO URL: http://10.0.2.2:$HTTP_PORT/$TEST_ISO"

# =============================================================================
# STEP 6: Start QEMU with network test configuration
# =============================================================================
echo ""
echo "========================================"
echo "  Starting QEMU"
echo "========================================"
echo ""
log_info "Network configuration:"
echo "  - Guest IP:     10.0.2.15 (DHCP from QEMU)"
echo "  - Gateway:      10.0.2.2"
echo "  - Host access:  10.0.2.2:$HTTP_PORT"
echo ""
log_info "Expected boot sequence:"
echo "  1. UEFI boot from ESP"
echo "  2. Exit Boot Services"
echo "  3. Initialize VirtIO-net"
echo "  4. DHCP to get IP"
echo "  5. HTTP GET http://10.0.2.2:$HTTP_PORT/$TEST_ISO"
echo "  6. Write ISO to VirtIO-blk"
echo ""
log_warn "Press Ctrl+A then X to exit QEMU"
echo ""

# QEMU command with VirtIO-net and VirtIO-blk
# User networking provides:
# - DHCP server (assigns 10.0.2.15)
# - Gateway at 10.0.2.2 (host)
# - DNS at 10.0.2.3

QEMU_CMD=(
    qemu-system-x86_64
    # UEFI firmware
    -bios /usr/share/OVMF/x64/OVMF_CODE.4m.fd
    # ESP with bootloader
    -drive format=raw,file=esp.img,if=none,id=esp
    -device virtio-blk-pci,drive=esp,bus=pci.0,addr=0x04
    # Target disk for ISO storage
    -drive format=raw,file=$TARGET_DISK,if=none,id=target
    -device virtio-blk-pci,drive=target,bus=pci.0,addr=0x05
    # VirtIO network with user networking (modern mode for MMIO access)
    -device virtio-net-pci,netdev=net0,bus=pci.0,addr=0x03,mac=52:54:00:12:34:56,disable-legacy=on,disable-modern=off
    -netdev "user,id=net0,hostfwd=tcp::2222-:22,guestfwd=tcp:10.0.2.100:${HTTP_PORT}-tcp:127.0.0.1:${HTTP_PORT}"
    # System config
    -smp 4
    -m 4G
    # Serial console
    -serial mon:stdio
    # Debug
    -d guest_errors
)

# Add GUI or nographic based on option
if [[ "$QEMU_EXTRA_ARGS" == *"-nographic"* ]]; then
    QEMU_CMD+=(-nographic)
else
    QEMU_CMD+=(-vga virtio -display gtk)
fi

# Run QEMU
"${QEMU_CMD[@]}"

# =============================================================================
# STEP 7: Verify ISO was written to target disk
# =============================================================================
echo ""
echo "========================================"
echo "  Verifying Results"
echo "========================================"
echo ""

# Check if target disk has the ISO magic header
if [ -f "$TARGET_DISK" ]; then
    HEADER=$(dd if="$TARGET_DISK" bs=1 count=22 skip=$((1024*1024)) status=none 2>/dev/null || echo "")
    if [ "$HEADER" == "MORPHEUS_TEST_ISO_v1.0" ]; then
        log_ok "SUCCESS: Test ISO was written to target disk!"
        log_info "First 1MB partition header, ISO starts at offset 1MB"
    else
        log_warn "ISO header not found at expected location"
        log_info "Manual verification needed - check target disk contents"
    fi
fi

log_info "Test complete."
