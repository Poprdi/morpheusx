#!/usr/bin/env bash
# =============================================================================
# qemu-e2e.sh - QEMU End-to-End Boot Test for MorpheusX
# =============================================================================
#
# Purpose:
#   Boot the MorpheusX UEFI bootloader in QEMU with OVMF and validate
#   successful boot by checking for a deterministic serial token.
#
# Preconditions:
#   - QEMU (qemu-system-x86_64) installed
#   - OVMF firmware available
#   - mtools and dosfstools for FAT image creation
#   - The .efi file to test
#
# Usage:
#   ./qemu-e2e.sh <efi-file> [options]
#
# Options:
#   --ovmf <path>     Path to OVMF firmware (auto-detected if not specified)
#   --timeout <sec>   Boot timeout in seconds (default: 120)
#   --token <string>  Expected boot token (default: MORPHEUSX_BOOT_OK)
#   --keep-esp        Don't delete the ESP image after test
#   --verbose         Show QEMU output in real-time
#   --kvm             Force KVM (fail if unavailable)
#   --no-kvm          Disable KVM (use TCG)
#
# Exit codes:
#   0 - Success (boot token found)
#   1 - Failure (boot token not found or QEMU error)
#   2 - Invalid arguments or missing dependencies
#
# =============================================================================

set -euo pipefail

readonly SCRIPT_NAME="$(basename "$0")"
readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Defaults
DEFAULT_TIMEOUT=120
DEFAULT_TOKEN="MORPHEUSX_BOOT_OK"
ESP_SIZE_MB=64

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

usage() {
    cat << EOF
Usage: $SCRIPT_NAME <efi-file> [options]

Boot MorpheusX UEFI bootloader in QEMU and validate via serial output.

Options:
    --ovmf <path>     Path to OVMF firmware (auto-detected if not specified)
    --timeout <sec>   Boot timeout in seconds (default: $DEFAULT_TIMEOUT)
    --token <string>  Expected boot token (default: $DEFAULT_TOKEN)
    --keep-esp        Don't delete the ESP image after test
    --verbose         Show QEMU output in real-time
    --kvm             Force KVM (fail if unavailable)
    --no-kvm          Disable KVM (use TCG)
    --help            Show this help

Examples:
    $SCRIPT_NAME target/x86_64-unknown-uefi/release/morpheus-bootloader.efi
    $SCRIPT_NAME BOOTX64.EFI --timeout 60 --verbose
    $SCRIPT_NAME bootloader.efi --ovmf /usr/share/OVMF/OVMF_CODE.fd

Exit codes:
    0 - Boot token found (success)
    1 - Boot token not found or QEMU error (failure)
    2 - Invalid arguments or missing dependencies
EOF
}

# Find OVMF firmware
find_ovmf() {
    local paths=(
        "/usr/share/OVMF/x64/OVMF.4m.fd"
        "/usr/share/OVMF/x64/OVMF_CODE.4m.fd"
        "/usr/share/edk2/x64/OVMF_CODE.4m.fd"
        "/usr/share/OVMF/OVMF_CODE.fd"
        "/usr/share/OVMF/OVMF.fd"
        "/usr/share/edk2/ovmf/OVMF_CODE.fd"
        "/usr/share/edk2-ovmf/OVMF_CODE.fd"
        "/usr/share/ovmf/OVMF.fd"
        "/usr/share/qemu/ovmf-x86_64-code.bin"
        "/usr/share/qemu/OVMF.fd"
    )
    
    for path in "${paths[@]}"; do
        if [[ -f "$path" ]]; then
            echo "$path"
            return 0
        fi
    done
    
    return 1
}

# Check dependencies
check_deps() {
    local missing=()
    
    command -v qemu-system-x86_64 &>/dev/null || missing+=("qemu-system-x86")
    command -v mkfs.vfat &>/dev/null || missing+=("dosfstools")
    command -v mtools &>/dev/null || missing+=("mtools")
    
    if [[ ${#missing[@]} -gt 0 ]]; then
        log_error "Missing dependencies: ${missing[*]}"
        log_error "Install with: sudo apt-get install ${missing[*]}"
        exit 2
    fi
}

# Create FAT32 ESP image with bootloader
create_esp_image() {
    local efi_file="$1"
    local esp_img="$2"
    
    log_info "Creating ${ESP_SIZE_MB}MB FAT32 ESP image..."
    
    # Create empty image
    dd if=/dev/zero of="$esp_img" bs=1M count=$ESP_SIZE_MB status=none
    
    # Format as FAT32
    mkfs.vfat -F 32 "$esp_img" >/dev/null
    
    # Create EFI directory structure
    mmd -i "$esp_img" ::/EFI
    mmd -i "$esp_img" ::/EFI/BOOT
    
    # Copy bootloader
    mcopy -i "$esp_img" "$efi_file" ::/EFI/BOOT/BOOTX64.EFI
    
    log_ok "ESP image created: $esp_img"
}

# Run QEMU and capture serial output
run_qemu() {
    local ovmf="$1"
    local esp_img="$2"
    local timeout="$3"
    local token="$4"
    local use_kvm="$5"
    local verbose="$6"
    local serial_log="$7"
    
    local qemu_args=(
        -m 512M
        -bios "$ovmf"
        -drive "format=raw,file=$esp_img"
        -net none
        -display none
        -no-reboot
        -no-shutdown
    )
    
    # KVM acceleration
    if [[ "$use_kvm" == "force" ]]; then
        if [[ ! -e /dev/kvm ]] || [[ ! -r /dev/kvm ]] || [[ ! -w /dev/kvm ]]; then
            log_error "KVM requested but /dev/kvm not available or not accessible"
            exit 2
        fi
        qemu_args+=(-enable-kvm)
        log_info "Using KVM acceleration"
    elif [[ "$use_kvm" == "auto" ]] && [[ -e /dev/kvm ]] && [[ -r /dev/kvm ]] && [[ -w /dev/kvm ]]; then
        qemu_args+=(-enable-kvm)
        log_info "Using KVM acceleration (auto-detected)"
    else
        log_info "Using TCG emulation (no KVM)"
    fi
    
    # Serial output
    if [[ "$verbose" == "true" ]]; then
        qemu_args+=(-serial mon:stdio)
    else
        qemu_args+=(-serial "file:$serial_log")
    fi
    
    log_info "Starting QEMU (timeout: ${timeout}s)..."
    log_info "Expected token: $token"
    
    if [[ "$verbose" == "true" ]]; then
        # Verbose mode: show output and use timeout
        timeout "$timeout" qemu-system-x86_64 "${qemu_args[@]}" &
        local qemu_pid=$!
    else
        # Background mode: poll serial log
        qemu-system-x86_64 "${qemu_args[@]}" &
        local qemu_pid=$!
    fi
    
    log_info "QEMU started (PID: $qemu_pid)"
    
    # Poll for boot token
    local start_time
    start_time=$(date +%s)
    
    while true; do
        local elapsed=$(($(date +%s) - start_time))
        
        # Check timeout
        if [[ $elapsed -ge $timeout ]]; then
            log_error "Timeout after ${timeout}s"
            kill "$qemu_pid" 2>/dev/null || true
            return 1
        fi
        
        # Check for token in serial log
        if [[ -f "$serial_log" ]] && grep -q "$token" "$serial_log" 2>/dev/null; then
            log_ok "Boot token found after ${elapsed}s!"
            kill "$qemu_pid" 2>/dev/null || true
            return 0
        fi
        
        # Check if QEMU is still running
        if ! kill -0 "$qemu_pid" 2>/dev/null; then
            log_warn "QEMU exited"
            
            # Check if token was emitted before exit
            if [[ -f "$serial_log" ]] && grep -q "$token" "$serial_log" 2>/dev/null; then
                log_ok "Boot token found (QEMU exited)"
                return 0
            fi
            
            return 1
        fi
        
        sleep 1
        [[ "$verbose" != "true" ]] && printf "."
    done
}

# Main
main() {
    local efi_file=""
    local ovmf_path=""
    local timeout=$DEFAULT_TIMEOUT
    local token=$DEFAULT_TOKEN
    local keep_esp=false
    local verbose=false
    local use_kvm="auto"
    
    # Parse arguments
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --ovmf)
                ovmf_path="$2"
                shift 2
                ;;
            --timeout)
                timeout="$2"
                shift 2
                ;;
            --token)
                token="$2"
                shift 2
                ;;
            --keep-esp)
                keep_esp=true
                shift
                ;;
            --verbose)
                verbose=true
                shift
                ;;
            --kvm)
                use_kvm="force"
                shift
                ;;
            --no-kvm)
                use_kvm="none"
                shift
                ;;
            --help|-h)
                usage
                exit 0
                ;;
            -*)
                log_error "Unknown option: $1"
                usage
                exit 2
                ;;
            *)
                if [[ -z "$efi_file" ]]; then
                    efi_file="$1"
                else
                    log_error "Unexpected argument: $1"
                    exit 2
                fi
                shift
                ;;
        esac
    done
    
    # Validate arguments
    if [[ -z "$efi_file" ]]; then
        log_error "No EFI file specified"
        usage
        exit 2
    fi
    
    if [[ ! -f "$efi_file" ]]; then
        log_error "EFI file not found: $efi_file"
        exit 2
    fi
    
    # Check dependencies
    check_deps
    
    # Find OVMF
    if [[ -z "$ovmf_path" ]]; then
        if ! ovmf_path=$(find_ovmf); then
            log_error "OVMF firmware not found"
            log_error "Install with: sudo apt-get install ovmf"
            exit 2
        fi
    fi
    
    if [[ ! -f "$ovmf_path" ]]; then
        log_error "OVMF not found at: $ovmf_path"
        exit 2
    fi
    
    log_info "OVMF: $ovmf_path"
    log_info "EFI: $efi_file"
    
    # Create temporary files
    local esp_img
    esp_img=$(mktemp --suffix=.img)
    local serial_log
    serial_log=$(mktemp --suffix=.log)
    
    # Cleanup trap
    cleanup() {
        [[ "$keep_esp" != "true" ]] && rm -f "$esp_img"
        if [[ -f "$serial_log" ]]; then
            echo ""
            echo "=== Serial Log ==="
            cat "$serial_log"
            rm -f "$serial_log"
        fi
    }
    trap cleanup EXIT
    
    # Create ESP image
    create_esp_image "$efi_file" "$esp_img"
    
    # Run QEMU
    echo ""
    if run_qemu "$ovmf_path" "$esp_img" "$timeout" "$token" "$use_kvm" "$verbose" "$serial_log"; then
        echo ""
        log_ok "=== E2E TEST PASSED ==="
        exit 0
    else
        echo ""
        log_error "=== E2E TEST FAILED ==="
        exit 1
    fi
}

main "$@"
