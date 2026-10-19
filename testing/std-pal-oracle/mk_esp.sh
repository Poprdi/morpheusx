#!/usr/bin/env bash
# mk_esp.sh <name> <init-elf> [extra-elf:/dest ...]
# Builds $S/img/<name>.esp.img (S = this script's dir): plain FAT32 with /EFI/BOOT/BOOTX64.EFI (kernel) and
# /morpheus/helix.img (HelixFS v3 RAM root, /bin/init = <init-elf>). No losetup.
set -euo pipefail
S=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$S/../.." && pwd)
HOST_TOOL=${HOST_TOOL:-$REPO/helix/host_tool/target/release/host_tool}
EFI=${EFI:-$REPO/target/x86_64-unknown-uefi/release/morpheus-bootloader.efi}
HELIX_MB=${HELIX_MB:-32}
ESP_MB=${ESP_MB:-$((HELIX_MB + 16))}
name=$1; init=$2; shift 2
mkdir -p "$S/img"
hx=$S/img/$name.helix.img
esp=$S/img/$name.esp.img
rm -f "$hx" "$esp"
dd if=/dev/zero of="$hx" bs=1M count="$HELIX_MB" status=none
"$HOST_TOOL" inject "$hx" "$init" --dest /bin/init >/dev/null
for spec in "$@"; do
  "$HOST_TOOL" inject "$hx" "${spec%%:*}" --dest "${spec#*:}" >/dev/null
done
dd if=/dev/zero of="$esp" bs=1M count="$ESP_MB" status=none
mkfs.vfat -F 32 "$esp" >/dev/null
mmd -i "$esp" ::/EFI ::/EFI/BOOT ::/morpheus
mcopy -i "$esp" "$EFI" ::/EFI/BOOT/BOOTX64.EFI
mcopy -i "$esp" "$hx" ::/morpheus/helix.img
echo "$esp"
