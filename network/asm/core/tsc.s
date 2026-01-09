; ═══════════════════════════════════════════════════════════════════════════
; TSC (Time Stamp Counter) primitives
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_tsc_read: Read TSC (~40 cycles, non-serializing)
;   - asm_tsc_read_serialized: Read TSC with CPUID serialize (~200 cycles)
;
; Reference: NETWORK_IMPL_GUIDE.md §2.2.1
; ═══════════════════════════════════════════════════════════════════════════

section .text

; Export symbols
global asm_tsc_read
global asm_tsc_read_serialized

; ───────────────────────────────────────────────────────────────────────────
; asm_tsc_read
; ───────────────────────────────────────────────────────────────────────────
; Read Time Stamp Counter (non-serializing, ~40 cycles)
;
; The RDTSC instruction reads the processor's time-stamp counter into
; EDX:EAX. This is NOT serializing - instructions may be reordered around it.
; Use for low-overhead timing where slight inaccuracy is acceptable.
;
; Parameters: None
; Returns: RAX = 64-bit TSC value
; Clobbers: RDX (used internally, combined into RAX)
; ───────────────────────────────────────────────────────────────────────────
asm_tsc_read:
    rdtsc                       ; EDX:EAX = TSC (low in EAX, high in EDX)
    shl     rdx, 32             ; Shift high 32 bits to upper half of RDX
    or      rax, rdx            ; Combine: RAX = (EDX << 32) | EAX
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_tsc_read_serialized
; ───────────────────────────────────────────────────────────────────────────
; Read TSC with full CPU serialization (~200 cycles)
;
; Uses CPUID to serialize the instruction stream before reading TSC.
; This ensures all prior instructions have completed before the read.
; Use for precise measurements where accuracy is critical.
;
; Parameters: None
; Returns: RAX = 64-bit TSC value
; Clobbers: RBX, RCX, RDX (CPUID clobbers these)
;
; Note: Microsoft x64 ABI requires preserving RBX, so we save/restore it.
; ───────────────────────────────────────────────────────────────────────────
asm_tsc_read_serialized:
    push    rbx                 ; Save RBX (non-volatile in MS x64 ABI)
    
    ; Serialize with CPUID (leaf 0 is always valid)
    xor     eax, eax            ; CPUID leaf 0
    cpuid                       ; Serializing instruction - waits for all
                                ; prior instructions to complete
    
    ; Now read TSC
    rdtsc                       ; EDX:EAX = TSC
    shl     rdx, 32             ; Shift high bits
    or      rax, rdx            ; Combine into RAX
    
    pop     rbx                 ; Restore RBX
    ret
