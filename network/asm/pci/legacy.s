; ═══════════════════════════════════════════════════════════════════════════
; PCI Legacy Configuration Space Access (CF8/CFC)
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; TODO: Implement the following functions:
;   - asm_pci_legacy_read8: Read 8-bit from PCI config space
;   - asm_pci_legacy_read16: Read 16-bit from PCI config space
;   - asm_pci_legacy_read32: Read 32-bit from PCI config space
;   - asm_pci_legacy_write8: Write 8-bit to PCI config space
;   - asm_pci_legacy_write16: Write 16-bit to PCI config space
;   - asm_pci_legacy_write32: Write 32-bit to PCI config space
;
; PCI Config Address Format (CF8h):
;   Bit 31: Enable bit (1)
;   Bits 23-16: Bus number
;   Bits 15-11: Device number
;   Bits 10-8: Function number
;   Bits 7-0: Register offset (aligned)
;
; Reference: ARCHITECTURE_V3.md - PCI layer
; ═══════════════════════════════════════════════════════════════════════════

section .text
