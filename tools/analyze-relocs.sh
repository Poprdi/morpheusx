#!/bin/bash
# Analyze .reloc section to show how many pointers will be reversed

set -e

if [ $# -ne 1 ]; then
    echo "Usage: $0 <bootloader.efi>"
    exit 1
fi

EFI_FILE="$1"

# Use objdump to show relocation count
echo "=== PE File Analysis ==="
echo ""

# Get ImageBase
IMAGE_BASE=$(./extract-image-base.sh "$EFI_FILE")
echo "ImageBase: $IMAGE_BASE"
echo ""

# Parse .reloc section manually
E_LFANEW=$(od -An -t x4 -j 0x3C -N 4 "$EFI_FILE" | tr -d ' ')
E_LFANEW_DEC=$((0x$E_LFANEW))

# Get section count
SECTION_COUNT_OFFSET=$((E_LFANEW_DEC + 6))
SECTION_COUNT=$(od -An -t x2 -j "$SECTION_COUNT_OFFSET" -N 2 "$EFI_FILE" | tr -d ' ')
SECTION_COUNT_DEC=$((0x$SECTION_COUNT))

# Get optional header size
OPT_HDR_SIZE_OFFSET=$((E_LFANEW_DEC + 20))
OPT_HDR_SIZE=$(od -An -t x2 -j "$OPT_HDR_SIZE_OFFSET" -N 2 "$EFI_FILE" | tr -d ' ')
OPT_HDR_SIZE_DEC=$((0x$OPT_HDR_SIZE))

# Section table starts after optional header
SECTION_TABLE_OFFSET=$((E_LFANEW_DEC + 24 + OPT_HDR_SIZE_DEC))

# Find .reloc section
for ((i=0; i<SECTION_COUNT_DEC; i++)); do
    SEC_OFFSET=$((SECTION_TABLE_OFFSET + i * 40))
    SEC_NAME=$(dd if="$EFI_FILE" bs=1 skip=$SEC_OFFSET count=8 2>/dev/null | tr -d '\0')
    
    if [ "$SEC_NAME" = ".reloc" ]; then
        RELOC_VIRT_SIZE_OFFSET=$((SEC_OFFSET + 8))
        RELOC_VIRT_SIZE=$(od -An -t x4 -j "$RELOC_VIRT_SIZE_OFFSET" -N 4 "$EFI_FILE" | tr -d ' ')
        RELOC_VIRT_SIZE_DEC=$((0x$RELOC_VIRT_SIZE))
        
        RELOC_RAW_SIZE_OFFSET=$((SEC_OFFSET + 16))
        RELOC_RAW_SIZE=$(od -An -t x4 -j "$RELOC_RAW_SIZE_OFFSET" -N 4 "$EFI_FILE" | tr -d ' ')
        RELOC_RAW_SIZE_DEC=$((0x$RELOC_RAW_SIZE))
        
        echo ".reloc section found:"
        echo "  Virtual size: $RELOC_VIRT_SIZE_DEC bytes (0x$RELOC_VIRT_SIZE)"
        echo "  Raw size:     $RELOC_RAW_SIZE_DEC bytes (0x$RELOC_RAW_SIZE)"
        
        # Estimate relocation count (rough - assumes 8 byte header per block)
        # Each block has 8 byte header, rest are 2-byte entries
        # Rough estimate: (size - 8) / 2 per block, assume ~1-4 blocks
        EST_ENTRIES=$(( (RELOC_RAW_SIZE_DEC - 32) / 2 ))
        
        echo ""
        echo "Estimated DIR64 relocations: ~$EST_ENTRIES pointers"
        echo ""
        echo "When installed from memory, the unrelocate loop will:"
        echo "  1. Parse this .reloc section (from .morpheus copy)"
        echo "  2. Iterate through ALL blocks"
        echo "  3. Reverse each of the ~$EST_ENTRIES pointers"
        echo "  4. Restore ImageBase in header"
        echo ""
        echo "This ensures ALL pointers in code/data are restored to original values."
        
        exit 0
    fi
done

echo "ERROR: No .reloc section found"
exit 1
