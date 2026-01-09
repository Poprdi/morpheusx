; ═══════════════════════════════════════════════════════════════════════════
; VirtIO RX (Receive) Operations
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; TODO: Implement the following functions:
;   - asm_vq_submit_rx: Submit empty buffer to RX queue
;       Params: (vq_state*, buffer_idx, capacity) -> 0=ok, 1=full
;       Buffer capacity must be >= 1526 (12-byte header + 1514 MTU)
;   - asm_vq_poll_rx: Poll RX used ring for received packets
;       Params: (vq_state*, result*) -> 0=empty, 1=packet
;       Barrier sequence:
;         1. Read used.idx (volatile)
;         2. LFENCE
;         3. Read used ring entry
;         4. LFENCE before buffer access
;
; Reference: NETWORK_IMPL_GUIDE.md §2.4, §4.7
; ═══════════════════════════════════════════════════════════════════════════

section .text
