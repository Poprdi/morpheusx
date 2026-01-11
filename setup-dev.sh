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
check_tails()      { [[ -f "${ESP_DIR}/kernels/vmlinuz-tails" ]]; }
check_arch()       { [[ -f "${ESP_DIR}/kernels/vmlinuz-arch" ]]; }
check_any_distro() { check_tails || check_arch || compgen -G "${ESP_DIR}/kernels/vmlinuz-*" > /dev/null 2>&1; }
check_disk_50g()   { [[ -f "${TESTING_DIR}/test-disk-50g.img" ]]; }

ensure_dirs() {
    mkdir -p "${ESP_DIR}"/{EFI/BOOT,kernels,initrds,loader/entries,.iso}
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
    
    printf "\n${C_DIM}── Distributions ──${C_RESET}\n"
    print_check "$(check_tails && echo 1 || echo 0)" "Tails OS"
    print_check "$(check_arch && echo 1 || echo 0)" "Arch Linux"
    
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

do_install_tails() {
    log_step "Installing Tails OS"
    
    if check_tails && [[ "${FORCE_MODE}" != "true" ]]; then
        log_success "Tails already installed"
        return 0
    fi
    
    pushd "${TESTING_DIR}" >/dev/null
    yes y | ./install-tails.sh || ./install-tails.sh
    popd >/dev/null
    
    log_success "Tails installed"
}

do_install_arch() {
    log_step "Installing Arch Linux"
    
    if check_arch && [[ "${FORCE_MODE}" != "true" ]]; then
        log_success "Arch already installed"
        return 0
    fi
    
    pushd "${TESTING_DIR}" >/dev/null
    ./install-arch.sh
    popd >/dev/null
    
    log_success "Arch installed"
}

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
    
    log_info "Creating GPT partition table..."
    parted -s "$disk_img" mklabel gpt
    parted -s "$disk_img" mkpart primary fat32 1MiB 4GiB
    parted -s "$disk_img" set 1 esp on
    parted -s "$disk_img" mkpart primary ext4 4GiB 100%
    
    log_info "Setting up partitions..."
    local loop_dev
    loop_dev=$(sudo losetup -fP --show "$disk_img")
    
    trap "sudo losetup -d '$loop_dev' 2>/dev/null || true" EXIT
    
    sudo mkfs.vfat -F 32 -n "ESP" "${loop_dev}p1" >/dev/null
    sudo mkfs.ext4 -q -L "MORPHEUS_DATA" "${loop_dev}p2" >/dev/null
    
    local mnt
    mnt=$(mktemp -d)
    sudo mount "${loop_dev}p1" "$mnt"
    
    sudo mkdir -p "$mnt"/{EFI/BOOT,kernels,initrds,live,loader/entries,.iso}
    
    [[ -f "${ESP_DIR}/EFI/BOOT/BOOTX64.EFI" ]] && sudo cp "${ESP_DIR}/EFI/BOOT/BOOTX64.EFI" "$mnt/EFI/BOOT/"
    
    if [[ -f "${ESP_DIR}/initrds/filesystem.squashfs" ]]; then
        log_info "Copying Tails squashfs (~2GB, this takes a moment)..."
        sudo cp "${ESP_DIR}/initrds/filesystem.squashfs" "$mnt/live/"
    fi

    if compgen -G "${ESP_DIR}/.iso/*.iso" > /dev/null 2>&1; then
        log_info "Copying ISO images into ESP .iso/ (for bootloader ISO scan)..."
        sudo cp "${ESP_DIR}/.iso/"*.iso "$mnt/.iso/"
    else
        log_warn "No ISO images found in ${ESP_DIR}/.iso to copy"
    fi
    
    for kernel in "${ESP_DIR}"/kernels/vmlinuz-*; do
        [[ -f "$kernel" ]] || continue
        local kname distro
        kname=$(basename "$kernel")
        distro=${kname#vmlinuz-}
        
        sudo cp "$kernel" "$mnt/kernels/"
        [[ -f "${ESP_DIR}/initrds/initrd-${distro}.img" ]] && sudo cp "${ESP_DIR}/initrds/initrd-${distro}.img" "$mnt/initrds/"
        
        local title="$distro" cmdline="console=ttyS0,115200 console=tty0"
        case "$distro" in
            tails)  title="Tails OS"; cmdline="boot=live live-media-path=/live nopersistence noprompt splash=0 $cmdline" ;;
            arch)   title="Arch Linux"; cmdline="root=/dev/ram0 rw debug $cmdline" ;;
            ubuntu) title="Ubuntu"; cmdline="boot=casper quiet splash $cmdline" ;;
            debian) title="Debian"; cmdline="boot=live quiet $cmdline" ;;
            fedora) title="Fedora"; cmdline="rd.live.image quiet $cmdline" ;;
        esac
        
        local initrd_line=""
        [[ -f "$mnt/initrds/initrd-${distro}.img" ]] && initrd_line="initrd  /initrds/initrd-${distro}.img"
        
        sudo tee "$mnt/loader/entries/${distro}.conf" > /dev/null << EOF
title   $title
linux   /kernels/$kname
$initrd_line
options $cmdline
EOF
        log_info "Added boot entry: $title"
    done
    
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
            -bios "$ovmf_path" \
            -drive file="${TESTING_DIR}/test-disk-50g.img",format=raw,if=none,id=disk0 \
            -device virtio-blk-pci,drive=disk0,bus=pci.0,addr=0x04,disable-legacy=on \
            -device virtio-net-pci,netdev=net0,bus=pci.0,addr=0x03,disable-legacy=on \
            -netdev user,id=net0,hostfwd=tcp::2222-:22 \
            -smp 4 \
            -m 4G \
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
            -bios "$ovmf_path" \
            -drive file="$esp_img",format=raw,if=none,id=disk0 \
            -device virtio-blk-pci,drive=disk0,bus=pci.0,addr=0x04,disable-legacy=on \
            -device virtio-net-pci,netdev=net0,bus=pci.0,addr=0x03,disable-legacy=on \
            -netdev user,id=net0,hostfwd=tcp::2222-:22 \
            -smp 4 \
            -m 4G \
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
    
    if ! check_any_distro; then
        log_step "Installing Distributions"
        log_info "Installing Tails OS (this downloads ~1.3GB)..."
        do_install_tails
    else
        log_success "Distributions ready"
    fi
    
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
    
    printf "\n${C_BOLD}Which distributions to install?${C_RESET}\n"
    printf "  ${C_CYAN}[1]${C_RESET} Tails OS only (~1.3GB)\n"
    printf "  ${C_CYAN}[2]${C_RESET} Arch Linux only (~2GB)\n"
    printf "  ${C_CYAN}[3]${C_RESET} Both Tails and Arch\n"
    printf "  ${C_CYAN}[4]${C_RESET} Skip - I'll add distros later\n"
    printf "Choice [1]: "
    read -r distro_choice
    [[ -z "$distro_choice" ]] && distro_choice=1
    
    case "$distro_choice" in
        1) do_install_tails ;;
        2) do_install_arch ;;
        3) do_install_tails; do_install_arch ;;
        4) log_info "Skipping distributions" ;;
    esac
    
    if ask "Create 50GB test disk with bootloader and distros?"; then
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

cmd_install() {
    local target="${1:-}"
    print_banner
    
    case "$target" in
        tails)  FORCE_MODE=true; do_install_tails ;;
        arch)   FORCE_MODE=true; do_install_arch ;;
        both|all) FORCE_MODE=true; do_install_tails; do_install_arch ;;
        "")
            printf "Usage: %s install <target>\n\n" "$(basename "$0")"
            printf "Targets:\n"
            printf "  ${C_CYAN}tails${C_RESET}   Install Tails OS\n"
            printf "  ${C_CYAN}arch${C_RESET}    Install Arch Linux\n"
            printf "  ${C_CYAN}both${C_RESET}    Install both\n"
            return 1
            ;;
        *) die "Unknown target: $target" ;;
    esac
}

cmd_run() {
    print_banner
    check_bootloader || die "Bootloader not built. Run: $0 build"
    do_launch_qemu
}

cmd_clean() {
    print_banner
    log_step "Cleaning"
    
    printf "What to clean?\n"
    printf "  ${C_CYAN}[1]${C_RESET} Build artifacts only\n"
    printf "  ${C_CYAN}[2]${C_RESET} Disk images only\n"
    printf "  ${C_CYAN}[3]${C_RESET} Distributions only\n"
    printf "  ${C_CYAN}[4]${C_RESET} Everything\n"
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
            rm -f "${ESP_DIR}/kernels"/*
            rm -f "${ESP_DIR}/initrds"/*
            rm -rf "${ESP_DIR}/rootfs"
            log_success "Distributions cleaned"
            ;;
        4)
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
    printf "  installs deps, builds, downloads distros, creates disk, launches QEMU\n\n"
    
    printf "${C_BOLD}Options:${C_RESET}\n"
    printf "  ${C_CYAN}-i, --interactive${C_RESET}  Ask at each step what to do\n"
    printf "  ${C_CYAN}-f, --force${C_RESET}        Force rebuild/recreate\n"
    printf "  ${C_CYAN}-n, --no-qemu${C_RESET}      Setup everything but don't launch QEMU\n"
    printf "  ${C_CYAN}-h, --help${C_RESET}         Show this help\n"
    
    printf "\n${C_BOLD}Commands:${C_RESET} (for power users)\n"
    printf "  ${C_CYAN}setup${C_RESET}              Install dependencies only\n"
    printf "  ${C_CYAN}build${C_RESET}              Build bootloader only\n"
    printf "  ${C_CYAN}install${C_RESET} <target>   Install distro (tails|arch|both)\n"
    printf "  ${C_CYAN}disk${C_RESET} [target]      Create disk image (50g|info)\n"
    printf "  ${C_CYAN}run${C_RESET}                Launch QEMU\n"
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
        install|add)     cmd_install "${args[@]:-}" ;;
        disk|image)      cmd_disk "${args[@]:-}" ;;
        run|start|qemu)  cmd_run ;;
        status|info)     cmd_status ;;
        clean|purge)     cmd_clean ;;
        *)               die "Unknown command: $cmd" ;;
    esac
}

main "$@"
