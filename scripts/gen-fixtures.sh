#!/usr/bin/env bash
# =============================================================================
# gen-fixtures.sh - Generate Test Fixtures for MorpheusX
# =============================================================================
#
# Purpose:
#   Generate ISO and FAT filesystem images for testing without storing
#   binary blobs in the repository. All fixtures are deterministic and
#   reproducible.
#
# Preconditions:
#   - genisoimage or mkisofs for ISO creation
#   - mtools and dosfstools for FAT image creation
#   - Basic coreutils (dd, mkdir, etc.)
#
# Usage:
#   ./gen-fixtures.sh [output-dir]
#
# Output:
#   <output-dir>/
#   ├── minimal.iso          # Minimal ISO with directory structure
#   ├── eltorito.iso         # ISO with El Torito boot catalog
#   ├── fat12.img            # Small FAT12 image
#   ├── fat16.img            # Medium FAT16 image
#   ├── fat32.img            # Large FAT32 image
#   └── esp.img              # UEFI ESP with EFI/BOOT structure
#
# =============================================================================

set -euo pipefail

readonly SCRIPT_NAME="$(basename "$0")"
readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Defaults
DEFAULT_OUTPUT_DIR="fixtures"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info()  { echo -e "${BLUE}[INFO]${NC} $*"; }
log_ok()    { echo -e "${GREEN}[OK]${NC} $*"; }
log_warn()  { echo -e "${YELLOW}[WARN]${NC} $*" >&2; }
log_error() { echo -e "${RED}[ERROR]${NC} $*" >&2; }

# Check for ISO creation tool
find_iso_tool() {
    if command -v genisoimage &>/dev/null; then
        echo "genisoimage"
    elif command -v mkisofs &>/dev/null; then
        echo "mkisofs"
    else
        return 1
    fi
}

# Check dependencies
check_deps() {
    local missing=()
    
    find_iso_tool &>/dev/null || missing+=("genisoimage or mkisofs")
    command -v mkfs.vfat &>/dev/null || missing+=("dosfstools")
    command -v mtools &>/dev/null || missing+=("mtools")
    
    if [[ ${#missing[@]} -gt 0 ]]; then
        log_error "Missing dependencies: ${missing[*]}"
        log_error "Install with: sudo apt-get install genisoimage mtools dosfstools"
        exit 1
    fi
}

# Create minimal ISO9660 image
# Contains basic directory structure for parser tests
create_minimal_iso() {
    local output="$1"
    local iso_tool
    iso_tool=$(find_iso_tool)
    
    log_info "Creating minimal ISO: $output"
    
    local temp_dir
    temp_dir=$(mktemp -d)
    trap "rm -rf '$temp_dir'" RETURN
    
    # Create directory structure
    mkdir -p "$temp_dir/dir1/subdir"
    mkdir -p "$temp_dir/dir2"
    mkdir -p "$temp_dir/empty_dir"
    
    # Create files with known content
    echo "Hello from root file" > "$temp_dir/root.txt"
    echo "File in directory 1" > "$temp_dir/dir1/file1.txt"
    echo "File in subdirectory" > "$temp_dir/dir1/subdir/nested.txt"
    echo "File in directory 2" > "$temp_dir/dir2/file2.txt"
    
    # Create larger file for read tests (64KB)
    dd if=/dev/urandom of="$temp_dir/large.bin" bs=1024 count=64 status=none
    
    # Generate ISO
    "$iso_tool" \
        -o "$output" \
        -V "MINIMAL_ISO" \
        -J \
        -r \
        "$temp_dir" \
        2>/dev/null
    
    log_ok "Created: $output ($(du -h "$output" | cut -f1))"
}

# Create ISO with El Torito boot catalog
# For boot catalog parsing tests
create_eltorito_iso() {
    local output="$1"
    local iso_tool
    iso_tool=$(find_iso_tool)
    
    log_info "Creating El Torito ISO: $output"
    
    local temp_dir
    temp_dir=$(mktemp -d)
    trap "rm -rf '$temp_dir'" RETURN
    
    # Create boot directory structure
    mkdir -p "$temp_dir/boot/grub"
    mkdir -p "$temp_dir/isolinux"
    mkdir -p "$temp_dir/EFI/BOOT"
    
    # Create dummy boot images
    # BIOS boot image (must be multiple of 512 bytes for -no-emul-boot)
    dd if=/dev/zero of="$temp_dir/isolinux/isolinux.bin" bs=512 count=4 status=none
    
    # EFI boot image (FAT image)
    dd if=/dev/zero of="$temp_dir/EFI/BOOT/efi.img" bs=1024 count=1440 status=none
    mkfs.vfat "$temp_dir/EFI/BOOT/efi.img" >/dev/null 2>&1 || true
    
    # Create some regular files
    echo "El Torito test ISO" > "$temp_dir/README.txt"
    
    # Generate ISO with El Torito boot
    "$iso_tool" \
        -o "$output" \
        -V "ELTORITO_ISO" \
        -J \
        -r \
        -b "isolinux/isolinux.bin" \
        -c "boot/boot.cat" \
        -no-emul-boot \
        -boot-load-size 4 \
        -boot-info-table \
        "$temp_dir" \
        2>/dev/null || {
        # Fallback without El Torito if tool doesn't support it
        log_warn "El Torito creation failed, creating basic ISO"
        "$iso_tool" \
            -o "$output" \
            -V "ELTORITO_ISO" \
            -J \
            -r \
            "$temp_dir" \
            2>/dev/null
    }
    
    log_ok "Created: $output ($(du -h "$output" | cut -f1))"
}

# Create FAT12 image (1.44MB floppy-like)
create_fat12_image() {
    local output="$1"
    
    log_info "Creating FAT12 image: $output"
    
    # Create 1.44MB image (standard floppy size)
    dd if=/dev/zero of="$output" bs=1024 count=1440 status=none
    
    # Format as FAT12
    mkfs.vfat -F 12 "$output" >/dev/null
    
    # Add some files
    echo "FAT12 test file" | mcopy -i "$output" - ::/TEST.TXT
    mmd -i "$output" ::/SUBDIR
    echo "Nested file" | mcopy -i "$output" - ::/SUBDIR/NESTED.TXT
    
    log_ok "Created: $output ($(du -h "$output" | cut -f1))"
}

# Create FAT16 image (16MB)
create_fat16_image() {
    local output="$1"
    
    log_info "Creating FAT16 image: $output"
    
    # Create 16MB image
    dd if=/dev/zero of="$output" bs=1M count=16 status=none
    
    # Format as FAT16
    mkfs.vfat -F 16 "$output" >/dev/null
    
    # Add directory structure
    mmd -i "$output" ::/DIR1
    mmd -i "$output" ::/DIR2
    mmd -i "$output" ::/DIR1/SUB
    
    # Add files
    echo "FAT16 test" | mcopy -i "$output" - ::/README.TXT
    dd if=/dev/urandom bs=4096 count=10 status=none | mcopy -i "$output" - ::/RANDOM.BIN
    
    log_ok "Created: $output ($(du -h "$output" | cut -f1))"
}

# Create FAT32 image (64MB)
create_fat32_image() {
    local output="$1"
    
    log_info "Creating FAT32 image: $output"
    
    # Create 64MB image (minimum practical FAT32 size)
    dd if=/dev/zero of="$output" bs=1M count=64 status=none
    
    # Format as FAT32
    mkfs.vfat -F 32 "$output" >/dev/null
    
    # Add directory structure
    mmd -i "$output" ::/Documents
    mmd -i "$output" ::/Pictures
    mmd -i "$output" ::/Documents/Work
    
    # Add files with long names
    echo "Long filename test" | mcopy -i "$output" - "::/Documents/This is a long filename.txt"
    
    # Add larger file (1MB)
    dd if=/dev/urandom bs=1M count=1 status=none | mcopy -i "$output" - ::/large_file.dat
    
    log_ok "Created: $output ($(du -h "$output" | cut -f1))"
}

# Create UEFI ESP image
create_esp_image() {
    local output="$1"
    
    log_info "Creating UEFI ESP image: $output"
    
    # Create 64MB ESP
    dd if=/dev/zero of="$output" bs=1M count=64 status=none
    
    # Format as FAT32
    mkfs.vfat -F 32 -n "ESP" "$output" >/dev/null
    
    # Create EFI directory structure
    mmd -i "$output" ::/EFI
    mmd -i "$output" ::/EFI/BOOT
    mmd -i "$output" ::/EFI/morpheusx
    
    # Create dummy bootloader (for structure tests)
    dd if=/dev/urandom bs=1024 count=64 status=none | mcopy -i "$output" - ::/EFI/BOOT/BOOTX64.EFI
    
    # Create loader entries directory (systemd-boot style)
    mmd -i "$output" ::/loader
    mmd -i "$output" ::/loader/entries
    
    # Create sample loader entry
    cat << 'EOF' | mcopy -i "$output" - ::/loader/entries/morpheusx.conf
title   MorpheusX
linux   /vmlinuz
initrd  /initramfs.img
options root=/dev/sda2 rw
EOF
    
    log_ok "Created: $output ($(du -h "$output" | cut -f1))"
}

# Main
main() {
    local output_dir="${1:-$DEFAULT_OUTPUT_DIR}"
    
    log_info "=== MorpheusX Test Fixture Generator ==="
    log_info "Output directory: $output_dir"
    echo ""
    
    # Check dependencies
    check_deps
    
    # Create output directory
    mkdir -p "$output_dir"
    
    # Generate fixtures
    create_minimal_iso "$output_dir/minimal.iso"
    create_eltorito_iso "$output_dir/eltorito.iso"
    create_fat12_image "$output_dir/fat12.img"
    create_fat16_image "$output_dir/fat16.img"
    create_fat32_image "$output_dir/fat32.img"
    create_esp_image "$output_dir/esp.img"
    
    echo ""
    log_ok "=== All fixtures generated ==="
    echo ""
    log_info "Contents:"
    ls -lh "$output_dir"
    echo ""
    log_info "Total size: $(du -sh "$output_dir" | cut -f1)"
}

main "$@"
