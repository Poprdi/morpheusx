; ═══════════════════════════════════════════════════════════════════════════
; VirtIO RX (Receive) Operations
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_vq_submit_rx: Submit empty buffer to RX queue for receiving
;   - asm_vq_poll_rx: Poll RX used ring for received packets
;   - asm_vq_rx_pending: Check if RX packets are pending
;
; RxResult struct layout (must match Rust #[repr(C)]):
;   +0x00: buffer_idx    (u16) - Index of buffer with received packet
;   +0x02: length        (u16) - Length of received data
;   +0x04: _reserved     (u32)
;
; Differences from TX:
;   - RX descriptors have WRITE flag set (device writes to buffer)
;   - Buffer capacity must be >= 1526 bytes (12-byte VirtIO header + 1514 MTU)
;   - Received length includes VirtIO header
;
; Reference: VirtIO Spec 1.2, NETWORK_IMPL_GUIDE.md §4.7
; ═══════════════════════════════════════════════════════════════════════════

section .data
    ; VirtqueueState offsets (same as tx.s)
    VQ_DESC_BASE        equ 0x00
    VQ_AVAIL_BASE       equ 0x08
    VQ_USED_BASE        equ 0x10
    VQ_QUEUE_SIZE       equ 0x18
    VQ_QUEUE_INDEX      equ 0x1A
    VQ_NOTIFY_ADDR      equ 0x20
    VQ_LAST_USED_IDX    equ 0x28
    VQ_NEXT_AVAIL_IDX   equ 0x2A
    VQ_DESC_CPU_PTR     equ 0x30
    VQ_BUFFER_CPU_BASE  equ 0x38
    VQ_BUFFER_BUS_BASE  equ 0x40
    VQ_BUFFER_SIZE      equ 0x48
    VQ_BUFFER_COUNT     equ 0x4C
    
    ; Ring offsets
    AVAIL_FLAGS         equ 0x00
    AVAIL_IDX           equ 0x02
    AVAIL_RING          equ 0x04
    USED_FLAGS          equ 0x00
    USED_IDX            equ 0x02
    USED_RING           equ 0x04
    
    ; Descriptor offsets
    DESC_ADDR           equ 0x00
    DESC_LEN            equ 0x08
    DESC_FLAGS          equ 0x0C
    DESC_NEXT           equ 0x0E
    
    ; Descriptor flags
    VIRTQ_DESC_F_WRITE  equ 2       ; Device writes to buffer

section .text

; Export symbols
global asm_vq_submit_rx
global asm_vq_poll_rx
global asm_vq_rx_pending

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_submit_rx
; ───────────────────────────────────────────────────────────────────────────
; Submit an empty buffer to the RX virtqueue for receiving
;
; Parameters:
;   RCX = pointer to VirtqueueState
;   DX  = buffer index
;   R8D = buffer capacity (must be >= 1526)
; Returns:
;   EAX = 0 if success
;         1 if queue full
;
; Differences from TX:
;   - Descriptor flags = WRITE (device writes to our buffer)
;   - Buffer length = capacity (maximum device can write)
;
; Barrier sequence (same as TX):
;   1. Write descriptor with WRITE flag
;   2. SFENCE
;   3. Write avail ring entry
;   4. SFENCE
;   5. Increment avail.idx
;   6. MFENCE
; ───────────────────────────────────────────────────────────────────────────
asm_vq_submit_rx:
    push    rbx
    push    r12
    push    r13
    push    r14
    push    r15
    
    mov     r12, rcx            ; VirtqueueState*
    movzx   r13d, dx            ; buffer_idx
    mov     r14d, r8d           ; capacity
    
    ; Get queue size and mask
    movzx   eax, word [r12 + VQ_QUEUE_SIZE]
    dec     eax
    mov     r15d, eax           ; mask
    
    ; Check if queue full
    movzx   ecx, word [r12 + VQ_NEXT_AVAIL_IDX]
    movzx   edx, word [r12 + VQ_LAST_USED_IDX]
    sub     ecx, edx
    and     ecx, 0xFFFF
    movzx   eax, word [r12 + VQ_QUEUE_SIZE]
    cmp     ecx, eax
    jge     .queue_full
    
    ; ─── Step 1: Write descriptor with WRITE flag ───
    ; Descriptor address
    mov     rax, [r12 + VQ_DESC_CPU_PTR]
    mov     ecx, r13d
    shl     ecx, 4
    add     rax, rcx            ; RAX = descriptor pointer
    
    ; Buffer physical address
    mov     rbx, [r12 + VQ_BUFFER_BUS_BASE]
    mov     ecx, [r12 + VQ_BUFFER_SIZE]
    imul    ecx, r13d
    add     rbx, rcx            ; RBX = buffer physical address
    
    ; Write descriptor fields
    mov     [rax + DESC_ADDR], rbx              ; addr
    mov     [rax + DESC_LEN], r14d              ; len = capacity
    mov     word [rax + DESC_FLAGS], VIRTQ_DESC_F_WRITE  ; WRITE flag!
    mov     word [rax + DESC_NEXT], 0
    
    ; ─── Step 2: SFENCE ───
    sfence
    
    ; ─── Step 3: Write avail ring entry ───
    mov     rax, [r12 + VQ_AVAIL_BASE]
    movzx   ecx, word [rax + AVAIL_IDX]
    and     ecx, r15d
    lea     rbx, [rax + AVAIL_RING]
    mov     [rbx + rcx*2], r13w
    
    ; ─── Step 4: SFENCE ───
    sfence
    
    ; ─── Step 5: Increment avail.idx ───
    movzx   ecx, word [rax + AVAIL_IDX]
    inc     ecx
    mov     [rax + AVAIL_IDX], cx
    
    movzx   ecx, word [r12 + VQ_NEXT_AVAIL_IDX]
    inc     ecx
    mov     [r12 + VQ_NEXT_AVAIL_IDX], cx
    
    ; ─── Step 6: MFENCE ───
    mfence
    
    xor     eax, eax
    jmp     .done
    
.queue_full:
    mov     eax, 1
    
.done:
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_poll_rx
; ───────────────────────────────────────────────────────────────────────────
; Poll the RX used ring for received packets
;
; Parameters:
;   RCX = pointer to VirtqueueState
;   RDX = pointer to RxResult struct to populate
; Returns:
;   EAX = 0 if no packet available
;         1 if packet received (RxResult populated)
;
; Barrier sequence:
;   1. Read used.idx (device-written)
;   2. Compare with last_used_idx
;   3. LFENCE - ensure idx read before ring
;   4. Read used ring entry (id, len)
;   5. LFENCE - ensure ring read before buffer access
;   6. Increment last_used_idx
;
; After return:
;   - If 1: Buffer at result->buffer_idx contains packet
;   - Caller should process packet, then resubmit buffer
; ───────────────────────────────────────────────────────────────────────────
asm_vq_poll_rx:
    push    rbx
    push    r12
    
    mov     rbx, rcx            ; VirtqueueState*
    mov     r12, rdx            ; RxResult*
    
    ; ─── Step 1: Read used.idx ───
    mov     rax, [rbx + VQ_USED_BASE]
    movzx   ecx, word [rax + USED_IDX]
    
    ; ─── Step 2: Compare with last_used_idx ───
    movzx   edx, word [rbx + VQ_LAST_USED_IDX]
    cmp     cx, dx
    je      .no_packet
    
    ; ─── Step 3: LFENCE ───
    lfence
    
    ; Calculate ring slot
    movzx   eax, word [rbx + VQ_QUEUE_SIZE]
    dec     eax                 ; mask
    and     edx, eax            ; slot = last_used_idx & mask
    
    ; ─── Step 4: Read used ring entry ───
    ; Each entry: { u32 id, u32 len }
    mov     rax, [rbx + VQ_USED_BASE]
    lea     rax, [rax + USED_RING]
    shl     edx, 3              ; slot * 8
    
    mov     ecx, [rax + rdx]        ; id (buffer index)
    mov     r8d, [rax + rdx + 4]    ; len (bytes received)
    
    ; ─── Step 5: LFENCE ───
    lfence
    
    ; Populate RxResult
    mov     [r12 + 0], cx       ; buffer_idx (u16)
    mov     [r12 + 2], r8w      ; length (u16)
    mov     dword [r12 + 4], 0  ; _reserved
    
    ; ─── Step 6: Increment last_used_idx ───
    movzx   ecx, word [rbx + VQ_LAST_USED_IDX]
    inc     ecx
    mov     [rbx + VQ_LAST_USED_IDX], cx
    
    mov     eax, 1              ; Packet available
    jmp     .done
    
.no_packet:
    xor     eax, eax
    
.done:
    pop     r12
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_rx_pending
; ───────────────────────────────────────────────────────────────────────────
; Check if any RX packets are pending without consuming them
;
; Parameters:
;   RCX = pointer to VirtqueueState
; Returns:
;   EAX = 0 if no packets pending
;         nonzero (count) if packets pending
; ───────────────────────────────────────────────────────────────────────────
asm_vq_rx_pending:
    ; Read used.idx
    mov     rax, [rcx + VQ_USED_BASE]
    movzx   edx, word [rax + USED_IDX]
    
    ; Compare with last_used_idx
    movzx   eax, word [rcx + VQ_LAST_USED_IDX]
    sub     edx, eax
    and     edx, 0xFFFF         ; Handle wraparound
    mov     eax, edx
    ret
