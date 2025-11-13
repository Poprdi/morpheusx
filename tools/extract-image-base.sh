#!/bin/bash
# Extract ImageBase from compiled EFI binary
# Usage: extract-image-base.sh <path-to-efi>

set -e

if [ $# -ne 1 ]; then
    echo "Usage: $0 <bootloader.efi>"
    exit 1
fi

EFI_FILE="$1"

if [ ! -f "$EFI_FILE" ]; then
    echo "Error: File not found: $EFI_FILE"
    exit 1
fi

# Read e_lfanew offset (at 0x3C)
E_LFANEW=$(od -An -t x4 -j 0x3C -N 4 "$EFI_FILE" | tr -d ' ')
E_LFANEW_DEC=$((0x$E_LFANEW))

# ImageBase is at: PE_OFFSET + 4 (PE sig) + 20 (COFF) + 24 (OptionalHeader offset)
IMAGE_BASE_OFFSET=$((E_LFANEW_DEC + 4 + 20 + 24))

# Read ImageBase (8 bytes, little-endian)
IMAGE_BASE_HEX=$(od -An -t x8 -j "$IMAGE_BASE_OFFSET" -N 8 "$EFI_FILE" | tr -d ' ')

# Convert little-endian to big-endian for display
# Split into bytes and reverse
BYTE1=$(echo "$IMAGE_BASE_HEX" | cut -c15-16)
BYTE2=$(echo "$IMAGE_BASE_HEX" | cut -c13-14)
BYTE3=$(echo "$IMAGE_BASE_HEX" | cut -c11-12)
BYTE4=$(echo "$IMAGE_BASE_HEX" | cut -c9-10)
BYTE5=$(echo "$IMAGE_BASE_HEX" | cut -c7-8)
BYTE6=$(echo "$IMAGE_BASE_HEX" | cut -c5-6)
BYTE7=$(echo "$IMAGE_BASE_HEX" | cut -c3-4)
BYTE8=$(echo "$IMAGE_BASE_HEX" | cut -c1-2)

IMAGE_BASE="0x${BYTE1}${BYTE2}${BYTE3}${BYTE4}${BYTE5}${BYTE6}${BYTE7}${BYTE8}"

echo "$IMAGE_BASE"
