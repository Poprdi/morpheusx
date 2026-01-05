#!/usr/bin/env bash

set -euo pipefail
IFS=$'\n\t'

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly PROJECT_ROOT="${SCRIPT_DIR}"
readonly TESTING_DIR="${PROJECT_ROOT}/testing"
readonly ESP_DIR="${TESTING_DIR}/esp"
readonly VERSION="1.1.0"

readonly C_RED='\033[0;31m'
readonly C_GREEN='\033[0;32m'
readonly C_YELLOW='\033[1;33m'
readonly C_BLUE='\033[0;34m'
readonly C_CYAN='\033[0;36m'
readonly C_MAGENTA='\033[0;35m'
readonly C_BOLD='\033[1m'
readonly C_DIM='\033[2m'
readonly C_RESET='\033[0m'

readonly SYM_CHECK="✓"
readonly SYM_CROSS="✗"
readonly SYM_ARROW="➜"
readonly SYM_BULLET="•"

FORCE_MODE=false
AUTO_MODE=false
VERBOSE=false

log_info()    { printf "${C_BLUE}${SYM_ARROW} %s${C_RESET}\n" "$1"; }
log_success() { printf "${C_GREEN}${SYM_CHECK} %s${C_RESET}\n" "$1"; }
log_warn()    { printf "${C_YELLOW}${SYM_BULLET} %s${C_RESET}\n" "$1" >&2; }
log_error()   { printf "${C_RED}${SYM_CROSS} %s${C_RESET}\n" "$1" >&2; }
log_step()    { printf "\n${C_BOLD}${C_BLUE}==>${C_RESET} ${C_BOLD}%s${C_RESET}\n" "$1"; }
die()         { log_error "$1"; exit 1; }

has_cmd() { command -v "$1" &>/dev/null; }

confirm() {
    [[ "${AUTO_MODE}" == "true" ]] && return 0
    printf "${C_YELLOW}%s [y/N] ${C_RESET}" "$1"
    read -r -n 1 response
    printf "\n"
    [[ "$response" =~ ^[yY]$ ]]
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
    printf "${C_DIM}Development Environment Manager v${VERSION}${C_RESET}\n\n"
}

detect_distro() {
    [[ -f /etc/os-release ]] && { source /etc/os-release; echo "${ID}"; return; }
    echo "unknown"
}

check_rust()      { has_cmd rustc && rustup target list 2>/dev/null | grep -q "x86_64-unknown-uefi (installed)"; }
check_ovmf()      { get_ovmf_path &>/dev/null; }
check_tails()     { [[ -f "${ESP_DIR}/kernels/vmlinuz-tails" ]]; }
check_arch()      { [[ -f "${ESP_DIR}/kernels/vmlinuz-arch" ]]; }
check_disk_50g()  { [[ -f "${TESTING_DIR}/test-disk-50g.img" ]]; }
check_disk_10g()  { [[ -f "${TESTING_DIR}/test-disk-10g.img" ]]; }
check_esp_img()   { [[ -f "${TESTING_DIR}/esp.img" ]]; }
check_bootloader(){ [[ -f "${ESP_DIR}/EFI/BOOT/BOOTX64.EFI" ]]; }
check_qemu()      { has_cmd qemu-system-x86_64; }

get_ovmf_path() {
    local -a paths=(
        "/usr/share/OVMF/OVMF_CODE.fd"
        "/usr/share/edk2/ovmf/OVMF_CODE.fd"
        "/usr/share/edk2-ovmf/OVMF_CODE.fd"
        "/usr/share/ovmf/OVMF.fd"
        "/usr/share/qemu/ovmf-x86_64-code.bin"
    )
    for p in "${paths[@]}"; do [[ -f "$p" ]] && { echo "$p"; return 0; }; done
    return 1
}

status_line() {
    local name=$1 check=$2 extra=${3:-}
    if eval "$check"; then
        printf "  ${C_GREEN}${SYM_CHECK}${C_RESET} %-28s %s\n" "$name" "${C_DIM}${extra}${C_RESET}"
    else
        printf "  ${C_RED}${SYM_CROSS}${C_RESET} %-28s %s\n" "$name" "${C_DIM}${extra}${C_RESET}"
    fi
}

get_disk_size() {
    [[ -f "$1" ]] && du -h "$1" 2>/dev/null | cut -f1 || echo "N/A"
}

cmd_status() {
    print_banner
    printf "${C_BOLD}Environment Status:${C_RESET}\n\n"
    
    printf "${C_DIM}── Toolchain ──${C_RESET}\n"
    status_line "Rust Compiler" "has_cmd rustc" "$(rustc --version 2>/dev/null | cut -d' ' -f2 || echo '')"
    status_line "UEFI Target" "check_rust"
    status_line "NASM Assembler" "has_cmd nasm"
    status_line "QEMU" "check_qemu" "$(qemu-system-x86_64 --version 2>/dev/null | head -1 | cut -d' ' -f4 || echo '')"
    status_line "OVMF Firmware" "check_ovmf" "$(get_ovmf_path 2>/dev/null || echo '')"
    
    printf "\n${C_DIM}── Build Artifacts ──${C_RESET}\n"
    status_line "Bootloader (BOOTX64.EFI)" "check_bootloader"
    
    printf "\n${C_DIM}── Virtual Disks ──${C_RESET}\n"
    status_line "ESP Image" "check_esp_img" "$(get_disk_size "${TESTING_DIR}/esp.img")"
    status_line "Test Disk 50GB" "check_disk_50g" "$(get_disk_size "${TESTING_DIR}/test-disk-50g.img")"
    status_line "Test Disk 10GB" "check_disk_10g" "$(get_disk_size "${TESTING_DIR}/test-disk-10g.img")"
    
    printf "\n${C_DIM}── Distributions ──${C_RESET}\n"
    status_line "Tails OS" "check_tails"
    status_line "Arch Linux" "check_arch"
    
    for kernel in "${ESP_DIR}"/kernels/vmlinuz-* 2>/dev/null; do
        [[ -f "$kernel" ]] || continue
        local name=$(basename "$kernel" | sed 's/vmlinuz-//')
        [[ "$name" == "tails" || "$name" == "arch" ]] && continue
        status_line "${name^}" "[[ -f \"$kernel\" ]]"
    done
    
    printf "\n"
}

cmd_setup() {
    print_banner
    log_step "Installing System Dependencies"
    
    local distro=$(detect_distro)
    local -a pkgs=()
    local install_cmd=""
    
    case "${distro}" in
        arch|manjaro|endeavouros)
            pkgs=(nasm qemu-full ovmf rust parted dosfstools)
            install_cmd="sudo pacman -S --needed --noconfirm"
            ;;
        debian|ubuntu|pop|linuxmint|kali)
            pkgs=(nasm qemu-system-x86 ovmf curl rsync parted dosfstools qemu-utils)
            install_cmd="sudo apt-get install -y -qq"
            sudo apt-get update -qq
            ;;
        fedora)
            pkgs=(nasm qemu-system-x86 edk2-ovmf curl rsync parted dosfstools qemu-img)
            install_cmd="sudo dnf install -y -q"
            ;;
        rhel|centos|almalinux|rocky)
            pkgs=(nasm qemu-kvm edk2-ovmf curl rsync parted dosfstools)
            install_cmd="sudo yum install -y -q"
            ;;
        opensuse*|suse)
            pkgs=(nasm qemu-x86 qemu-ovmf-x86_64 curl rsync parted dosfstools)
            install_cmd="sudo zypper install -y"
            ;;
        alpine)
            pkgs=(nasm qemu-system-x86_64 ovmf curl rsync parted dosfstools bash)
            install_cmd="sudo apk add"
            ;;
        *)
            die "Unsupported distribution: ${distro}"
            ;;
    esac
    
    log_info "Detected: ${distro}"
    ${install_cmd} "${pkgs[@]}" || true
    log_success "System packages installed"
    
    log_step "Rust Toolchain"
    if ! has_cmd rustc; then
        log_info "Installing Rust..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
        source "$HOME/.cargo/env"
    fi
    
    if ! rustup target list | grep -q "x86_64-unknown-uefi (installed)"; then
        rustup target add x86_64-unknown-uefi
    fi
    log_success "Rust ready"
    
    log_step "OVMF Configuration"
    local ovmf_path
    ovmf_path=$(get_ovmf_path) || die "OVMF not found"
    
    if [[ -f "${TESTING_DIR}/run.sh" ]] && ! grep -Fq "${ovmf_path}" "${TESTING_DIR}/run.sh"; then
        sed -i "s|/usr/share/OVMF/OVMF_CODE.fd|${ovmf_path}|g; \
                s|/usr/share/edk2/ovmf/OVMF_CODE.fd|${ovmf_path}|g" "${TESTING_DIR}/run.sh"
    fi
    log_success "OVMF: ${ovmf_path}"
    
    log_step "Workspace"
    mkdir -p "${ESP_DIR}"/{EFI/BOOT,kernels,initrds,loader/entries}
    log_success "Directory structure ready"
    
    printf "\n${C_GREEN}${C_BOLD}Setup complete!${C_RESET}\n"
}

cmd_build() {
    print_banner
    log_step "Building MorpheusX"
    
    pushd "${TESTING_DIR}" >/dev/null
    ./build.sh
    popd >/dev/null
    
    check_bootloader && log_success "Build complete: esp/EFI/BOOT/BOOTX64.EFI"
}

cmd_disk() {
    local target="${1:-}"
    
    print_banner
    
    case "${target}" in
        esp)
            cmd_disk_esp
            ;;
        50g|50G|large)
            cmd_disk_50g
            ;;
        10g|10G|small)
            cmd_disk_10g
            ;;
        all)
            cmd_disk_esp
            cmd_disk_50g
            cmd_disk_10g
            ;;
        info)
            cmd_disk_info
            ;;
        *)
            printf "Usage: %s disk <target>\n\n" "$(basename "$0")"
            printf "Targets:\n"
            printf "  ${C_CYAN}esp${C_RESET}     Create/update ESP image from esp/ directory\n"
            printf "  ${C_CYAN}50g${C_RESET}     Create 50GB test disk with GPT + ESP + root partitions\n"
            printf "  ${C_CYAN}10g${C_RESET}     Create 10GB persistence test disk (empty, for installer testing)\n"
            printf "  ${C_CYAN}all${C_RESET}     Create all disk images\n"
            printf "  ${C_CYAN}info${C_RESET}    Show disk image details\n"
            return 1
            ;;
    esac
}

cmd_disk_info() {
    log_step "Disk Image Details"
    
    printf "\n${C_DIM}── Images ──${C_RESET}\n"
    
    for img in "${TESTING_DIR}"/*.img; do
        [[ -f "$img" ]] || continue
        local name=$(basename "$img")
        local size=$(du -h "$img" | cut -f1)
        local vsize=$(qemu-img info "$img" 2>/dev/null | grep "virtual size" | cut -d: -f2 | xargs || echo "N/A")
        printf "  ${C_CYAN}%s${C_RESET}\n" "$name"
        printf "    Actual: %s  Virtual: %s\n" "$size" "$vsize"
        
        if parted -s "$img" print 2>/dev/null | grep -q "Partition Table"; then
            printf "    Partitions:\n"
            parted -s "$img" print 2>/dev/null | grep -E "^\s*[0-9]" | while read -r line; do
                printf "      %s\n" "$line"
            done
        fi
        printf "\n"
    done
}

cmd_disk_esp() {
    log_step "Creating ESP Image"
    
    local esp_img="${TESTING_DIR}/esp.img"
    
    local esp_size=$(du -sb "${ESP_DIR}" 2>/dev/null | awk '{print int(($1 / 1024 / 1024) + 64)}')
    [[ $esp_size -lt 64 ]] && esp_size=64
    
    log_info "ESP directory size: ~${esp_size}MB"
    
    rm -f "$esp_img"
    dd if=/dev/zero of="$esp_img" bs=1M count=$esp_size status=none
    mkfs.vfat -F 32 -n "ESP" "$esp_img" >/dev/null
    
    local mount_point=$(mktemp -d)
    sudo mount -o loop "$esp_img" "$mount_point"
    
    trap "sudo umount '$mount_point' 2>/dev/null; rmdir '$mount_point'" EXIT
    
    sudo rsync -a --exclude='rootfs' "${ESP_DIR}/" "$mount_point/" 2>/dev/null || true
    
    sudo umount "$mount_point"
    rmdir "$mount_point"
    trap - EXIT
    
    log_success "ESP image created: $(du -h "$esp_img" | cut -f1)"
}

cmd_disk_50g() {
    log_step "Creating 50GB Test Disk"
    
    local disk_img="${TESTING_DIR}/test-disk-50g.img"
    
    if [[ -f "$disk_img" && "${FORCE_MODE}" == "false" ]]; then
        confirm "Disk exists. Recreate?" || { log_info "Skipped"; return 0; }
    fi
    
    rm -f "$disk_img"
    
    log_info "Creating sparse image..."
    qemu-img create -f raw "$disk_img" 50G >/dev/null
    
    log_info "Creating GPT partition table..."
    parted -s "$disk_img" mklabel gpt
    parted -s "$disk_img" mkpart primary fat32 1MiB 513MiB
    parted -s "$disk_img" set 1 esp on
    parted -s "$disk_img" mkpart primary ext4 513MiB 100%
    
    log_info "Setting up loop device..."
    local loop_dev=$(sudo losetup -fP --show "$disk_img")
    
    trap "sudo umount /tmp/morpheus-esp 2>/dev/null || true; sudo losetup -d '$loop_dev' 2>/dev/null || true" EXIT
    
    log_info "Formatting partitions..."
    sudo mkfs.vfat -F 32 -n "ESP" "${loop_dev}p1" >/dev/null
    sudo mkfs.ext4 -q -L "MORPHEUS_ROOT" "${loop_dev}p2" >/dev/null
    
    mkdir -p /tmp/morpheus-esp
    sudo mount "${loop_dev}p1" /tmp/morpheus-esp
    
    sudo mkdir -p /tmp/morpheus-esp/{EFI/BOOT,kernels,initrds,loader/entries}
    
    [[ -f "${ESP_DIR}/EFI/BOOT/BOOTX64.EFI" ]] && sudo cp "${ESP_DIR}/EFI/BOOT/BOOTX64.EFI" /tmp/morpheus-esp/EFI/BOOT/
    [[ -d "${ESP_DIR}/kernels" ]] && sudo cp -r "${ESP_DIR}/kernels/"* /tmp/morpheus-esp/kernels/ 2>/dev/null || true
    [[ -d "${ESP_DIR}/initrds" ]] && sudo cp -r "${ESP_DIR}/initrds/"* /tmp/morpheus-esp/initrds/ 2>/dev/null || true
    
    for kernel in /tmp/morpheus-esp/kernels/vmlinuz-*; do
        [[ -f "$kernel" ]] || continue
        local kname=$(basename "$kernel")
        local distro=$(echo "$kname" | sed 's/vmlinuz-//')
        local title="${distro^}"
        local cmdline="console=ttyS0,115200 console=tty0"
        
        case "$distro" in
            tails)  cmdline="boot=live live-media-path=/live nopersistence noprompt timezone=Etc/UTC splash=0 $cmdline"; title="Tails OS" ;;
            arch)   cmdline="root=/dev/ram0 rw debug $cmdline"; title="Arch Linux" ;;
            ubuntu) cmdline="boot=casper quiet splash $cmdline"; title="Ubuntu" ;;
            debian) cmdline="boot=live quiet $cmdline"; title="Debian" ;;
            fedora) cmdline="rd.live.image quiet $cmdline"; title="Fedora" ;;
            kali)   cmdline="boot=live quiet $cmdline"; title="Kali Linux" ;;
        esac
        
        local initrd=""
        [[ -f "/tmp/morpheus-esp/initrds/initrd-${distro}.img" ]] && initrd="initrd  \\\\initrds\\\\initrd-${distro}.img"
        
        sudo tee "/tmp/morpheus-esp/loader/entries/${distro}.conf" > /dev/null <<EOF
title   $title
linux   \\kernels\\$kname
$initrd
options $cmdline
EOF
        log_info "Boot entry: $title"
    done
    
    sudo sync
    sudo umount /tmp/morpheus-esp
    sudo losetup -d "$loop_dev"
    trap - EXIT
    
    log_success "50GB disk created: $(du -h "$disk_img" | cut -f1) (sparse)"
}

cmd_disk_10g() {
    log_step "Creating 10GB Persistence Disk"
    
    local disk_img="${TESTING_DIR}/test-disk-10g.img"
    
    if [[ -f "$disk_img" && "${FORCE_MODE}" == "false" ]]; then
        confirm "Disk exists. Recreate?" || { log_info "Skipped"; return 0; }
    fi
    
    rm -f "$disk_img"
    qemu-img create -f raw "$disk_img" 10G >/dev/null
    
    log_success "10GB disk created (empty, for installer testing)"
}

cmd_run() {
    local mode="${1:-}"
    
    print_banner
    check_bootloader || die "Bootloader not built. Run: ./setup-dev.sh build"
    
    local ovmf_path
    ovmf_path=$(get_ovmf_path) || die "OVMF not found"
    
    case "${mode}" in
        esp|1)
            check_esp_img || cmd_disk_esp
            log_info "Booting from ESP image..."
            qemu-system-x86_64 \
                -bios "$ovmf_path" \
                -drive format=raw,file="${TESTING_DIR}/esp.img" \
                -net none -smp 4 -m 4G \
                -vga virtio -display gtk,gl=on \
                -serial mon:stdio
            ;;
        50g|large|2)
            check_disk_50g || die "50GB disk not found. Run: ./setup-dev.sh disk 50g"
            log_info "Booting from 50GB test disk..."
            qemu-system-x86_64 \
                -s -bios "$ovmf_path" \
                -drive format=raw,file="${TESTING_DIR}/test-disk-50g.img" \
                -net none -smp 8 -m 12G \
                -vga virtio -display gtk,gl=on \
                -serial mon:stdio
            ;;
        10g|persist|3)
            check_disk_10g || die "10GB disk not found. Run: ./setup-dev.sh disk 10g"
            log_info "Booting from 10GB persistence disk..."
            qemu-system-x86_64 \
                -s -bios "$ovmf_path" \
                -drive format=raw,file="${TESTING_DIR}/test-disk-10g.img" \
                -net none -smp 8 -m 12G \
                -vga virtio -display gtk,gl=on \
                -serial mon:stdio
            ;;
        "")
            printf "Select boot mode:\n"
            printf "  ${C_CYAN}[1]${C_RESET} ESP image (quick test)\n"
            printf "  ${C_CYAN}[2]${C_RESET} 50GB disk (full test with partitions)\n"
            printf "  ${C_CYAN}[3]${C_RESET} 10GB disk (persistence/installer test)\n"
            printf "\n"
            read -r -n 1 -p "Choice: " choice
            printf "\n"
            cmd_run "$choice"
            ;;
        *)
            printf "Usage: %s run [mode]\n\n" "$(basename "$0")"
            printf "Modes:\n"
            printf "  ${C_CYAN}esp${C_RESET}      Boot from ESP image\n"
            printf "  ${C_CYAN}50g${C_RESET}      Boot from 50GB test disk\n"
            printf "  ${C_CYAN}10g${C_RESET}      Boot from 10GB persistence disk\n"
            return 1
            ;;
    esac
}

cmd_install() {
    local target="${1:-}"
    
    print_banner
    
    case "${target}" in
        tails)
            log_step "Installing Tails OS"
            pushd "${TESTING_DIR}" >/dev/null
            [[ "${AUTO_MODE}" == "true" ]] && yes y | ./install-tails.sh || ./install-tails.sh
            popd >/dev/null
            ;;
        arch)
            log_step "Installing Arch Linux"
            pushd "${TESTING_DIR}" >/dev/null
            [[ "${FORCE_MODE}" == "true" ]] && ./install-arch.sh --force || ./install-arch.sh
            popd >/dev/null
            ;;
        distro)
            log_step "Live Distribution Installer"
            pushd "${TESTING_DIR}" >/dev/null
            ./install-live-distro.sh
            popd >/dev/null
            ;;
        *)
            printf "Usage: %s install <target>\n\n" "$(basename "$0")"
            printf "Targets:\n"
            printf "  ${C_CYAN}tails${C_RESET}   Install Tails OS (1.3GB)\n"
            printf "  ${C_CYAN}arch${C_RESET}    Install Arch Linux (~2GB)\n"
            printf "  ${C_CYAN}distro${C_RESET}  Interactive distro selector (Ubuntu, Debian, Fedora, Kali)\n"
            return 1
            ;;
    esac
}

cmd_clean() {
    print_banner
    log_step "Cleaning"
    
    printf "What to clean?\n"
    printf "  ${C_CYAN}[1]${C_RESET} Build artifacts only (target/, BOOTX64.EFI)\n"
    printf "  ${C_CYAN}[2]${C_RESET} Disk images (esp.img, test-disk-*.img)\n"
    printf "  ${C_CYAN}[3]${C_RESET} Distributions (kernels, initrds)\n"
    printf "  ${C_CYAN}[4]${C_RESET} Everything\n"
    printf "  ${C_CYAN}[0]${C_RESET} Cancel\n"
    printf "\n"
    
    read -r -n 1 -p "Choice: " choice
    printf "\n\n"
    
    case "$choice" in
        1)
            rm -rf "${PROJECT_ROOT}/target"
            rm -f "${ESP_DIR}/EFI/BOOT/BOOTX64.EFI"
            log_success "Build artifacts removed"
            ;;
        2)
            rm -f "${TESTING_DIR}"/*.img
            log_success "Disk images removed"
            ;;
        3)
            rm -rf "${ESP_DIR}/kernels/"*
            rm -rf "${ESP_DIR}/initrds/"*
            log_success "Distributions removed"
            ;;
        4)
            confirm "Remove EVERYTHING?" || return 0
            rm -rf "${PROJECT_ROOT}/target"
            rm -f "${ESP_DIR}/EFI/BOOT/BOOTX64.EFI"
            rm -f "${TESTING_DIR}"/*.img
            rm -rf "${ESP_DIR}/kernels/"*
            rm -rf "${ESP_DIR}/initrds/"*
            log_success "All artifacts removed"
            ;;
        0|*) log_info "Cancelled" ;;
    esac
}

cmd_interactive() {
    while true; do
        clear
        print_banner
        
        printf "${C_BOLD}Main Menu${C_RESET}\n\n"
        
        local st
        check_rust && check_qemu && st="${C_GREEN}${SYM_CHECK}${C_RESET}" || st="${C_RED}${SYM_CROSS}${C_RESET}"
        printf "  ${C_CYAN}[1]${C_RESET} Setup Environment        %b\n" "$st"
        
        check_bootloader && st="${C_GREEN}${SYM_CHECK}${C_RESET}" || st="${C_RED}${SYM_CROSS}${C_RESET}"
        printf "  ${C_CYAN}[2]${C_RESET} Build Bootloader         %b\n" "$st"
        
        printf "\n${C_DIM}  ── Distributions ──${C_RESET}\n"
        check_tails && st="${C_GREEN}${SYM_CHECK}${C_RESET}" || st="${C_RED}${SYM_CROSS}${C_RESET}"
        printf "  ${C_CYAN}[3]${C_RESET} Install Tails            %b\n" "$st"
        check_arch && st="${C_GREEN}${SYM_CHECK}${C_RESET}" || st="${C_RED}${SYM_CROSS}${C_RESET}"
        printf "  ${C_CYAN}[4]${C_RESET} Install Arch             %b\n" "$st"
        printf "  ${C_CYAN}[5]${C_RESET} Install Other Distro\n"
        
        printf "\n${C_DIM}  ── Disk Images ──${C_RESET}\n"
        check_esp_img && st="${C_GREEN}${SYM_CHECK}${C_RESET}" || st="${C_RED}${SYM_CROSS}${C_RESET}"
        printf "  ${C_CYAN}[6]${C_RESET} Create ESP Image         %b\n" "$st"
        check_disk_50g && st="${C_GREEN}${SYM_CHECK}${C_RESET}" || st="${C_RED}${SYM_CROSS}${C_RESET}"
        printf "  ${C_CYAN}[7]${C_RESET} Create 50GB Test Disk    %b\n" "$st"
        check_disk_10g && st="${C_GREEN}${SYM_CHECK}${C_RESET}" || st="${C_RED}${SYM_CROSS}${C_RESET}"
        printf "  ${C_CYAN}[8]${C_RESET} Create 10GB Test Disk    %b\n" "$st"
        
        printf "\n${C_DIM}  ── Actions ──${C_RESET}\n"
        printf "  ${C_CYAN}[r]${C_RESET} Run QEMU\n"
        printf "  ${C_CYAN}[s]${C_RESET} Show Status\n"
        printf "  ${C_CYAN}[c]${C_RESET} Clean\n"
        printf "  ${C_CYAN}[q]${C_RESET} Quit\n"
        printf "\n"
        
        read -r -n 1 -p "Select: " choice
        printf "\n\n"
        
        case "$choice" in
            1) cmd_setup; read -r -p "Press Enter..." ;;
            2) cmd_build; read -r -p "Press Enter..." ;;
            3) cmd_install tails; read -r -p "Press Enter..." ;;
            4) cmd_install arch; read -r -p "Press Enter..." ;;
            5) cmd_install distro; read -r -p "Press Enter..." ;;
            6) cmd_disk esp; read -r -p "Press Enter..." ;;
            7) cmd_disk 50g; read -r -p "Press Enter..." ;;
            8) cmd_disk 10g; read -r -p "Press Enter..." ;;
            r|R) cmd_run ;;
            s|S) cmd_status; read -r -p "Press Enter..." ;;
            c|C) cmd_clean; read -r -p "Press Enter..." ;;
            q|Q|0) printf "Bye!\n"; exit 0 ;;
            *) ;;
        esac
    done
}

usage() {
    print_banner
    cat << EOF
${C_BOLD}Usage:${C_RESET} $(basename "$0") [options] <command> [args]

${C_BOLD}Commands:${C_RESET}
  ${C_CYAN}setup${C_RESET}              Install dependencies and configure environment
  ${C_CYAN}build${C_RESET}              Build the bootloader
  ${C_CYAN}run${C_RESET} [mode]         Launch QEMU (esp|50g|10g)
  ${C_CYAN}disk${C_RESET} <target>      Manage disk images (esp|50g|10g|all|info)
  ${C_CYAN}install${C_RESET} <target>   Install distributions (tails|arch|distro)
  ${C_CYAN}status${C_RESET}             Show environment status
  ${C_CYAN}clean${C_RESET}              Remove build artifacts
  ${C_CYAN}interactive${C_RESET}        Launch interactive menu (default)

${C_BOLD}Options:${C_RESET}
  -f, --force          Force re-creation/rebuild
  -y, --yes, --auto    Non-interactive mode
  -h, --help           Show this help

${C_BOLD}Examples:${C_RESET}
  $(basename "$0")                    # Interactive menu
  $(basename "$0") setup              # Setup environment
  $(basename "$0") disk all           # Create all disk images
  $(basename "$0") install tails      # Install Tails OS
  $(basename "$0") run 50g            # Boot from 50GB disk
  $(basename "$0") -f disk 50g        # Force recreate 50GB disk

EOF
}

main() {
    local cmd=""
    local -a args=()
    
    while [[ $# -gt 0 ]]; do
        case "$1" in
            -f|--force)  FORCE_MODE=true; shift ;;
            -y|--yes|--auto) AUTO_MODE=true; shift ;;
            -v|--verbose) VERBOSE=true; shift ;;
            -h|--help)   usage; exit 0 ;;
            -*)          die "Unknown option: $1" ;;
            *)           [[ -z "$cmd" ]] && cmd="$1" || args+=("$1"); shift ;;
        esac
    done
    
    [[ -z "$cmd" ]] && cmd="interactive"
    
    case "$cmd" in
        setup|init)       cmd_setup ;;
        build|compile)    cmd_build ;;
        run|start|qemu)   cmd_run "${args[@]:-}" ;;
        disk|image)       cmd_disk "${args[@]:-}" ;;
        install|add)      cmd_install "${args[@]:-}" ;;
        status|info)      cmd_status ;;
        clean|purge)      cmd_clean ;;
        interactive|menu|i) cmd_interactive ;;
        *)                die "Unknown command: $cmd. Try --help" ;;
    esac
}

main "$@"
