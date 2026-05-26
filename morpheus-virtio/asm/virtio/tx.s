; ═══════════════════════════════════════════════════════════════════════════
; VirtIO TX (Transmit) Operations
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_vq_submit_tx: Submit buffer to TX queue with proper barriers
;   - asm_vq_poll_tx_complete: Poll TX used ring for completions
;   - asm_vq_tx_avail_slots: Get number of available TX slots
;
; VirtqueueState struct layout (must match Rust #[repr(C)]):
;   +0x00: desc_base       (u64) - Physical addr of descriptor table
;   +0x08: avail_base      (u64) - Physical addr of available ring
;   +0x10: used_base       (u64) - Physical addr of used ring
;   +0x18: queue_size      (u16) - Number of descriptors
;   +0x1A: queue_index     (u16) - Queue index (0=RX, 1=TX)
;   +0x1C: _pad            (u32)
;   +0x20: notify_addr     (u64) - MMIO address for notification
;   +0x28: last_used_idx   (u16) - Last seen used index
;   +0x2A: next_avail_idx  (u16) - Next available index to use
;   +0x2C: _pad2           (u32)
;   +0x30: desc_cpu_ptr    (u64) - CPU pointer to descriptors
;   +0x38: buffer_cpu_base (u64) - CPU pointer to buffer region
;   +0x40: buffer_bus_base (u64) - Bus addr of buffer region
;   +0x48: buffer_size     (u32) - Size of each buffer
;   +0x4C: buffer_count    (u32) - Number of buffers
;
; Avail Ring Layout:
;   +0x00: flags           (u16)
;   +0x02: idx             (u16) - Next slot to write
;   +0x04: ring[N]         (u16 each)
;
; Used Ring Layout:
;   +0x00: flags           (u16)
;   +0x02: idx             (u16) - Next slot device writes
;   +0x04: ring[N]         (struct { u32 id, u32 len })
;
; CRITICAL: TX is fire-and-forget - NEVER wait for completion!
;           Completions collected in main loop Phase 5.
;
; Reference: VirtIO Spec 1.2, NETWORK_IMPL_GUIDE.md §2.4, §4.6
; ═══════════════════════════════════════════════════════════════════════════

section .data
    ; VirtqueueState offsets
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
    
    ; Avail ring offsets
    AVAIL_FLAGS         equ 0x00
    AVAIL_IDX           equ 0x02
    AVAIL_RING          equ 0x04
    
    ; Used ring offsets
    USED_FLAGS          equ 0x00
    USED_IDX            equ 0x02
    USED_RING           equ 0x04     ; Each entry: u32 id, u32 len
    
    ; Descriptor offsets
    DESC_ADDR           equ 0x00
    DESC_LEN            equ 0x08
    DESC_FLAGS          equ 0x0C
    DESC_NEXT           equ 0x0E

section .text

; Export symbols
global asm_vq_submit_tx
global asm_vq_poll_tx_complete
global asm_vq_tx_avail_slots

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_submit_tx
; ───────────────────────────────────────────────────────────────────────────
; Submit a buffer to the TX virtqueue with correct barrier sequence
;
; Parameters:
;   RCX = pointer to VirtqueueState
;   DX  = buffer index (which buffer slot to use)
;   R8D = buffer length (including VirtIO header)
; Returns:
;   EAX = 0 if success (buffer submitted)
;         1 if queue full (buffer NOT submitted)
;
; Barrier sequence (per VirtIO spec and AUDIT):
;   1. Write descriptor entry
;   2. SFENCE - ensure descriptor visible before ring
;   3. Write avail.ring[avail.idx & mask] = desc_idx
;   4. SFENCE - ensure ring entry visible before idx
;   5. Increment avail.idx
;   6. MFENCE - full barrier before notify decision
;
; Buffer ownership:
;   - Caller must mark buffer as DEVICE-OWNED before calling
;   - Buffer remains DEVICE-OWNED until poll_tx_complete returns it
; ───────────────────────────────────────────────────────────────────────────
asm_vq_submit_tx:
    push    rbx
    push    r12
    push    r13
    push    r14
    push    r15
    
    mov     r12, rcx            ; VirtqueueState*
    movzx   r13d, dx            ; buffer_idx
    mov     r14d, r8d           ; buffer_len
    
    ; Get queue size and calculate mask
    movzx   eax, word [r12 + VQ_QUEUE_SIZE]
    dec     eax                 ; mask = queue_size - 1
    mov     r15d, eax           ; R15 = mask
    
    ; Check if queue is full
    ; Queue is full if: (next_avail_idx - last_used_idx) >= queue_size
    movzx   ecx, word [r12 + VQ_NEXT_AVAIL_IDX]
    movzx   edx, word [r12 + VQ_LAST_USED_IDX]
    sub     ecx, edx            ; ecx = used slots (with wraparound)
    and     ecx, 0xFFFF         ; Handle u16 wraparound
    movzx   eax, word [r12 + VQ_QUEUE_SIZE]
    cmp     ecx, eax
    jge     .queue_full
    
    ; ─── Step 1: Write descriptor ───
    ; Descriptor address = desc_cpu_ptr + (buffer_idx * 16)
    mov     rax, [r12 + VQ_DESC_CPU_PTR]
    mov     ecx, r13d
    shl     ecx, 4              ; * 16
    add     rax, rcx            ; RAX = descriptor pointer
    
    ; Buffer physical address = buffer_bus_base + (buffer_idx * buffer_size)
    mov     rbx, [r12 + VQ_BUFFER_BUS_BASE]
    mov     ecx, [r12 + VQ_BUFFER_SIZE]
    imul    ecx, r13d           ; ecx = buffer_idx * buffer_size
    add     rbx, rcx            ; RBX = buffer physical address
    
    ; Write descriptor fields
    mov     [rax + DESC_ADDR], rbx      ; addr = buffer physical
    mov     [rax + DESC_LEN], r14d      ; len = buffer_len
    mov     word [rax + DESC_FLAGS], 0  ; flags = 0 (no NEXT, no WRITE)
    mov     word [rax + DESC_NEXT], 0   ; next = 0 (unused)
    
    ; ─── Step 2: SFENCE ───
    sfence
    
    ; ─── Step 3: Write avail ring entry ───
    ; avail.ring[avail.idx & mask] = buffer_idx
    mov     rax, [r12 + VQ_AVAIL_BASE]  ; avail ring base
    movzx   ecx, word [rax + AVAIL_IDX] ; current avail.idx
    and     ecx, r15d                   ; & mask
    lea     rbx, [rax + AVAIL_RING]     ; ring array start
    mov     [rbx + rcx*2], r13w         ; ring[slot] = buffer_idx
    
    ; ─── Step 4: SFENCE ───
    sfence
    
    ; ─── Step 5: Increment avail.idx ───
    movzx   ecx, word [rax + AVAIL_IDX]
    inc     ecx
    mov     [rax + AVAIL_IDX], cx
    
    ; Update our tracking
    movzx   ecx, word [r12 + VQ_NEXT_AVAIL_IDX]
    inc     ecx
    mov     [r12 + VQ_NEXT_AVAIL_IDX], cx
    
    ; ─── Step 6: MFENCE ───
    mfence
    
    ; Success
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
; asm_vq_poll_tx_complete
; ───────────────────────────────────────────────────────────────────────────
; Poll the TX used ring for completed transmissions
;
; Parameters:
;   RCX = pointer to VirtqueueState
; Returns:
;   EAX = buffer index if completion available (0x0000-0xFFFE)
;         0xFFFFFFFF if no completion available
;
; This function:
;   1. Compares used.idx with our last_used_idx
;   2. If different, reads used.ring[last_used_idx & mask]
;   3. Extracts buffer index from used entry
;   4. Increments last_used_idx
;   5. Returns buffer index (now DRIVER-OWNED again)
;
; Caller must call this repeatedly until 0xFFFFFFFF to drain all completions.
; ───────────────────────────────────────────────────────────────────────────
asm_vq_poll_tx_complete:
    push    rbx
    
    mov     rbx, rcx            ; VirtqueueState*
    
    ; Read used.idx (device updates this)
    mov     rax, [rbx + VQ_USED_BASE]
    movzx   ecx, word [rax + USED_IDX]
    
    ; Compare with our last_used_idx
    movzx   edx, word [rbx + VQ_LAST_USED_IDX]
    cmp     cx, dx
    je      .no_completion      ; Equal = no new completions
    
    ; LFENCE before reading ring entry (ensure idx read completes)
    lfence
    
    ; Calculate ring slot: last_used_idx & mask
    movzx   eax, word [rbx + VQ_QUEUE_SIZE]
    dec     eax                 ; mask
    and     edx, eax            ; slot = last_used_idx & mask
    
    ; Read used.ring[slot].id (buffer index)
    ; Each used entry is 8 bytes: u32 id, u32 len
    mov     rax, [rbx + VQ_USED_BASE]
    lea     rax, [rax + USED_RING]
    shl     edx, 3              ; slot * 8
    mov     eax, [rax + rdx]    ; EAX = id (buffer index)
    
    ; LFENCE after reading ring entry
    lfence
    
    ; Increment our last_used_idx
    movzx   ecx, word [rbx + VQ_LAST_USED_IDX]
    inc     ecx
    mov     [rbx + VQ_LAST_USED_IDX], cx
    
    ; Return buffer index (already in EAX)
    jmp     .done
    
.no_completion:
    mov     eax, 0xFFFFFFFF
    
.done:
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_tx_avail_slots
; ───────────────────────────────────────────────────────────────────────────
; Get number of available slots in TX queue
;
; Parameters:
;   RCX = pointer to VirtqueueState
; Returns:
;   EAX = number of available slots
;
; Available = queue_size - (next_avail_idx - last_used_idx)
; ───────────────────────────────────────────────────────────────────────────
asm_vq_tx_avail_slots:
    ; Get queue_size
    movzx   eax, word [rcx + VQ_QUEUE_SIZE]
    
    ; Get used count
    movzx   edx, word [rcx + VQ_NEXT_AVAIL_IDX]
    movzx   r8d, word [rcx + VQ_LAST_USED_IDX]
    sub     edx, r8d
    and     edx, 0xFFFF         ; Handle wraparound
    
    ; Available = queue_size - used
    sub     eax, edx
    ret
