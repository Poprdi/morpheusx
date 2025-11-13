# Implementation Roadmap - Persistent Module

Track progress on PE relocation reverse engineering and self-persistence.

## Phase 1: Core PE Parser (Platform-Neutral)

Foundation for all architectures - parsing PE/COFF structures.

### 1.1 DOS Header Parsing
- [ ] Implement `DosHeader::parse()`
- [ ] Validate MZ signature
- [ ] Extract e_lfanew offset
- [ ] Add bounds checking
- [ ] Unit test with sample PE files

### 1.2 PE Signature & COFF Header
- [ ] Parse PE signature at e_lfanew offset
- [ ] Validate "PE\0\0" magic bytes
- [ ] Implement `CoffHeader::parse()`
- [ ] Detect machine type (x64, ARM64, ARM)
- [ ] Extract section count
- [ ] Unit test architecture detection

### 1.3 Optional Header Parsing
- [ ] Implement `OptionalHeader64::parse()` for PE32+
- [ ] Extract ImageBase field
- [ ] Extract size_of_image
- [ ] Extract size_of_headers
- [ ] Parse data directories array
- [ ] Find .reloc directory entry
- [ ] Unit test on real UEFI binaries

### 1.4 Section Table Parsing
- [ ] Implement `SectionTable::parse()`
- [ ] Iterate section headers
- [ ] Find .reloc section by name
- [ ] Extract virtual_address and size_of_raw_data
- [ ] Validate section alignment
- [ ] Unit test section enumeration

### 1.5 Complete Header Parser
- [ ] Implement `PeHeaders::parse()`
- [ ] Chain all header parsing together
- [ ] Return structured header info
- [ ] Add comprehensive error handling
- [ ] Integration test with bootloader binary

**Deliverable**: Can parse PE headers and locate .reloc section

---

## Phase 2: x86_64 Relocation Engine

Simple pointer fixups for x86_64.

### 2.1 Relocation Block Iterator
- [ ] Implement `RelocationBlockIter::new()`
- [ ] Implement `Iterator::next()` for blocks
- [ ] Parse `BaseRelocationBlock` header
- [ ] Extract relocation entries (u16 array)
- [ ] Handle block size correctly
- [ ] Unit test with hand-crafted .reloc data

### 2.2 DIR64 Relocation Application
- [ ] Implement `X64RelocationEngine::apply_relocation()`
- [ ] Calculate absolute RVA from page + offset
- [ ] Read 64-bit value at location
- [ ] Add delta to value
- [ ] Write back modified value
- [ ] Handle bounds checking
- [ ] Unit test with mock image data

### 2.3 DIR64 Relocation Reversal
- [ ] Implement `X64RelocationEngine::unapply_relocation()`
- [ ] Same as apply but subtract delta
- [ ] Ensure symmetry (apply then unapply = identity)
- [ ] Unit test round-trip
- [ ] Edge case testing (zero delta, negative delta)

### 2.4 x86_64 Engine Integration
- [ ] Wire engine into `create_bootable_image()`
- [ ] Process all relocation blocks
- [ ] Handle errors gracefully
- [ ] Log progress for debugging
- [ ] Integration test with real bootloader

**Deliverable**: Can unrelocate x86_64 PE binaries

---

## Phase 3: Memory Capture & Unrelocate

Extract running bootloader and reverse relocations.

### 3.1 Memory Image Capture
- [ ] Implement `MemoryImage::capture_from_memory()`
- [ ] Allocate Vec and copy from image_base
- [ ] Parse PE headers from memory
- [ ] Extract current ImageBase (relocated)
- [ ] Find original ImageBase (from PE signature area)
- [ ] Calculate relocation delta
- [ ] Store in MemoryImage struct

### 3.2 Bootable Image Creation
- [ ] Implement `MemoryImage::create_bootable_image()`
- [ ] Clone memory image data
- [ ] Get platform-specific relocation engine
- [ ] Find .reloc section in headers
- [ ] Iterate all relocation blocks
- [ ] Call engine.unapply_relocation() for each entry
- [ ] Restore original ImageBase in header
- [ ] Return unrelocated image

### 3.3 Verification
- [ ] Add checksum validation (optional)
- [ ] Compare unrelocated image size with original
- [ ] Verify .reloc section integrity
- [ ] Test with known-good binaries

**Deliverable**: Can create bootable image from running memory

---

## Phase 4: ESP Integration

Wire into existing bootloader installer.

### 4.1 ESP Backend Implementation
- [ ] Implement `EspBackend::new()`
- [ ] Store block I/O adapter reference
- [ ] Store partition LBA start
- [ ] Implement `store_bootloader()`
- [ ] Use `morpheus_core::fs::fat32_ops::write_file()`
- [ ] Write to `/EFI/BOOT/BOOTX64.EFI`
- [ ] Implement `retrieve_bootloader()` (for verification)
- [ ] Implement `is_persisted()`

### 4.2 Installer Integration
- [ ] Update `bootloader/src/installer/mod.rs`
- [ ] Replace current image copy with `MemoryImage::capture()`
- [ ] Replace `restore_pe_image_base()` with `create_bootable_image()`
- [ ] Use `EspBackend::store_bootloader()`
- [ ] Remove old relocation code
- [ ] Update error handling

### 4.3 Testing
- [ ] Build bootloader
- [ ] Run in QEMU
- [ ] Install to test disk
- [ ] Reboot QEMU
- [ ] Verify bootloader loads from persisted image
- [ ] Compare persisted image with original binary

**Deliverable**: Self-persisting bootloader on x86_64

---

## Phase 5: ARM64 Support

Extend to aarch64 architecture.

### 5.1 ARM64 Relocation Engine (Simple)
- [ ] Implement `Aarch64RelocationEngine::apply_relocation()`
- [ ] Start with data pointer fixups (same as x86_64)
- [ ] Implement `Aarch64RelocationEngine::unapply_relocation()`
- [ ] Unit test with ARM64 binaries
- [ ] Integration test in ARM64 QEMU

### 5.2 Instruction Detection (Advanced)
- [ ] Detect 4-byte aligned relocations (potential instructions)
- [ ] Identify ADRP instruction encoding (opcode check)
- [ ] Identify ADD instruction encoding
- [ ] Research if Rust UEFI builds use ADRP/ADD for relocations
- [ ] Implement instruction patching if needed
- [ ] Document findings in code comments

### 5.3 ARM64 Testing
- [ ] Build bootloader for aarch64-unknown-uefi
- [ ] Test in QEMU with ARM64 firmware
- [ ] Install to virtual disk
- [ ] Verify boot from persisted image
- [ ] Compare with original binary

**Deliverable**: Self-persisting bootloader on ARM64

---

## Phase 6: Additional Persistence Layers

Multi-layer persistence beyond ESP.

### 6.1 TPM Backend (Layer 1)
- [ ] Research UEFI TPM protocols
- [ ] Implement TPM2 interface
- [ ] Store SHA-256 hash in PCR
- [ ] Implement `TpmBackend::store_bootloader()`
- [ ] Implement verification on boot
- [ ] Test on hardware with TPM

### 6.2 CMOS Backend (Layer 2)
- [ ] Research UEFI variable storage
- [ ] Create minimal recovery stub (< 512 bytes)
- [ ] Implement `CmosBackend::store_bootloader()`
- [ ] Store in UEFI NVRAM variable
- [ ] Implement fallback chainload logic
- [ ] Test recovery scenario

### 6.3 HVRAM Backend (Layer 3)
- [ ] Research hypervisor detection
- [ ] Identify hypercall interface (if virtualized)
- [ ] Design memory hiding strategy
- [ ] Implement `HvramBackend::store_bootloader()`
- [ ] Test under KVM/VMware/Hyper-V
- [ ] Document stealth capabilities

### 6.4 Orchestrator
- [ ] Implement `PersistenceOrchestrator`
- [ ] Add multi-layer storage
- [ ] Add multi-layer verification
- [ ] Add layer priority/fallback logic
- [ ] Integration test all layers

**Deliverable**: Multi-layer persistence system

---

## Phase 7: Security & Obfuscation (Future)

Beyond basic persistence - hardening and stealth.

### 7.1 Signing
- [ ] Research UEFI Secure Boot
- [ ] Generate signing keys
- [ ] Sign persisted images
- [ ] Verify signatures on load
- [ ] Test with Secure Boot enabled

### 7.2 Encryption
- [ ] Design encryption module
- [ ] Choose cipher (AES-256-GCM)
- [ ] Implement key derivation
- [ ] Encrypt bootloader before storage
- [ ] Decrypt on retrieval
- [ ] Test performance impact

### 7.3 Obfuscation
- [ ] Research code polymorphism
- [ ] Implement basic obfuscation
- [ ] Add anti-debugging measures
- [ ] Test against forensic tools

**Deliverable**: Hardened persistence system

---

## Testing Strategy

### Unit Tests
- Parse known PE files
- Test relocation calculations
- Verify apply/unapply symmetry
- Edge cases (corrupted PE, invalid offsets)

### Integration Tests  
- Build bootloader
- Install to test disk
- Reboot and verify
- Compare binaries

### Cross-Platform Tests
- x86_64 QEMU
- ARM64 QEMU  
- Real hardware (if available)

### Regression Tests
- Ensure old bootloaders still work
- Verify backward compatibility

---

## Current Status

**Completed:**
- [x] Architecture designed
- [x] Module structure created
- [x] Platform-specific stubs
- [x] Trait abstractions
- [x] Documentation written

## Getting Started - Practical First Steps

### Step 1: Verify You Have Everything You Need (5 min)

You already have from UEFI:
- `LoadedImageProtocol.image_base` - where your binary is loaded in RAM
- `LoadedImageProtocol.image_size` - size of loaded image
- The entire PE file in memory (relocated)

The PE file in memory contains:
- DOS/PE headers (with current ImageBase)
- All sections including `.reloc` section
- Relocated code/data

**You have everything. No external tools needed.**

### Step 2: Build a PE Header Dumper First (1-2 hours)

Before implementing the whole parser, create a debug tool:

```rust
// In persistent/src/pe/debug.rs or similar
pub fn dump_pe_info(image_base: *const u8) {
    // Read DOS header
    let dos_sig = read_u16(image_base, 0);
    let e_lfanew = read_u32(image_base, 0x3C);
    
    // Read PE sig
    let pe_sig = read_u32(image_base, e_lfanew);
    
    // Read COFF header  
    let machine = read_u16(image_base, e_lfanew + 4);
    let num_sections = read_u16(image_base, e_lfanew + 6);
    
    // Read optional header
    let image_base_field = read_u64(image_base, e_lfanew + 24 + 24);
    
    // Print everything
    log!("DOS sig: 0x{:04X}", dos_sig);
    log!("PE offset: 0x{:X}", e_lfanew);
    log!("Machine: 0x{:04X}", machine);
    log!("Sections: {}", num_sections);
    log!("ImageBase (in header): 0x{:016X}", image_base_field);
    log!("Actual load address: {:p}", image_base);
    
    // Calculate delta
    let delta = (image_base as u64) as i64 - (image_base_field as i64);
    log!("Relocation delta: 0x{:016X}", delta);
}
```

**Why this first?**
- Verifies your understanding of PE format
- Shows you actual values from running bootloader
- Confirms relocation delta calculation
- Takes 1-2 hours, gives immediate feedback

### Step 3: Find and Dump .reloc Section (2-3 hours)

Extend the dumper to locate `.reloc`:

```rust
pub fn dump_reloc_section(image_base: *const u8) {
    // ... parse headers ...
    
    // Find section table
    let section_table_offset = e_lfanew + 24 + opt_header_size;
    
    // Iterate sections
    for i in 0..num_sections {
        let section_offset = section_table_offset + (i * 40);
        let name = read_section_name(image_base, section_offset);
        let virtual_addr = read_u32(image_base, section_offset + 12);
        let raw_size = read_u32(image_base, section_offset + 16);
        
        log!("Section {}: {} @ RVA 0x{:X} (size: {} bytes)", 
             i, name, virtual_addr, raw_size);
        
        if name == ".reloc" {
            log!("Found .reloc section!");
            dump_first_reloc_block(image_base, virtual_addr);
        }
    }
}

fn dump_first_reloc_block(image_base: *const u8, reloc_rva: u32) {
    let reloc_ptr = unsafe { image_base.offset(reloc_rva as isize) };
    let page_rva = read_u32(reloc_ptr, 0);
    let block_size = read_u32(reloc_ptr, 4);
    
    log!("First reloc block:");
    log!("  Page RVA: 0x{:X}", page_rva);
    log!("  Block size: {} bytes", block_size);
    log!("  Entries: {}", (block_size - 8) / 2);
    
    // Dump first few entries
    for i in 0..5 {
        let entry = read_u16(reloc_ptr, 8 + (i * 2));
        let typ = entry >> 12;
        let offset = entry & 0x0FFF;
        log!("    Entry {}: type={} offset=0x{:X}", i, typ, offset);
    }
}
```

**Why this next?**
- Confirms `.reloc` section exists and is readable
- Shows you actual relocation entries
- Verifies the format matches your expectations
- Still just debugging, no complex logic yet

### Step 4: Implement One Relocation Unapply (3-4 hours)

Pick ONE relocation entry and reverse it:

```rust
pub fn test_single_unrelocate(image_base: *mut u8, delta: i64) {
    // Get first .reloc entry
    let reloc_rva = find_reloc_section_rva(image_base);
    let reloc_ptr = unsafe { image_base.offset(reloc_rva as isize) };
    
    let page_rva = read_u32(reloc_ptr, 0);
    let first_entry = read_u16(reloc_ptr, 8);
    
    let typ = first_entry >> 12;
    let offset = first_entry & 0x0FFF;
    
    if typ == 10 {  // DIR64
        let target_rva = page_rva + (offset as u32);
        let target_ptr = unsafe { image_base.offset(target_rva as isize) as *mut u64 };
        
        let old_value = unsafe { *target_ptr };
        log!("Before unrelocate: 0x{:016X}", old_value);
        
        // SUBTRACT delta (reverse the relocation)
        let new_value = (old_value as i64 - delta) as u64;
        unsafe { *target_ptr = new_value };
        
        log!("After unrelocate: 0x{:016X}", new_value);
    }
}
```

**Why this?**
- Proves the concept works
- You can verify before/after values
- Tests with ONE entry before doing all of them
- Builds confidence

### Step 5: Iterate All Relocations (Half day)

Now scale up to all entries:

```rust
pub fn unrelocate_all(image_base: *mut u8, delta: i64) -> Result<usize, PeError> {
    let reloc_rva = find_reloc_section_rva(image_base);
    let reloc_size = find_reloc_section_size(image_base);
    
    let mut offset = 0usize;
    let mut count = 0usize;
    
    while offset < reloc_size as usize {
        let block_ptr = unsafe { image_base.offset((reloc_rva + offset as u32) as isize) };
        
        let page_rva = read_u32(block_ptr, 0);
        let block_size = read_u32(block_ptr, 4) as usize;
        
        if block_size == 0 { break; }
        
        let num_entries = (block_size - 8) / 2;
        
        for i in 0..num_entries {
            let entry = read_u16(block_ptr, 8 + (i * 2));
            let typ = entry >> 12;
            let entry_offset = entry & 0x0FFF;
            
            if typ == 10 {  // DIR64
                let target_rva = page_rva + (entry_offset as u32);
                let target_ptr = unsafe { image_base.offset(target_rva as isize) as *mut u64 };
                
                let old_value = unsafe { *target_ptr };
                let new_value = (old_value as i64 - delta) as u64;
                unsafe { *target_ptr = new_value };
                
                count += 1;
            }
        }
        
        offset += block_size;
    }
    
    log!("Unrelocated {} entries", count);
    Ok(count)
}
```

**Why this?**
- Completes the core functionality
- Still simple, no fancy abstractions yet
- Works for real x86_64 binaries
- Can test immediately

### Step 6: Test It! (Critical)

Add to your installer menu:

```rust
// In bootloader installer menu
"[T] Test unrelocate (debug)" => {
    let captured = MemoryImage::capture(...);
    let bootable = captured.create_bootable_image()?;
    
    // Write to /EFI/TEST/BOOTX64.EFI
    // Reboot and see if it loads
    
    // Compare with original binary
    let original = read_file("/EFI/BOOT/BOOTX64.EFI");
    if bootable == original {
        log!("SUCCESS: Unrelocated image matches original!");
    }
}
```

**This is the moment of truth.**

### Realistic Timeline

- **Hour 1-2**: PE header dumper working, see actual values
- **Hour 3-5**: Find .reloc, dump entries, understand format  
- **Hour 6-9**: Single relocation test, verify it works
- **Hour 10-14**: Full unrelocate loop implementation
- **Hour 15-16**: Integration testing in QEMU
- **Hour 17-20**: Fix bugs, handle edge cases

**Total: ~2-3 days of focused work for basic x86_64 unrelocate.**

Then refactor into proper traits/modules once it works.

---

**Next Steps:**
1. Start with Step 2 (PE dumper) - immediate feedback
2. Don't write full parser yet - just enough to dump info
3. Iterate quickly with small tests
4. Get something working end-to-end before abstracting

**Estimated Timeline:**
- Phase 1: 1-2 weeks
- Phase 2: 1 week
- Phase 3: 1 week
- Phase 4: 1 week
- Phase 5: 2 weeks
- Phase 6: 4+ weeks (research-heavy)

Total: ~10-12 weeks for self-persistence on x86_64 and ARM64

---

**This file tracks actual implementation progress. Update checkboxes as work completes.**
