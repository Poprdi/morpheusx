; ═══════════════════════════════════════════════════════════════════════════
; Link Status Detection
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; TODO: Implement the following functions:
;   - asm_link_status: Check link status from PHY
;       Params: (mmio_base) -> 0=down, 1=up
;   - asm_link_speed: Detect negotiated link speed
;       Params: (mmio_base) -> speed in Mbps (10, 100, 1000, 10000)
;   - asm_link_duplex: Detect duplex mode
;       Params: (mmio_base) -> 0=half, 1=full
;
; Reference: ARCHITECTURE_V3.md - PHY layer
; ═══════════════════════════════════════════════════════════════════════════

section .text
