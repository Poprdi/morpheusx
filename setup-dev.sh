#!/usr/bin/env bash

set -euo pipefail
IFS=$'\n\t'

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly PROJECT_ROOT="${SCRIPT_DIR}"
readonly TESTING_DIR="${PROJECT_ROOT}/testing"
readonly ESP_DIR="${TESTING_DIR}/esp"
readonly VERSION="1.0.0"

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
check_ovmf()      { find /usr/share -name 'OVMF_CODE.fd' -o -name 'OVMF.fd' -o -name 'ovmf-x86_64*.bin' 2>/dev/null | head -1 | grep -q .; }
check_tails()     { [[ -f "${ESP_DIR}/kernels/vmlinuz-tails" ]]; }
check_arch()      { [[ -f "${ESP_DIR}/kernels/vmlinuz-arch" ]]; }
check_disks()     { [[ -f "${TESTING_DIR}/test-disk-50g.img" ]]; }
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
    for p in "${paths[@]}"; do [[ -f "$p" ]] && { echo "$p"; return; }; done
    return 1
}

status_line() {
    local check=$1 name=$2
    if $check; then
        printf "  ${C_GREEN}${SYM_CHECK}${C_RESET} %s\n" "$name"
    else
        printf "  ${C_RED}${SYM_CROSS}${C_RESET} %s\n" "$name"
    fi
}

cmd_status() {
    print_banner
    printf "${C_BOLD}Environment Status:${C_RESET}\n\n"
    
    printf "${C_DIM}Toolchain${C_RESET}\n"
    status_line "has_cmd rustc" "Rust Compiler"
    status_line "check_rust" "UEFI Target (x86_64-unknown-uefi)"
    status_line "has_cmd nasm" "NASM Assembler"
    status_line "check_qemu" "QEMU"
    status_line "check_ovmf" "OVMF Firmware"
    
    printf "\n${C_DIM}Artifacts${C_RESET}\n"
    status_line "check_bootloader" "Bootloader (BOOTX64.EFI)"
    status_line "check_disks" "Test Disk (50GB)"
    
    printf "\n${C_DIM}Distributions${C_RESET}\n"
    status_line "check_tails" "Tails OS"
    status_line "check_arch" "Arch Linux"
    
    local ovmf_path
    if ovmf_path=$(get_ovmf_path); then
        printf "\n${C_DIM}OVMF Path:${C_RESET} %s\n" "$ovmf_path"
    fi
}

cmd_setup() {
    print_banner
    log_step "Installing System Dependencies"
    
    local distro=$(detect_distro)
    local -a pkgs=()
    local install_cmd=""
    
    case "${distro}" in
        arch|manjaro|endeavouros)
            pkgs=(nasm qemu-full ovmf rust)
            install_cmd="sudo pacman -S --needed --noconfirm"
            ;;
        debian|ubuntu|pop|linuxmint|kali)
            pkgs=(nasm qemu-system-x86 ovmf curl rsync parted dosfstools)
            install_cmd="sudo apt-get install -y -qq"
            sudo apt-get update -qq
            ;;
        fedora)
            pkgs=(nasm qemu-system-x86 edk2-ovmf curl rsync parted dosfstools)
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
    ${install_cmd} "${pkgs[@]}"
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
    mkdir -p "${ESP_DIR}"/{EFI/BOOT,kernels,initrds}
    log_success "Directory structure ready"
    
    printf "\n${C_GREEN}${C_BOLD}Setup complete!${C_RESET}\n"
    printf "Run ${C_CYAN}./setup-dev.sh build${C_RESET} to compile the bootloader\n"
}

cmd_build() {
    print_banner
    log_step "Building MorpheusX"
    
    pushd "${TESTING_DIR}" >/dev/null
    ./build.sh
    popd >/dev/null
    
    check_bootloader && log_success "Build complete: esp/EFI/BOOT/BOOTX64.EFI"
}

cmd_run() {
    print_banner
    check_bootloader || die "Bootloader not built. Run: ./setup-dev.sh build"
    
    log_info "Launching QEMU..."
    pushd "${TESTING_DIR}" >/dev/null
    ./run.sh
    popd >/dev/null
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
        disk|disks)
            log_step "Creating Test Disks"
            pushd "${TESTING_DIR}" >/dev/null
            [[ "${AUTO_MODE}" == "true" ]] && yes y | ./create-test-disk.sh || ./create-test-disk.sh
            popd >/dev/null
            ;;
        *)
            printf "Usage: %s install <target>\n\n" "$(basename "$0")"
            printf "Targets:\n"
            printf "  ${C_CYAN}tails${C_RESET}   Install Tails OS (1.3GB)\n"
            printf "  ${C_CYAN}arch${C_RESET}    Install Arch Linux (~2GB)\n"
            printf "  ${C_CYAN}distro${C_RESET}  Interactive distro selector\n"
            printf "  ${C_CYAN}disk${C_RESET}    Create test disk images\n"
            return 1
            ;;
    esac
}

cmd_clean() {
    print_banner
    log_step "Cleaning build artifacts"
    
    confirm "Remove build artifacts?" || return 0
    
    rm -rf "${PROJECT_ROOT}/target"
    rm -f "${ESP_DIR}/EFI/BOOT/BOOTX64.EFI"
    rm -f "${TESTING_DIR}/esp.img"
    
    if confirm "Also remove test disks? (50GB+10GB)"; then
        rm -f "${TESTING_DIR}"/test-disk-*.img
    fi
    
    log_success "Clean complete"
}

cmd_interactive() {
    while true; do
        clear
        print_banner
        
        printf "${C_BOLD}Main Menu${C_RESET}\n\n"
        
        local st_setup st_build st_tails st_arch st_disk
        check_rust && check_qemu && st_setup="${C_GREEN}${SYM_CHECK}${C_RESET}" || st_setup="${C_RED}${SYM_CROSS}${C_RESET}"
        check_bootloader && st_build="${C_GREEN}${SYM_CHECK}${C_RESET}" || st_build="${C_RED}${SYM_CROSS}${C_RESET}"
        check_tails && st_tails="${C_GREEN}${SYM_CHECK}${C_RESET}" || st_tails="${C_RED}${SYM_CROSS}${C_RESET}"
        check_arch && st_arch="${C_GREEN}${SYM_CHECK}${C_RESET}" || st_arch="${C_RED}${SYM_CROSS}${C_RESET}"
        check_disks && st_disk="${C_GREEN}${SYM_CHECK}${C_RESET}" || st_disk="${C_RED}${SYM_CROSS}${C_RESET}"
        
        printf "  ${C_CYAN}[1]${C_RESET} Setup Environment        %b\n" "$st_setup"
        printf "  ${C_CYAN}[2]${C_RESET} Build Bootloader         %b\n" "$st_build"
        printf "  ${C_CYAN}[3]${C_RESET} Install Tails            %b\n" "$st_tails"
        printf "  ${C_CYAN}[4]${C_RESET} Install Arch             %b\n" "$st_arch"
        printf "  ${C_CYAN}[5]${C_RESET} Install Other Distro\n"
        printf "  ${C_CYAN}[6]${C_RESET} Create Test Disks        %b\n" "$st_disk"
        printf "  ${C_CYAN}[7]${C_RESET} Run QEMU\n"
        printf "  ${C_CYAN}[8]${C_RESET} Show Status\n"
        printf "  ${C_CYAN}[9]${C_RESET} Clean\n"
        printf "  ${C_CYAN}[0]${C_RESET} Exit\n"
        printf "\n"
        
        read -r -n 1 -p "Select: " choice
        printf "\n\n"
        
        case "$choice" in
            1) cmd_setup; read -r -p "Press Enter..." ;;
            2) cmd_build; read -r -p "Press Enter..." ;;
            3) cmd_install tails; read -r -p "Press Enter..." ;;
            4) cmd_install arch; read -r -p "Press Enter..." ;;
            5) cmd_install distro; read -r -p "Press Enter..." ;;
            6) cmd_install disk; read -r -p "Press Enter..." ;;
            7) cmd_run ;;
            8) cmd_status; read -r -p "Press Enter..." ;;
            9) cmd_clean; read -r -p "Press Enter..." ;;
            0|q) printf "Bye!\n"; exit 0 ;;
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
  ${C_CYAN}run${C_RESET}                Launch QEMU with the bootloader
  ${C_CYAN}install${C_RESET} <target>   Install a distribution or create disks
                       Targets: tails, arch, distro, disk
  ${C_CYAN}status${C_RESET}             Show environment status
  ${C_CYAN}clean${C_RESET}              Remove build artifacts
  ${C_CYAN}interactive${C_RESET}        Launch interactive menu (default if no command)

${C_BOLD}Options:${C_RESET}
  -f, --force          Force re-installation/rebuild
  -y, --yes, --auto    Non-interactive mode (assume yes)
  -h, --help           Show this help

${C_BOLD}Examples:${C_RESET}
  $(basename "$0")                    # Interactive mode
  $(basename "$0") setup              # Setup environment
  $(basename "$0") install tails      # Install Tails OS
  $(basename "$0") -y install arch    # Install Arch (non-interactive)
  $(basename "$0") build && $(basename "$0") run

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
        setup|init)      cmd_setup ;;
        build|compile)   cmd_build ;;
        run|start|qemu)  cmd_run ;;
        install|add)     cmd_install "${args[@]:-}" ;;
        status|info)     cmd_status ;;
        clean|purge)     cmd_clean ;;
        interactive|menu|i) cmd_interactive ;;
        *)               die "Unknown command: $cmd. Try --help" ;;
    esac
}

main "$@"
