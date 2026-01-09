; ═══════════════════════════════════════════════════════════════════════════
; PCIe ECAM (Enhanced Configuration Access Mechanism)
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; TODO: Implement the following functions:
;   - asm_pci_ecam_read8: Read 8-bit from PCIe ECAM space
;   - asm_pci_ecam_read16: Read 16-bit from PCIe ECAM space
;   - asm_pci_ecam_read32: Read 32-bit from PCIe ECAM space
;   - asm_pci_ecam_write8: Write 8-bit to PCIe ECAM space
;   - asm_pci_ecam_write16: Write 16-bit to PCIe ECAM space
;   - asm_pci_ecam_write32: Write 32-bit to PCIe ECAM space
;
; ECAM Address Format:
;   Base + (Bus << 20) + (Device << 15) + (Function << 12) + Register
;
; Reference: ARCHITECTURE_V3.md - PCI layer
; ═══════════════════════════════════════════════════════════════════════════

section .text
