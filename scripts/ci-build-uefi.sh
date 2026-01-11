#!/usr/bin/env bash
# =============================================================================
# ci-build-uefi.sh - CI Build Script for UEFI Bootloader
# =============================================================================
#
# Purpose:
#   Wrapper script for the 2-pass UEFI build required for relocation embedding.
#   Used by CI workflows and local development.
#
# Why 2-pass build?
#   UEFI discards the .reloc section from memory after applying relocations.
#   MorpheusX needs the original relocation data to "unrelocate" itself for
#   persistence (self-replication). We extract the .reloc section from the
#   first build and embed it as data in the second build.
#
# Preconditions:
#   - Rust toolchain with x86_64-unknown-uefi target
#   - NASM assembler
#   - tools/extract-reloc-data.sh must be executable
#
# Usage:
#   ./ci-build-uefi.sh [options]
#
# Options:
#   --release         Build in release mode (default)
#   --debug           Build in debug mode
#   --clean           Clean target directory before building
#   --skip-pass1      Skip pass 1 (use existing binary for reloc extraction)
#   --output <dir>    Copy final .efi to this directory
#   --verbose         Show detailed build output
#
# Output:
#   target/x86_64-unknown-uefi/release/morpheus-bootloader.efi
#
# =============================================================================

set -euo pipefail

readonly SCRIPT_NAME="$(basename "$0")"
readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly PROJECT_ROOT="${SCRIPT_DIR}/.."

# Build configuration
TARGET="x86_64-unknown-uefi"
PACKAGE="morpheus-bootloader"
PROFILE="release"
CLEAN=false
SKIP_PASS1=false
OUTPUT_DIR=""
VERBOSE=false

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
log_step()  { echo -e "\n${BLUE}==>${NC} ${*}"; }

usage() {
    cat << EOF
Usage: $SCRIPT_NAME [options]

Build MorpheusX UEFI bootloader with 2-pass relocation embedding.

Options:
    --release         Build in release mode (default)
    --debug           Build in debug mode
    --clean           Clean target directory before building
    --skip-pass1      Skip pass 1 (use existing binary)
    --output <dir>    Copy final .efi to this directory
    --verbose         Show detailed build output
    --help            Show this help

Examples:
    $SCRIPT_NAME                           # Standard release build
    $SCRIPT_NAME --clean                   # Clean build
    $SCRIPT_NAME --output testing/esp/EFI/BOOT  # Build and deploy
EOF
}

# Check prerequisites
check_prereqs() {
    log_step "Checking prerequisites"
    
    local missing=()
    
    # Check Rust
    if ! command -v cargo &>/dev/null; then
        missing+=("cargo (Rust toolchain)")
    fi
    
    # Check NASM
    if ! command -v nasm &>/dev/null; then
        missing+=("nasm")
    fi
    
    # Check UEFI target
    if ! rustup target list 2>/dev/null | grep -q "$TARGET (installed)"; then
        log_warn "UEFI target not installed, installing..."
        rustup target add "$TARGET"
    fi
    
    # Check extract script
    if [[ ! -x "$PROJECT_ROOT/tools/extract-reloc-data.sh" ]]; then
        log_warn "Making extract-reloc-data.sh executable"
        chmod +x "$PROJECT_ROOT/tools/extract-reloc-data.sh"
    fi
    
    if [[ ${#missing[@]} -gt 0 ]]; then
        log_error "Missing prerequisites: ${missing[*]}"
        exit 1
    fi
    
    log_ok "All prerequisites satisfied"
}

# Run cargo build with optional verbosity
cargo_build() {
    local args=("build" "--target" "$TARGET" "-p" "$PACKAGE")
    
    if [[ "$PROFILE" == "release" ]]; then
        args+=("--release")
    fi
    
    if [[ "$VERBOSE" == "true" ]]; then
        cargo "${args[@]}"
    else
        cargo "${args[@]}" 2>&1 | grep -E "^(Compiling|Finished|error|warning)" || true
    fi
}

# Main build process
main() {
    cd "$PROJECT_ROOT"
    
    # Parse arguments
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --release)
                PROFILE="release"
                shift
                ;;
            --debug)
                PROFILE="debug"
                shift
                ;;
            --clean)
                CLEAN=true
                shift
                ;;
            --skip-pass1)
                SKIP_PASS1=true
                shift
                ;;
            --output)
                OUTPUT_DIR="$2"
                shift 2
                ;;
            --verbose)
                VERBOSE=true
                shift
                ;;
            --help|-h)
                usage
                exit 0
                ;;
            *)
                log_error "Unknown option: $1"
                usage
                exit 1
                ;;
        esac
    done
    
    echo ""
    log_info "=== MorpheusX UEFI Build (2-Pass) ==="
    log_info "Profile: $PROFILE"
    log_info "Target: $TARGET"
    echo ""
    
    # Check prerequisites
    check_prereqs
    
    # Clean if requested
    if [[ "$CLEAN" == "true" ]]; then
        log_step "Cleaning target directory"
        rm -rf target/
        log_ok "Target directory cleaned"
    fi
    
    # =========================================================================
    # PASS 1: Build bootloader to get binary for reloc extraction
    # =========================================================================
    if [[ "$SKIP_PASS1" != "true" ]]; then
        log_step "Pass 1: Building bootloader"
        
        if ! cargo_build; then
            log_error "Pass 1 build failed"
            exit 1
        fi
        
        log_ok "Pass 1 complete"
    else
        log_step "Skipping Pass 1 (using existing binary)"
    fi
    
    # Check that binary exists
    local efi_file="target/$TARGET/$PROFILE/morpheus-bootloader.efi"
    if [[ ! -f "$efi_file" ]]; then
        log_error "EFI file not found: $efi_file"
        log_error "Cannot extract relocation data without initial build"
        exit 1
    fi
    
    # =========================================================================
    # EXTRACT: Get relocation data from the built binary
    # =========================================================================
    log_step "Extracting relocation data"
    
    if ! ./tools/extract-reloc-data.sh "$efi_file"; then
        log_error "Relocation extraction failed"
        exit 1
    fi
    
    # Verify generated file
    local reloc_rs="persistent/src/pe/embedded_reloc_data.rs"
    if [[ ! -f "$reloc_rs" ]]; then
        log_error "Generated file not found: $reloc_rs"
        exit 1
    fi
    
    log_ok "Relocation data extracted"
    log_info "Generated: $reloc_rs"
    
    # =========================================================================
    # PASS 2: Rebuild with embedded relocation data
    # =========================================================================
    log_step "Pass 2: Rebuilding with embedded relocations"
    
    # Clean only the bootloader to force rebuild with new reloc data
    cargo clean -p "$PACKAGE" 2>/dev/null || true
    
    if ! cargo_build; then
        log_error "Pass 2 build failed"
        exit 1
    fi
    
    log_ok "Pass 2 complete"
    
    # =========================================================================
    # VERIFY: Check the final artifact
    # =========================================================================
    log_step "Verifying final artifact"
    
    if [[ ! -f "$efi_file" ]]; then
        log_error "Final EFI file not found!"
        exit 1
    fi
    
    # Check it's a valid PE file
    if ! file "$efi_file" | grep -q "PE32+"; then
        log_error "Output is not a valid PE32+ executable!"
        exit 1
    fi
    
    local size
    size=$(du -h "$efi_file" | cut -f1)
    log_ok "Built: $efi_file ($size)"
    
    # =========================================================================
    # DEPLOY: Copy to output directory if specified
    # =========================================================================
    if [[ -n "$OUTPUT_DIR" ]]; then
        log_step "Deploying to: $OUTPUT_DIR"
        
        mkdir -p "$OUTPUT_DIR"
        cp "$efi_file" "$OUTPUT_DIR/BOOTX64.EFI"
        
        log_ok "Deployed: $OUTPUT_DIR/BOOTX64.EFI"
    fi
    
    # =========================================================================
    # DONE
    # =========================================================================
    echo ""
    log_ok "=== Build Complete ==="
    echo ""
    log_info "Artifact: $efi_file"
    log_info "Size: $size"
    
    if [[ -n "$OUTPUT_DIR" ]]; then
        log_info "Deployed: $OUTPUT_DIR/BOOTX64.EFI"
    fi
    
    echo ""
    log_info "To test: ./scripts/qemu-e2e.sh $efi_file"
}

main "$@"
