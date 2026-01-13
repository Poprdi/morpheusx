#!/usr/bin/env bash

set -euo pipefail

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly PROJECT_ROOT="${SCRIPT_DIR}"
readonly TESTING_DIR="${PROJECT_ROOT}/testing"
readonly ESP_DIR="${TESTING_DIR}/esp"
readonly VERSION="2.0.0"

readonly C_RED='\033[0;31m'
readonly C_GREEN='\033[0;32m'
readonly C_YELLOW='\033[1;33m'
readonly C_BLUE='\033[0;34m'
readonly C_CYAN='\033[0;36m'
readonly C_BOLD='\033[1m'
readonly C_DIM='\033[2m'
readonly C_RESET='\033[0m'

readonly SYM_CHECK="✓"
readonly SYM_CROSS="✗"
readonly SYM_ARROW="➜"
readonly SYM_BULLET="•"

INTERACTIVE=false
FORCE_MODE=false
SKIP_QEMU=false

log_info()    { printf "${C_BLUE}${SYM_ARROW} %s${C_RESET}\n" "$1"; }
log_success() { printf "${C_GREEN}${SYM_CHECK} %s${C_RESET}\n" "$1"; }
log_warn()    { printf "${C_YELLOW}${SYM_BULLET} %s${C_RESET}\n" "$1" >&2; }
log_error()   { printf "${C_RED}${SYM_CROSS} %s${C_RESET}\n" "$1" >&2; }
log_step()    { printf "\n${C_BOLD}${C_BLUE}==>${C_RESET} ${C_BOLD}%s${C_RESET}\n" "$1"; }
die()         { log_error "$1"; exit 1; }

has_cmd() { command -v "$1" &>/dev/null; }

ask() {
    [[ "${INTERACTIVE}" != "true" ]] && return 0
    printf "${C_YELLOW}%s [Y/n] ${C_RESET}" "$1"
    read -r -n 1 response
    printf "\n"
    [[ ! "$response" =~ ^[nN]$ ]]
}

print_banner() {
    printf "${C_CYAN}"
    cat << 'BANNER'
    __  ___                 __                   _  __
   /  |/  /___  _________  / /_  ___  __  _____ | |/ /
  / /|_/ / __ \/ ___/ __ \/ __ \/ _ \/ / / / ___/|   / 
 / /  / / /_/ / /  / /_/ / / / /  __/ /_/ (__  )/   |  
/_/  /_/\____/_/  / .___/_/ /_/\___/\__,_/____//_/|_|  
                 /_/                                   
BANNER
    printf "${C_RESET}"
    printf "${C_DIM}Development Environment Bootstrap v${VERSION}${C_RESET}\n\n"
}

detect_distro() {
    [[ -f /etc/os-release ]] && { source /etc/os-release; echo "${ID}"; return; }
    echo "unknown"
}

get_ovmf_path() {
    local -a paths=(
        "/usr/share/OVMF/x64/OVMF.4m.fd"
        "/usr/share/OVMF/x64/OVMF_CODE.4m.fd"
        "/usr/share/edk2/x64/OVMF_CODE.4m.fd"
        "/usr/share/OVMF/OVMF_CODE.fd"
        "/usr/share/edk2/ovmf/OVMF_CODE.fd"
        "/usr/share/edk2-ovmf/OVMF_CODE.fd"
        "/usr/share/ovmf/OVMF.fd"
        "/usr/share/qemu/ovmf-x86_64-code.bin"
    )
    for p in "${paths[@]}"; do [[ -f "$p" ]] && { echo "$p"; return 0; }; done
    return 1
}

check_rust()       { has_cmd rustc && rustup target list 2>/dev/null | grep -q "x86_64-unknown-uefi (installed)"; }
check_ovmf()       { get_ovmf_path &>/dev/null; }
check_bootloader() { [[ -f "${ESP_DIR}/EFI/BOOT/BOOTX64.EFI" ]]; }
check_qemu()       { has_cmd qemu-system-x86_64; }
check_disk_tools() { has_cmd qemu-img && has_cmd parted && has_cmd mkfs.vfat && has_cmd mkfs.ext4; }
# Distribution checks removed - network downloader handles ISO acquisition
check_disk_50g()   { [[ -f "${TESTING_DIR}/test-disk-50g.img" ]]; }

ensure_dirs() {
    mkdir -p "${ESP_DIR}/EFI/BOOT"
    mkdir -p "${TESTING_DIR}"
}

print_check() {
    local ok=$1 name=$2 extra=${3:-}
    if [[ "$ok" == "1" ]]; then
        printf "  ${C_GREEN}${SYM_CHECK}${C_RESET} %-28s ${C_DIM}%s${C_RESET}\n" "$name" "$extra"
    else
        printf "  ${C_RED}${SYM_CROSS}${C_RESET} %-28s ${C_DIM}%s${C_RESET}\n" "$name" "$extra"
    fi
}

cmd_status() {
    print_banner
    printf "${C_BOLD}Environment Status:${C_RESET}\n\n"
    
    printf "${C_DIM}── Toolchain ──${C_RESET}\n"
    print_check "$(has_cmd rustc && echo 1 || echo 0)" "Rust Compiler" "$(rustc --version 2>/dev/null | cut -d' ' -f2 || echo 'missing')"
    print_check "$(check_rust && echo 1 || echo 0)" "UEFI Target"
    print_check "$(has_cmd nasm && echo 1 || echo 0)" "NASM Assembler"
    print_check "$(check_qemu && echo 1 || echo 0)" "QEMU" "$(qemu-system-x86_64 --version 2>/dev/null | head -1 | grep -oP '\d+\.\d+\.\d+' || echo 'missing')"
    print_check "$(check_ovmf && echo 1 || echo 0)" "OVMF Firmware" "$(get_ovmf_path 2>/dev/null || echo 'missing')"
    print_check "$(check_disk_tools && echo 1 || echo 0)" "Disk Tools" "parted, mkfs.vfat, mkfs.ext4"
    
    printf "\n${C_DIM}── Build ──${C_RESET}\n"
    print_check "$(check_bootloader && echo 1 || echo 0)" "Bootloader (BOOTX64.EFI)"
    
    printf "\n${C_DIM}── Disk Images ──${C_RESET}\n"
    print_check "$(check_disk_50g && echo 1 || echo 0)" "Test Disk 50GB"
    printf "\n"
}

do_install_packages() {
    log_step "Installing System Packages"
    
    local distro
    distro=$(detect_distro)
    local -a pkgs=()
    local install_cmd=""
    
    case "${distro}" in
        arch|manjaro|endeavouros)
            pkgs=(base-devel nasm qemu-full ovmf parted dosfstools e2fsprogs util-linux rsync curl wget squashfs-tools cdrtools)
            install_cmd="sudo pacman -S --needed --noconfirm"
            ;;
        debian|ubuntu|pop|linuxmint|kali)
            pkgs=(build-essential nasm qemu-system-x86 ovmf parted dosfstools e2fsprogs util-linux rsync curl wget squashfs-tools genisoimage qemu-utils)
            install_cmd="sudo apt-get install -y -qq"
            log_info "Updating package lists..."
            sudo apt-get update -qq
            ;;
        fedora)
            pkgs=(gcc make nasm qemu-system-x86 edk2-ovmf parted dosfstools e2fsprogs util-linux rsync curl wget squashfs-tools genisoimage qemu-img)
            install_cmd="sudo dnf install -y -q"
            ;;
        rhel|centos|almalinux|rocky)
            pkgs=(gcc make nasm qemu-kvm edk2-ovmf parted dosfstools e2fsprogs util-linux rsync curl wget squashfs-tools genisoimage)
            install_cmd="sudo yum install -y -q"
            ;;
        opensuse*|suse)
            pkgs=(gcc make nasm qemu-x86 qemu-ovmf-x86_64 parted dosfstools e2fsprogs util-linux rsync curl wget squashfs genisoimage)
            install_cmd="sudo zypper install -y"
            ;;
        alpine)
            pkgs=(build-base nasm qemu-system-x86_64 ovmf parted dosfstools e2fsprogs util-linux rsync curl wget squashfs-tools cdrkit bash)
            install_cmd="sudo apk add"
            ;;
        *)
            log_warn "Unknown distro: ${distro}"
            log_info "Please install: nasm qemu ovmf parted dosfstools e2fsprogs rsync curl wget"
            return 0
            ;;
    esac
    
    log_info "Detected: ${distro}"
    log_info "Installing packages..."
    
    if ! ${install_cmd} "${pkgs[@]}"; then
        log_warn "Some packages failed - retrying individually..."
        for pkg in "${pkgs[@]}"; do
            ${install_cmd} "$pkg" 2>/dev/null || log_warn "Failed: $pkg"
        done
    fi
    
    log_success "System packages ready"
}

do_install_rust() {
    log_step "Rust Toolchain"
    
    if ! has_cmd rustc; then
        log_info "Installing Rust..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
        source "$HOME/.cargo/env" 2>/dev/null || export PATH="$HOME/.cargo/bin:$PATH"
    fi
    
    if ! rustup target list 2>/dev/null | grep -q "x86_64-unknown-uefi (installed)"; then
        log_info "Adding UEFI target..."
        rustup target add x86_64-unknown-uefi
    fi
    
    log_success "Rust: $(rustc --version | cut -d' ' -f2)"
}

do_configure_ovmf() {
    log_step "OVMF Configuration"
    
    local ovmf_path
    ovmf_path=$(get_ovmf_path) || die "OVMF not found after package install"
    
    if [[ -f "${TESTING_DIR}/run.sh" ]]; then
        sed -i "s|/usr/share/OVMF/OVMF_CODE.fd|${ovmf_path}|g" "${TESTING_DIR}/run.sh" 2>/dev/null || true
        sed -i "s|/usr/share/edk2/ovmf/OVMF_CODE.fd|${ovmf_path}|g" "${TESTING_DIR}/run.sh" 2>/dev/null || true
    fi
    
    log_success "OVMF: ${ovmf_path}"
}

do_clean() {
    log_step "Cleaning Build Artifacts"
    
    log_info "Removing target directory..."
    rm -rf "${PROJECT_ROOT}/target"
    
    log_info "Removing any stale ESP bootloader..."
    rm -f "${ESP_DIR}/EFI/BOOT/BOOTX64.EFI"
    
    log_success "Build artifacts cleaned"
}

do_build() {
    log_step "Building MorpheusX Bootloader (Clean Build)"
    
    ensure_dirs
    
    # Always clean before building to avoid stale artifacts
    log_info "Performing clean build (removing all cached artifacts)..."
    rm -rf "${PROJECT_ROOT}/target"
    
    if check_bootloader && [[ "${FORCE_MODE}" != "true" ]]; then
        log_success "Bootloader already built"
        return 0
    fi
    
    pushd "${TESTING_DIR}" >/dev/null
    ./build.sh
    popd >/dev/null
    
    check_bootloader || die "Build failed"
    log_success "Bootloader ready"
}

# do_install_tails removed - use network downloader in bootloader TUI

# do_install_arch removed - use network downloader in bootloader TUI

do_create_disk() {
    log_step "Creating Test Disk"
    
    if check_disk_50g && [[ "${FORCE_MODE}" != "true" ]]; then
        log_success "Test disk already exists"
        return 0
    fi
    
    local disk_img="${TESTING_DIR}/test-disk-50g.img"
    
    log_info "Creating 50GB sparse disk image..."
    rm -f "$disk_img"
    qemu-img create -f raw "$disk_img" 50G >/dev/null
    
    log_info "Creating GPT partition table with ESP only..."
    parted -s "$disk_img" mklabel gpt
    parted -s "$disk_img" mkpart ESP fat32 1MiB 4GiB
    parted -s "$disk_img" set 1 esp on
    # Leave remaining space FREE for bootloader to create ISO partitions dynamically
    
    log_info "Setting up ESP partition..."
    local loop_dev
    loop_dev=$(sudo losetup -fP --show "$disk_img")
    
    trap "sudo losetup -d '$loop_dev' 2>/dev/null || true" EXIT
    
    sudo mkfs.vfat -F 32 -n "ESP" "${loop_dev}p1" >/dev/null
    # No second partition - bootloader creates ISO partitions on-demand
    
    local mnt
    mnt=$(mktemp -d)
    sudo mount "${loop_dev}p1" "$mnt"
    
    sudo mkdir -p "$mnt/EFI/BOOT"
    
    [[ -f "${ESP_DIR}/EFI/BOOT/BOOTX64.EFI" ]] && sudo cp "${ESP_DIR}/EFI/BOOT/BOOTX64.EFI" "$mnt/EFI/BOOT/"
    
    log_info "Disk ready - use network downloader in bootloader TUI to fetch distributions"
    
    sudo umount "$mnt"
    rmdir "$mnt"
    
    # Force filesystem sync and ensure loop device is fully detached
    sync
    sleep 1
    
    sudo losetup -d "$loop_dev" 2>/dev/null || {
        log_warn "Failed to detach loop device, trying harder..."
        sleep 2
        sudo losetup -d "$loop_dev" 2>/dev/null || true
    }
    trap - EXIT
    
    # Final sync before QEMU uses the disk
    sync
    sleep 2
    
    log_success "Test disk ready: $(du -h "$disk_img" | cut -f1) (sparse)"
}

do_launch_qemu() {
    log_step "Launching QEMU"
    
    local ovmf_path
    ovmf_path=$(get_ovmf_path) || die "OVMF not found"
    
    local disk="${TESTING_DIR}/test-disk-50g.img"
    [[ -f "$disk" ]] || disk="${ESP_DIR}"
    
    log_info "Starting MorpheusX..."
    log_info "Press Ctrl+A X to exit QEMU"
    printf "\n"
    
    if [[ -f "${TESTING_DIR}/test-disk-50g.img" ]]; then
        qemu-system-x86_64 \
            -enable-kvm \
            -machine q35,accel=kvm \
            -cpu host \
            -bios "$ovmf_path" \
            -object iothread,id=iothread0 \
            -drive file="${TESTING_DIR}/test-disk-50g.img",format=raw,if=none,id=disk0,cache=writeback \
            -device virtio-blk-pci,drive=disk0,disable-legacy=on,iothread=iothread0 \
            -device virtio-net-pci,netdev=net0,disable-legacy=on \
            -netdev user,id=net0,hostfwd=tcp::2222-:22 \
            -smp 8 \
            -m 12G \
            -vga virtio \
            -display gtk,gl=on \
            -serial stdio
    else
        # Create a temp ESP image for virtio-blk
        local esp_img="${TESTING_DIR}/esp-temp.img"
        if [[ ! -f "$esp_img" ]] || [[ "${ESP_DIR}" -nt "$esp_img" ]]; then
            log_info "Creating ESP disk image..."
            local esp_size=$(du -sb "${ESP_DIR}" | awk '{print int(($1 / 1024 / 1024) + 50)}')
            dd if=/dev/zero of="$esp_img" bs=1M count=$esp_size status=none 2>/dev/null || true
            mkfs.vfat -F 32 -n ESP "$esp_img" >/dev/null 2>&1 || true
            local mnt=$(mktemp -d)
            sudo mount -o loop "$esp_img" "$mnt"
            sudo rsync -a "${ESP_DIR}/" "$mnt/" 2>/dev/null || true
            sudo umount "$mnt"
            rmdir "$mnt"
        fi
        qemu-system-x86_64 \
            -enable-kvm \
            -machine q35,accel=kvm \
            -cpu host \
            -bios "$ovmf_path" \
            -object iothread,id=iothread0 \
            -drive file="$esp_img",format=raw,if=none,id=disk0,cache=writeback \
            -device virtio-blk-pci,drive=disk0,disable-legacy=on,iothread=iothread0 \
            -device virtio-net-pci,netdev=net0,disable-legacy=on \
            -netdev user,id=net0,hostfwd=tcp::2222-:22 \
            -smp 8 \
            -m 12G \
            -vga virtio \
            -display gtk,gl=on \
            -serial stdio
    fi
}

run_full_auto() {
    print_banner
    
    log_info "Full automatic setup - sit back and relax"
    printf "\n"
    
    if ! check_disk_tools || ! check_qemu; then
        do_install_packages
    else
        log_success "System packages OK"
    fi
    
    if ! check_rust; then
        do_install_rust
    else
        log_success "Rust toolchain OK"
    fi
    
    if ! check_ovmf; then
        die "OVMF not found after setup"
    fi
    do_configure_ovmf
    
    do_build
    
    # Distributions are now downloaded via network downloader in bootloader TUI
    
    do_create_disk
    
    printf "\n${C_GREEN}${C_BOLD}${SYM_CHECK} Setup complete!${C_RESET}\n\n"
    
    # Kill any lingering QEMU processes before launching new one
    pkill -9 qemu-system-x86_64 2>/dev/null || true
    sleep 2
    
    if [[ "${SKIP_QEMU}" != "true" ]]; then
        do_launch_qemu
    fi
}

run_interactive() {
    print_banner
    
    log_info "Interactive setup - I'll ask you at each step"
    printf "\n"
    
    if ! check_disk_tools || ! check_qemu; then
        if ask "Install system packages (qemu, parted, etc)?"; then
            do_install_packages
        fi
    else
        log_success "System packages OK"
    fi
    
    if ! check_rust; then
        if ask "Install Rust toolchain?"; then
            do_install_rust
        fi
    else
        log_success "Rust toolchain OK"
    fi
    
    if check_ovmf; then
        do_configure_ovmf
    else
        log_warn "OVMF not found - QEMU boot will fail"
    fi
    
    if ask "Build MorpheusX bootloader?"; then
        FORCE_MODE=true
        do_build
    fi
    
    # Distributions are now downloaded via network downloader in bootloader TUI
    
    if ask "Create 50GB test disk with bootloader?"; then
        FORCE_MODE=true
        do_create_disk
    fi
    
    printf "\n${C_GREEN}${C_BOLD}${SYM_CHECK} Setup complete!${C_RESET}\n\n"
    
    if ask "Launch QEMU now?"; then
        do_launch_qemu
    fi
}

cmd_setup() {
    print_banner
    do_install_packages
    do_install_rust
    do_configure_ovmf
    ensure_dirs
    printf "\n${C_GREEN}${C_BOLD}${SYM_CHECK} Environment ready!${C_RESET}\n"
}

cmd_build() {
    print_banner
    check_rust || die "Rust not installed. Run: $0 setup"
    ensure_dirs
    FORCE_MODE=true
    do_build
}

cmd_disk() {
    local target="${1:-50g}"
    print_banner
    check_disk_tools || die "Disk tools missing. Run: $0 setup"
    
    case "$target" in
        50g|50G|all) FORCE_MODE=true; do_create_disk ;;
        info)
            printf "${C_BOLD}Disk Images:${C_RESET}\n\n"
            for img in "${TESTING_DIR}"/*.img; do
                [[ -f "$img" ]] || continue
                printf "  ${C_CYAN}%s${C_RESET}\n" "$(basename "$img")"
                printf "    Size: %s\n" "$(du -h "$img" | cut -f1)"
                printf "    Actual: %s\n" "$(stat --printf='%s' "$img" | numfmt --to=iec)"
            done
            ;;
        *) log_error "Unknown target: $target"; return 1 ;;
    esac
}

# cmd_install removed - distributions are downloaded via network downloader in bootloader TUI

cmd_run() {
    print_banner
    check_bootloader || die "Bootloader not built. Run: $0 build"
    do_launch_qemu
}

# ══════════════════════════════════════════════════════════════════════════════
# ThinkPad T450s Hardware Simulation
# Simulates real hardware: Intel AHCI SATA + Intel e1000 NIC
# Use this to test the unified device layer before deploying to real hardware
# ══════════════════════════════════════════════════════════════════════════════

do_launch_thinkpad() {
    log_step "Launching QEMU (ThinkPad T450s Hardware Simulation)"
    
    local ovmf_path
    ovmf_path=$(get_ovmf_path) || die "OVMF not found"
    
    local disk="${TESTING_DIR}/test-disk-50g.img"
    
    log_info "Hardware emulation:"
    log_info "  • Storage: Intel AHCI SATA (ICH9)"
    log_info "  • Network: Intel e1000 (82540EM)"
    log_info "  • Chipset: Q35 (PCH-like)"
    log_info ""
    log_info "This matches ThinkPad T450s hardware:"
    log_info "  • Intel Wildcat Point-LP SATA (0x8086:0x9C83)"
    log_info "  • Intel I218-LM Ethernet (0x8086:0x155A)"
    log_info ""
    log_info "Press Ctrl+A X to exit QEMU"
    printf "\n"
    
    if [[ -f "${TESTING_DIR}/test-disk-50g.img" ]]; then
        qemu-system-x86_64 \
            -enable-kvm \
            -machine q35,accel=kvm \
            -cpu host \
            -bios "$ovmf_path" \
            -smbios type=0,vendor=LENOVO,version=JBET71WW,date=03/01/2019 \
            -smbios type=1,manufacturer=LENOVO,product="ThinkPad T450s",version=ThinkPad,serial=PC0XXXXX,uuid=12345678-1234-1234-1234-123456789abc \
            -smbios type=2,manufacturer=LENOVO,product=20BWS0XX00 \
            -smbios type=3,manufacturer=LENOVO \
            -object iothread,id=iothread0 \
            -drive file="${TESTING_DIR}/test-disk-50g.img",format=raw,if=none,id=disk0,cache=writeback \
            -device ich9-ahci,id=ahci0 \
            -device ide-hd,drive=disk0,bus=ahci0.0,bootindex=1 \
            -device e1000,netdev=net0 \
            -netdev user,id=net0 \
            -smp 8 \
            -m 12G \
            -vga virtio \
            -display gtk,gl=on \
            -serial stdio
    else
        # Create a temp ESP image for AHCI
        local esp_img="${TESTING_DIR}/esp-temp.img"
        if [[ ! -f "$esp_img" ]] || [[ "${ESP_DIR}" -nt "$esp_img" ]]; then
            log_info "Creating ESP disk image..."
            local esp_size=$(du -sb "${ESP_DIR}" | awk '{print int(($1 / 1024 / 1024) + 50)}')
            dd if=/dev/zero of="$esp_img" bs=1M count=$esp_size status=none 2>/dev/null || true
            mkfs.vfat -F 32 -n ESP "$esp_img" >/dev/null 2>&1 || true
            local mnt=$(mktemp -d)
            sudo mount -o loop "$esp_img" "$mnt"
            sudo rsync -a "${ESP_DIR}/" "$mnt/" 2>/dev/null || true
            sudo umount "$mnt"
            rmdir "$mnt"
        fi
        qemu-system-x86_64 \
            -enable-kvm \
            -machine q35,accel=kvm \
            -cpu host \
            -bios "$ovmf_path" \
            -smbios type=0,vendor=LENOVO,version=JBET71WW,date=03/01/2019 \
            -smbios type=1,manufacturer=LENOVO,product="ThinkPad T450s",version=ThinkPad,serial=PC0XXXXX,uuid=12345678-1234-1234-1234-123456789abc \
            -smbios type=2,manufacturer=LENOVO,product=20BWS0XX00 \
            -smbios type=3,manufacturer=LENOVO \
            -object iothread,id=iothread0 \
            -drive file="$esp_img",format=raw,if=none,id=disk0,cache=writeback \
            -device ich9-ahci,id=ahci0 \
            -device ide-hd,drive=disk0,bus=ahci0.0,bootindex=1 \
            -device e1000,netdev=net0 \
            -netdev user,id=net0 \
            -smp 8 \
            -m 12G \
            -vga virtio \
            -display gtk,gl=on \
            -serial stdio
    fi
}

cmd_thinkpad() {
    print_banner
    check_bootloader || die "Bootloader not built. Run: $0 build"
    
    log_info "ThinkPad T450s Hardware Test Mode"
    log_info "Testing unified device layer with real hardware emulation"
    printf "\n"
    
    do_launch_thinkpad
}

cmd_clean() {
    print_banner
    log_step "Cleaning"
    
    printf "What to clean?\n"
    printf "  ${C_CYAN}[1]${C_RESET} Build artifacts only\n"
    printf "  ${C_CYAN}[2]${C_RESET} Disk images only\n"
    printf "  ${C_CYAN}[3]${C_RESET} Everything\n"
    printf "  ${C_CYAN}[0]${C_RESET} Cancel\n\n"
    
    read -r -n 1 -p "Choice: " choice
    printf "\n\n"
    
    case "$choice" in
        1)
            rm -rf "${PROJECT_ROOT}/target"
            rm -f "${ESP_DIR}/EFI/BOOT/BOOTX64.EFI"
            log_success "Build artifacts cleaned"
            ;;
        2)
            rm -f "${TESTING_DIR}"/*.img
            log_success "Disk images cleaned"
            ;;
        3)
            rm -rf "${PROJECT_ROOT}/target"
            rm -f "${ESP_DIR}/EFI/BOOT/BOOTX64.EFI"
            rm -f "${TESTING_DIR}"/*.img
            rm -f "${ESP_DIR}/kernels"/*
            rm -f "${ESP_DIR}/initrds"/*
            rm -rf "${ESP_DIR}/rootfs"
            log_success "Everything cleaned"
            ;;
        0|*) log_info "Cancelled" ;;
    esac
}

usage() {
    print_banner
    printf "${C_BOLD}Usage:${C_RESET} %s [options] [command]\n\n" "$(basename "$0")"
    
    printf "${C_BOLD}Default Behavior:${C_RESET}\n"
    printf "  Running without arguments does EVERYTHING automatically:\n"
    printf "  installs deps, builds, creates disk, launches QEMU\n"
    printf "  ${C_DIM}(Use network downloader in bootloader TUI to fetch ISOs)${C_RESET}\n\n"
    
    printf "${C_BOLD}Options:${C_RESET}\n"
    printf "  ${C_CYAN}-i, --interactive${C_RESET}  Ask at each step what to do\n"
    printf "  ${C_CYAN}-f, --force${C_RESET}        Force rebuild/recreate\n"
    printf "  ${C_CYAN}-n, --no-qemu${C_RESET}      Setup everything but don't launch QEMU\n"
    printf "  ${C_CYAN}-h, --help${C_RESET}         Show this help\n"
    
    printf "\n${C_BOLD}Commands:${C_RESET} (for power users)\n"
    printf "  ${C_CYAN}setup${C_RESET}              Install dependencies only\n"
    printf "  ${C_CYAN}build${C_RESET}              Build bootloader only\n"
    printf "  ${C_CYAN}disk${C_RESET} [target]      Create disk image (50g|info)\n"
    printf "  ${C_CYAN}run${C_RESET}                Launch QEMU (VirtIO devices)\n"
    printf "  ${C_CYAN}thinkpad${C_RESET}           Launch QEMU with ThinkPad T450s hardware\n"
    printf "  ${C_CYAN}status${C_RESET}             Show environment status\n"
    printf "  ${C_CYAN}clean${C_RESET}              Remove artifacts\n"
    
    printf "\n${C_BOLD}Examples:${C_RESET}\n"
    printf "  ${C_DIM}# Complete auto-setup + launch${C_RESET}\n"
    printf "  %s\n\n" "$(basename "$0")"
    printf "  ${C_DIM}# Interactive guided setup${C_RESET}\n"
    printf "  %s --interactive\n\n" "$(basename "$0")"
    printf "  ${C_DIM}# Setup without launching QEMU${C_RESET}\n"
    printf "  %s --no-qemu\n\n" "$(basename "$0")"
    printf "  ${C_DIM}# Just rebuild${C_RESET}\n"
    printf "  %s build\n\n" "$(basename "$0")"
    printf "  ${C_DIM}# Test with ThinkPad T450s hardware (AHCI + Intel e1000)${C_RESET}\n"
    printf "  %s thinkpad\n\n" "$(basename "$0")"
}

main() {
    local cmd=""
    local -a args=()
    
    while [[ $# -gt 0 ]]; do
        case "$1" in
            -i|--interactive) INTERACTIVE=true; shift ;;
            -f|--force)       FORCE_MODE=true; shift ;;
            -n|--no-qemu)     SKIP_QEMU=true; shift ;;
            -h|--help)        usage; exit 0 ;;
            -*)               die "Unknown option: $1" ;;
            *)                [[ -z "$cmd" ]] && cmd="$1" || args+=("$1"); shift ;;
        esac
    done
    
    case "${cmd:-}" in
        "")
            if [[ "${INTERACTIVE}" == "true" ]]; then
                run_interactive
            else
                run_full_auto
            fi
            ;;
        setup|init)      cmd_setup ;;
        build|compile)   cmd_build ;;
        disk|image)      cmd_disk "${args[@]:-}" ;;
        run|start|qemu)  cmd_run ;;
        thinkpad|t450s)  cmd_thinkpad ;;
        status|info)     cmd_status ;;
        clean|purge)     cmd_clean ;;
        *)               die "Unknown command: $cmd" ;;
    esac
}

main "$@"
