; ═══════════════════════════════════════════════════════════════════════════
; Memory barrier primitives
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_bar_sfence: Store fence - orders all prior stores
;   - asm_bar_lfence: Load fence - orders all prior loads
;   - asm_bar_mfence: Full memory fence - orders all prior loads AND stores
;
; These barriers are CRITICAL for DMA correctness. The compiler cannot
; reorder across external function calls, and these instructions prevent
; CPU reordering.
;
; Reference: NETWORK_IMPL_GUIDE.md §2.2.1, §2.4
; ═══════════════════════════════════════════════════════════════════════════

section .text

; Export symbols
global asm_bar_sfence
global asm_bar_lfence
global asm_bar_mfence

; ───────────────────────────────────────────────────────────────────────────
; asm_bar_sfence
; ───────────────────────────────────────────────────────────────────────────
; Store Fence - ensures all prior stores are globally visible
;
; Use BEFORE notifying device that data is ready (e.g., after writing
; descriptor, before incrementing avail.idx)
;
; Parameters: None
; Returns: None
; Clobbers: None
; ───────────────────────────────────────────────────────────────────────────
asm_bar_sfence:
    sfence                      ; Wait for all prior stores to complete
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_bar_lfence
; ───────────────────────────────────────────────────────────────────────────
; Load Fence - ensures all prior loads complete before subsequent loads
;
; Use AFTER reading an index from device, BEFORE reading data at that index
; (e.g., after reading used.idx, before reading the used ring entry)
;
; Parameters: None
; Returns: None
; Clobbers: None
; ───────────────────────────────────────────────────────────────────────────
asm_bar_lfence:
    lfence                      ; Wait for all prior loads to complete
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_bar_mfence
; ───────────────────────────────────────────────────────────────────────────
; Full Memory Fence - ensures all prior loads AND stores complete
;
; Use when both load and store ordering is required, such as:
;   - Before MMIO write to notify register (ensures all descriptor writes visible)
;   - Between reading device state and acting on it
;
; This is the strongest barrier and has the highest latency (~20-40 cycles).
;
; Parameters: None
; Returns: None
; Clobbers: None
; ───────────────────────────────────────────────────────────────────────────
asm_bar_mfence:
    mfence                      ; Full memory barrier
    ret
