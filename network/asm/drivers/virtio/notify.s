; ═══════════════════════════════════════════════════════════════════════════
; VirtIO Queue Notification
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_vq_notify: Notify device that buffers are available
;   - asm_vq_notify_if_needed: Notify only if device wants notification
;   - asm_vq_set_notify_addr: Set the notification address for a queue
;
; Notification is an MMIO write to the queue's notify address.
; For MMIO VirtIO devices:
;   notify_addr = mmio_base + 0x50 (QueueNotify register)
;   Value written = queue index (0, 1, 2, ...)
;
; Batching: Can batch multiple buffer submissions before notifying.
; Event suppression: Check avail_event to reduce unnecessary notifications.
;
; Reference: VirtIO Spec 1.2 §2.7, NETWORK_IMPL_GUIDE.md §2.2.2
; ═══════════════════════════════════════════════════════════════════════════

section .data
    ; VirtqueueState offsets
    VQ_QUEUE_INDEX      equ 0x1A
    VQ_NOTIFY_ADDR      equ 0x20
    VQ_NEXT_AVAIL_IDX   equ 0x2A
    VQ_USED_BASE        equ 0x10
    VQ_QUEUE_SIZE       equ 0x18
    
    ; Used ring avail_event offset (after ring entries)
    ; avail_event is at: used_base + 4 + (queue_size * 8)
    ; But for simplicity, we'll calculate it

section .text

; Export symbols
global asm_vq_notify
global asm_vq_notify_direct
global asm_vq_should_notify
global asm_vq_set_notify_addr

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_notify
; ───────────────────────────────────────────────────────────────────────────
; Notify device that buffers are available in a queue
;
; Parameters:
;   RCX = pointer to VirtqueueState
; Returns: None
;
; Sequence:
;   1. MFENCE - ensure all prior writes visible
;   2. MMIO write to notify_addr with queue_index
;
; This notifies the device to process the queue. Should be called after
; submitting one or more buffers (can batch submissions).
; ───────────────────────────────────────────────────────────────────────────
asm_vq_notify:
    ; MFENCE before notification
    mfence
    
    ; Get notify address
    mov     rax, [rcx + VQ_NOTIFY_ADDR]
    
    ; Get queue index
    movzx   edx, word [rcx + VQ_QUEUE_INDEX]
    
    ; MMIO write: write queue_index to notify_addr
    mov     [rax], edx
    
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_notify_direct
; ───────────────────────────────────────────────────────────────────────────
; Notify device directly with explicit address and index
;
; Parameters:
;   RCX = MMIO notify address
;   EDX = queue index
; Returns: None
;
; Use when notify_addr is known and VirtqueueState not needed.
; ───────────────────────────────────────────────────────────────────────────
asm_vq_notify_direct:
    mfence
    mov     [rcx], edx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_should_notify
; ───────────────────────────────────────────────────────────────────────────
; Check if device wants notification (event suppression)
;
; Parameters:
;   RCX = pointer to VirtqueueState
; Returns:
;   EAX = 1 if should notify, 0 if notification suppressed
;
; Checks used ring's avail_event field (if VIRTIO_F_EVENT_IDX negotiated).
; We should notify if: avail_idx - avail_event - 1 < (avail_idx - old_avail)
; Simplified: always notify unless explicitly suppressed.
;
; For simplicity, we check the used ring flags field:
;   flags & 1 = VIRTQ_USED_F_NO_NOTIFY
;   If set, device doesn't want notifications.
; ───────────────────────────────────────────────────────────────────────────
asm_vq_should_notify:
    ; Read used.flags
    mov     rax, [rcx + VQ_USED_BASE]
    movzx   eax, word [rax]     ; flags at offset 0
    
    ; Check VIRTQ_USED_F_NO_NOTIFY (bit 0)
    test    eax, 1
    jnz     .no_notify
    
    mov     eax, 1              ; Should notify
    ret
    
.no_notify:
    xor     eax, eax            ; Don't notify
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_set_notify_addr
; ───────────────────────────────────────────────────────────────────────────
; Set the notification MMIO address for a virtqueue
;
; Parameters:
;   RCX = pointer to VirtqueueState
;   RDX = notification MMIO address
; Returns: None
;
; For VirtIO MMIO devices, this is typically:
;   mmio_base + 0x50 (QueueNotify register)
; ───────────────────────────────────────────────────────────────────────────
asm_vq_set_notify_addr:
    mov     [rcx + VQ_NOTIFY_ADDR], rdx
    ret
