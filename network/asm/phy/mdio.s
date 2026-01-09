; ═══════════════════════════════════════════════════════════════════════════
; MDIO (Management Data I/O) - PHY Register Access
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; TODO: Implement the following functions:
;   - asm_mdio_read_c22: Clause 22 MDIO read (PHY addr, reg) -> value
;   - asm_mdio_write_c22: Clause 22 MDIO write (PHY addr, reg, value)
;   - asm_mdio_read_c45: Clause 45 MDIO read (port, dev, reg) -> value
;   - asm_mdio_write_c45: Clause 45 MDIO write (port, dev, reg, value)
;
; MDIO Frame Format (Clause 22):
;   Preamble: 32 1-bits
;   Start: 01
;   Op: 10=read, 01=write
;   PHY addr: 5 bits
;   Reg addr: 5 bits
;   Turnaround: 2 bits
;   Data: 16 bits
;
; Reference: IEEE 802.3 Clause 22/45, ARCHITECTURE_V3.md
; ═══════════════════════════════════════════════════════════════════════════

section .text
