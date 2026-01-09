; ═══════════════════════════════════════════════════════════════════════════
; MII (Media Independent Interface) Register Definitions
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; TODO: Implement the following functions:
;   - asm_mii_read_bmcr: Read Basic Mode Control Register (reg 0)
;   - asm_mii_write_bmcr: Write Basic Mode Control Register
;   - asm_mii_read_bmsr: Read Basic Mode Status Register (reg 1)
;   - asm_mii_read_phyid: Read PHY ID registers (reg 2-3)
;   - asm_mii_read_anar: Read Auto-Neg Advertisement Register (reg 4)
;   - asm_mii_read_anlpar: Read Auto-Neg Link Partner Ability (reg 5)
;
; Standard MII Registers:
;   0 - BMCR (Basic Mode Control)
;   1 - BMSR (Basic Mode Status)
;   2 - PHYID1 (PHY ID high)
;   3 - PHYID2 (PHY ID low)
;   4 - ANAR (Auto-Neg Advertisement)
;   5 - ANLPAR (Auto-Neg Link Partner)
;
; Reference: IEEE 802.3 Clause 22, ARCHITECTURE_V3.md
; ═══════════════════════════════════════════════════════════════════════════

section .text
