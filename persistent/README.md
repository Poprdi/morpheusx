# Morpheus Persistent Module

Bootloader self-persistence and data persistence layer.

## What This Module Does

Extracts the running UEFI bootloader from memory and creates a bootable disk image by **reversing UEFI relocations**. This enables self-replication without needing the original binary file.

## The Challenge

When UEFI loads a PE binary:

1. Reads file from disk (e.g., `BOOTX64.EFI`)
2. Allocates memory at runtime address (not the linker's `ImageBase`)
3. Applies base relocations from `.reloc` section
4. Updates `ImageBase` field in PE header
5. Jumps to entry point

If you just copy the memory image back to disk, **it won't boot** because:
- All pointers are fixed up for the current load address
- The `.reloc` section isn't re-applied during next boot
- New load address will likely differ

## The Solution

This module **reverses the relocation process**:

1. Capture running image from `LoadedImageProtocol`
2. Parse PE headers and `.reloc` section
3. Calculate relocation delta (current address - original `ImageBase`)
4. **Unapply** all relocations (subtract delta instead of add)
5. Restore original `ImageBase` in header
6. Write unrelocated image to ESP

The result is byte-for-byte equivalent to the original disk file and is bootable.

## Platform Considerations

### x86_64 (Simple)
- PE32+ format
- DIR64 relocations (type 10)
- Simple pointer fixups: `*addr += delta` or `*addr -= delta`
- No special handling needed

### ARM64 / aarch64 (Complex)
- PE32+ format (same as x86_64)
- DIR64 relocations (type 10)
- **May involve instruction encoding:**
  - ADRP/ADD pairs for position-independent code
  - Cannot treat as simple pointer
  - Must decode instruction and patch immediate fields
- Implementation: Start simple (data pointers only), add instruction handling later

### ARM32 / armv7 (Future)
- PE32 format (32-bit)
- HIGHLOW relocations (type 3)
- Thumb mode considerations
- Most complex of all platforms

## Module Structure

```
persistent/
├── pe/              # PE/COFF parsing (platform-neutral)
│   ├── header.rs    # DOS/PE/COFF headers
│   ├── section.rs   # Section table
│   └── reloc.rs     # Relocation table + RelocationEngine trait
├── arch/            # Platform-specific relocation engines
│   ├── x86_64.rs    # Simple pointer fixups
│   └── aarch64.rs   # Instruction-aware fixups
├── capture/         # Memory image extraction
│   └── mod.rs       # Capture + unrelocate
└── storage/         # Persistence backends (multi-layer)
    ├── esp.rs       # Layer 0: ESP/FAT32 (primary)
    ├── tpm.rs       # Layer 1: TPM attestation
    ├── cmos.rs      # Layer 2: NVRAM recovery
    └── hvram.rs     # Layer 3: Hypervisor stealth
```

## Usage Example

```rust
use morpheus_persistent::capture::MemoryImage;
use morpheus_persistent::storage::{PersistenceBackend, esp::EspBackend};

unsafe {
    // Get current image from UEFI
    let loaded_image = get_loaded_image(bs, image_handle)?;
    let image_base = (*loaded_image).image_base as *const u8;
    let image_size = (*loaded_image).image_size as usize;
    
    // Capture and unrelocate
    let captured = MemoryImage::capture_from_memory(image_base, image_size)?;
    let bootable = captured.create_bootable_image()?;
    
    // Persist to ESP
    let mut esp = EspBackend::new(adapter, esp_start_lba);
    esp.store_bootloader(&bootable)?;
}
```

## Multi-Layer Persistence

Different persistence layers serve different purposes:

| Layer | Backend | Purpose | Size | Retrieval |
|-------|---------|---------|------|-----------|
| 0 | ESP | Primary bootable storage | Full binary | UEFI auto-loads |
| 1 | TPM | Attestation & integrity | 20-32 bytes (hash) | Verify on boot |
| 2 | CMOS | Emergency recovery | 128-512 bytes | Fallback chainloader |
| 3 | HVRAM | Stealth/anti-forensics | Full binary | Hypercall |

### Layer 0: ESP (Implemented)
Standard UEFI boot path. Writes `/EFI/BOOT/BOOTX64.EFI` to FAT32.

### Layer 1: TPM (TODO)
Store SHA-256 hash of bootloader in TPM PCR. Verify integrity on boot.

### Layer 2: CMOS (TODO)
Tiny stub in UEFI NVRAM variables. If ESP corrupted, chainload from network or recovery partition.

### Layer 3: HVRAM (TODO - Research)
If running under hypervisor, hide full bootloader in hypervisor-accessible RAM. Retrieve via hypercall.

## Implementation Status

- [x] Architecture defined
- [x] Module structure created
- [x] Platform-specific stubs in place
- [x] Trait abstractions designed
- [ ] Core PE parser (Phase 1)
- [ ] x86_64 relocation engine (Phase 2)
- [ ] Memory capture (Phase 3)
- [ ] ESP integration (Phase 4)
- [ ] ARM64 support (Phase 5)
- [ ] Additional persistence layers (Phase 6+)

## References

- [PE Format Specification](https://learn.microsoft.com/en-us/windows/win32/debug/pe-format)
- [UEFI Specification 2.10](https://uefi.org/specs/UEFI/2.10/)
- [Base Relocations](https://learn.microsoft.com/en-us/windows/win32/debug/pe-format#base-relocations)

## Why This Matters

This is the **first permanent architectural divergence** between platforms. Every decision here affects:

- Code maintainability forever
- Cross-platform compatibility
- Future obfuscation/encryption layers
- TPM attestation strategy
- Self-update mechanisms

We're designing this right the first time with proper trait abstractions to avoid refactoring hell later.

---

**Status**: Architecture complete, ready for implementation  
**Next Step**: Implement Phase 1 (core PE parser)
