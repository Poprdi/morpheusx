; ═══════════════════════════════════════════════════════════════════════════
; Delay/timing primitives
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; TODO: Implement the following functions:
;   - asm_tsc_delay_us: Delay for N microseconds using TSC
;   - asm_spin_loop: CPU hint for spin loop (PAUSE instruction)
;
; Reference: ARCHITECTURE_V3.md - delay primitives
; ═══════════════════════════════════════════════════════════════════════════

section .text
