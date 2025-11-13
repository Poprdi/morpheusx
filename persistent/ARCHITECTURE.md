# Persistent Module Architecture

## Overview

This module handles bootloader self-persistence - capturing the running bootloader from memory and creating bootable disk images.

## Critical Design Decision Point

**This is the FIRST location where platform-specific code diverges permanently.**

The PE/COFF relocation format is identical across platforms, but the *semantics* differ:

### x86_64 (Simple)
- Machine type: 0x8664
- Format: PE32+ (64-bit)
- Relocations: IMAGE_REL_BASED_DIR64 (type 10)
- Semantics: Simple pointer fixups
  - Read 64-bit value at relocation address
  - Add/subtract delta
  - Write back
- No instruction encoding tricks
- Straightforward implementation

### ARM64 / aarch64 (Complex)
- Machine type: 0xAA64  
- Format: PE32+ (64-bit)
- Relocations: IMAGE_REL_BASED_DIR64 (type 10)
- Semantics: **May involve instruction encoding**
  - Data relocations: Same as x86_64
  - Code relocations: **ADRP/ADD instruction pairs**
    - ADRP: Load page address (19-bit immediate in instruction)
    - ADD: Add offset within page (12-bit immediate)
    - Cannot treat as simple pointer - must decode instruction
- Requires instruction-level understanding
- More complex implementation

### ARM32 / armv7 (Future)
- Machine type: 0x01C4
- Format: PE32 (32-bit)
- Relocations: IMAGE_REL_BASED_HIGHLOW (type 3)
- Semantics: **Thumb mode considerations**
  - Mixed 16/32-bit instruction encoding
  - BL/BLX instruction patching
  - Literal pool relocations
- Most complex of all three

## Module Structure

```
persistent/
├── src/
│   ├── lib.rs              # Platform-agnostic public API
│   ├── pe/                 # PE/COFF parsing (platform-neutral)
│   │   ├── mod.rs          # Common PE types and errors
│   │   ├── header.rs       # DOS/PE/COFF headers
│   │   ├── section.rs      # Section table parsing
│   │   └── reloc.rs        # Relocation table + trait
│   ├── arch/               # Platform-specific engines
│   │   ├── x86_64.rs       # Simple pointer fixups
│   │   └── aarch64.rs      # Instruction-aware fixups
│   ├── capture/            # Memory image extraction
│   │   └── mod.rs          # Capture + unrelocate logic
│   └── storage/            # Persistence backends
│       ├── mod.rs          # Backend trait
│       ├── esp.rs          # Layer 0: ESP/FAT32
│       ├── tpm.rs          # Layer 1: TPM attestation
│       ├── cmos.rs         # Layer 2: NVRAM micro-persistence
│       └── hvram.rs        # Layer 3: Hypervisor RAM
```

## Trait-Based Abstraction

### `RelocationEngine` Trait

Defined in `pe/reloc.rs`, implemented per-architecture:

```rust
pub trait RelocationEngine {
    fn apply_relocation(...)   -> PeResult<()>;  // UEFI loader does this
    fn unapply_relocation(...) -> PeResult<()>;  // We do this for persistence
    fn arch() -> PeArch;
}
```

Implementations:
- `arch::x86_64::X64RelocationEngine`
- `arch::aarch64::Aarch64RelocationEngine`
- (future) `arch::armv7::ArmRelocationEngine`

### `PersistenceBackend` Trait

Defined in `storage/mod.rs`, one per storage layer:

```rust
pub trait PersistenceBackend {
    fn store_bootloader(&mut self, data: &[u8]) -> Result<(), PeError>;
    fn retrieve_bootloader(&mut self) -> Result<Vec<u8>, PeError>;
    fn is_persisted(&mut self) -> Result<bool, PeError>;
    fn name(&self) -> &str;
}
```

Implementations:
- `storage::esp::EspBackend` (Layer 0)
- (future) `storage::tpm::TpmBackend` (Layer 1)
- (future) `storage::cmos::CmosBackend` (Layer 2)
- (future) `storage::hvram::HvramBackend` (Layer 3)

## Persistence Layers

### Layer 0: ESP/FAT32 (Primary Bootable Storage)
- **Purpose**: Store bootable UEFI binary
- **Path**: `/EFI/BOOT/BOOTx64.EFI` or `/EFI/BOOT/BOOTAA64.EFI`
- **Size**: Full bootloader (~500KB - 2MB)
- **Retrieval**: UEFI firmware loads automatically
- **Status**: Partially implemented (FAT32 write exists)

### Layer 1: TPM (Attestation & Integrity)
- **Purpose**: Cryptographic measurement of boot state
- **Storage**: PCR registers (20 bytes each)
- **Data**: SHA-1 or SHA-256 hash of bootloader
- **Retrieval**: Verify hash matches expected value
- **Status**: TODO

### Layer 2: CMOS/NVRAM (Emergency Recovery)
- **Purpose**: Tiny stub for recovery if ESP corrupted
- **Storage**: UEFI variables or CMOS RAM (128-512 bytes)
- **Data**: Minimal chainloader or config
- **Retrieval**: Fallback boot mechanism
- **Status**: TODO

### Layer 3: HVRAM (Stealth/Anti-Forensics)
- **Purpose**: Hide in hypervisor-accessible RAM
- **Storage**: Reserved memory regions
- **Data**: Full bootloader or critical config
- **Retrieval**: Hypercall interface
- **Status**: TODO (research phase)

## PE Relocation Process

### UEFI Loader (what happens when booting)

1. Read PE file from disk
2. Parse headers, find ImageBase (e.g., 0x400000)
3. Allocate memory at *different* address (e.g., 0x76E4C000)
4. Copy PE sections to memory
5. Calculate delta: 0x76E4C000 - 0x400000 = 0x76A4C000
6. Parse .reloc section
7. For each DIR64 relocation:
   - Read 64-bit value at relocation RVA
   - Add delta to value
   - Write back
8. Update ImageBase field in PE header to 0x76E4C000
9. Jump to entry point

### Our Unrelocator (creating bootable image)

1. Capture running image from memory (already relocated)
2. Read ImageBase from PE header (current: 0x76E4C000)
3. Find original ImageBase from PE signature (0x400000)
4. Calculate delta: 0x76E4C000 - 0x400000 = 0x76A4C000
5. Parse .reloc section
6. For each DIR64 relocation:
   - Read 64-bit value at relocation RVA
   - **Subtract** delta from value
   - Write back
7. Restore ImageBase field to 0x400000
8. Write unrelocated image to disk
9. Image is now bootable!

## Implementation Phases

### Phase 1: Core PE Parser (Platform-Neutral)
- [ ] Parse DOS header
- [ ] Parse PE signature and COFF header
- [ ] Parse Optional Header (PE32+)
- [ ] Parse section table
- [ ] Find .reloc section
- [ ] Iterate relocation blocks

### Phase 2: x86_64 Relocation Engine
- [ ] Implement apply_relocation for DIR64
- [ ] Implement unapply_relocation for DIR64
- [ ] Handle edge cases (bounds checking)
- [ ] Add unit tests with sample PE data

### Phase 3: Memory Capture
- [ ] Capture image from UEFI LoadedImageProtocol
- [ ] Calculate relocation delta
- [ ] Create MemoryImage struct
- [ ] Implement create_bootable_image()

### Phase 4: ESP Integration
- [ ] Create EspBackend
- [ ] Wire into existing installer
- [ ] Replace current image copy with unrelocated version
- [ ] Test on QEMU

### Phase 5: ARM64 Support
- [ ] Implement aarch64 RelocationEngine
- [ ] Start with simple pointer fixups
- [ ] Add ADRP/ADD detection (if needed)
- [ ] Test on ARM64 QEMU or real hardware

### Phase 6: Additional Persistence Layers
- [ ] Design TPM interface
- [ ] Implement CMOS backend
- [ ] Research HVRAM techniques
- [ ] Multi-layer orchestration

## Testing Strategy

### Unit Tests
- Parse known-good PE files
- Verify relocation calculations
- Test edge cases (invalid PE, corrupt .reloc)

### Integration Tests
- Build bootloader
- Run in QEMU
- Install to ESP
- Reboot
- Verify bootloader loads from persisted image

### Cross-Platform Tests
- Build for x86_64
- Build for aarch64
- Verify both architectures can self-persist
- Compare unrelocated images with original binaries

## Security Considerations (Future)

- **Signing**: Sign persisted images (Phase 7)
- **Encryption**: Encrypt bootloader on disk (Phase 8)
- **Obfuscation**: Code polymorphism (Phase 9)
- **Attestation**: TPM-backed verification (Phase 6)

## References

- [PE Format Specification](https://learn.microsoft.com/en-us/windows/win32/debug/pe-format)
- [UEFI Specification 2.10](https://uefi.org/specs/UEFI/2.10/)
- [ARM64 Instruction Encoding](https://developer.arm.com/documentation/ddi0602/latest)
- [OSDev PE/COFF](https://wiki.osdev.org/PE)

---

**Status**: Architecture defined, stubs in place
**Next**: Implement Phase 1 (core PE parser)
