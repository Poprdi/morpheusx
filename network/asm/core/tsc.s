; ═══════════════════════════════════════════════════════════════════════════
; TSC (Time Stamp Counter) primitives
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; TODO: Implement the following functions:
;   - asm_tsc_read: Read TSC (~40 cycles, non-serializing)
;   - asm_tsc_read_serialized: Read TSC with CPUID serialize (~200 cycles)
;
; Reference: NETWORK_IMPL_GUIDE.md §2.2.1
; ═══════════════════════════════════════════════════════════════════════════

section .text
