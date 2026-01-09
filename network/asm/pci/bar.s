; ═══════════════════════════════════════════════════════════════════════════
; PCI BAR (Base Address Register) helpers
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; TODO: Implement the following functions:
;   - asm_pci_bar_read: Read BAR value
;   - asm_pci_bar_size: Determine BAR size (write 0xFFFFFFFF, read back)
;   - asm_pci_bar_type: Determine BAR type (MMIO vs I/O, 32-bit vs 64-bit)
;
; BAR Format:
;   Bit 0: 0=Memory, 1=I/O
;   For Memory BARs:
;     Bits 2-1: Type (00=32-bit, 10=64-bit)
;     Bit 3: Prefetchable
;     Bits 31-4: Base address (4KB aligned)
;
; Reference: ARCHITECTURE_V3.md - PCI layer
; ═══════════════════════════════════════════════════════════════════════════

section .text
