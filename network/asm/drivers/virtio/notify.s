; ═══════════════════════════════════════════════════════════════════════════
; VirtIO Queue Notification
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; TODO: Implement the following functions:
;   - asm_vq_notify: Notify device that buffers are available
;       Params: (vq_state*)
;       Sequence: MFENCE then MMIO write to notify_addr
;
; Reference: NETWORK_IMPL_GUIDE.md §2.2.2
; ═══════════════════════════════════════════════════════════════════════════

section .text
