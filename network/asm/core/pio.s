; ═══════════════════════════════════════════════════════════════════════════
; Port I/O primitives
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; TODO: Implement the following functions:
;   - asm_pio_read8: Read 8-bit from I/O port
;   - asm_pio_write8: Write 8-bit to I/O port
;   - asm_pio_read16: Read 16-bit from I/O port
;   - asm_pio_write16: Write 16-bit to I/O port
;   - asm_pio_read32: Read 32-bit from I/O port
;   - asm_pio_write32: Write 32-bit to I/O port
;
; Reference: ARCHITECTURE_V3.md - PIO layer
; ═══════════════════════════════════════════════════════════════════════════

section .text
