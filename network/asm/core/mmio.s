; ═══════════════════════════════════════════════════════════════════════════
; MMIO (Memory-Mapped I/O) primitives
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_mmio_read32: Read 32-bit from MMIO address
;   - asm_mmio_write32: Write 32-bit to MMIO address
;   - asm_mmio_read16: Read 16-bit from MMIO address
;   - asm_mmio_write16: Write 16-bit to MMIO address
;   - asm_mmio_read8: Read 8-bit from MMIO address
;   - asm_mmio_write8: Write 8-bit to MMIO address
;
; CRITICAL: These are simple loads/stores. The caller is responsible for
; appropriate barriers before/after. The standalone ASM call acts as a
; compiler barrier (compiler cannot reorder across function call).
;
; Reference: NETWORK_IMPL_GUIDE.md §2.2.1
; ═══════════════════════════════════════════════════════════════════════════

section .text

; Export symbols
global asm_mmio_read32
global asm_mmio_write32
global asm_mmio_read16
global asm_mmio_write16
global asm_mmio_read8
global asm_mmio_write8

; ───────────────────────────────────────────────────────────────────────────
; asm_mmio_read32
; ───────────────────────────────────────────────────────────────────────────
; Read 32-bit value from MMIO address
;
; Parameters:
;   RCX = MMIO address (must be 4-byte aligned)
; Returns:
;   RAX = 32-bit value (zero-extended to 64-bit)
; Clobbers: None
;
; Safety: Address must be valid MMIO address, 4-byte aligned
; ───────────────────────────────────────────────────────────────────────────
asm_mmio_read32:
    mov     eax, [rcx]          ; 32-bit load from address in RCX
    ret                         ; Return value in RAX (upper 32 bits zeroed)

; ───────────────────────────────────────────────────────────────────────────
; asm_mmio_write32
; ───────────────────────────────────────────────────────────────────────────
; Write 32-bit value to MMIO address
;
; Parameters:
;   RCX = MMIO address (must be 4-byte aligned)
;   RDX = 32-bit value to write (in low 32 bits)
; Returns: None
; Clobbers: None
;
; Safety: Address must be valid MMIO address, 4-byte aligned
; ───────────────────────────────────────────────────────────────────────────
asm_mmio_write32:
    mov     [rcx], edx          ; 32-bit store to address in RCX
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_mmio_read16
; ───────────────────────────────────────────────────────────────────────────
; Read 16-bit value from MMIO address
;
; Parameters:
;   RCX = MMIO address (must be 2-byte aligned)
; Returns:
;   RAX = 16-bit value (zero-extended to 64-bit)
; Clobbers: None
; ───────────────────────────────────────────────────────────────────────────
asm_mmio_read16:
    xor     eax, eax            ; Clear RAX (ensures upper bits are zero)
    mov     ax, [rcx]           ; 16-bit load
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_mmio_write16
; ───────────────────────────────────────────────────────────────────────────
; Write 16-bit value to MMIO address
;
; Parameters:
;   RCX = MMIO address (must be 2-byte aligned)
;   RDX = 16-bit value to write (in low 16 bits)
; Returns: None
; Clobbers: None
; ───────────────────────────────────────────────────────────────────────────
asm_mmio_write16:
    mov     [rcx], dx           ; 16-bit store
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_mmio_read8
; ───────────────────────────────────────────────────────────────────────────
; Read 8-bit value from MMIO address
;
; Parameters:
;   RCX = MMIO address
; Returns:
;   RAX = 8-bit value (zero-extended to 64-bit)
; Clobbers: None
; ───────────────────────────────────────────────────────────────────────────
asm_mmio_read8:
    xor     eax, eax            ; Clear RAX
    mov     al, [rcx]           ; 8-bit load
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_mmio_write8
; ───────────────────────────────────────────────────────────────────────────
; Write 8-bit value to MMIO address
;
; Parameters:
;   RCX = MMIO address
;   RDX = 8-bit value to write (in low 8 bits)
; Returns: None
; Clobbers: None
; ───────────────────────────────────────────────────────────────────────────
asm_mmio_write8:
    mov     [rcx], dl           ; 8-bit store
    ret
