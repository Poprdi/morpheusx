; ═══════════════════════════════════════════════════════════════════════════
; VirtIO TX (Transmit) Operations
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; TODO: Implement the following functions:
;   - asm_vq_submit_tx: Submit buffer to TX queue with barriers
;       Params: (vq_state*, buffer_idx, buffer_len) -> 0=ok, 1=full
;       Barrier sequence:
;         1. Write descriptor
;         2. SFENCE
;         3. Write avail ring entry
;         4. SFENCE
;         5. Increment avail.idx
;         6. MFENCE
;   - asm_vq_poll_tx_complete: Poll TX used ring for completions
;       Params: (vq_state*) -> buffer_idx or 0xFFFFFFFF if none
;
; CRITICAL: Fire-and-forget TX - never wait for completion!
;
; Reference: NETWORK_IMPL_GUIDE.md §2.4, §4.6
; ═══════════════════════════════════════════════════════════════════════════

section .text
