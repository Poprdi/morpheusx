; ═══════════════════════════════════════════════════════════════════════════
; VirtIO Virtqueue Setup
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_vq_select: Select a virtqueue by index
;   - asm_vq_get_max_size: Read maximum queue size from device
;   - asm_vq_set_size: Set queue size
;   - asm_vq_set_desc: Set descriptor table address (64-bit)
;   - asm_vq_set_driver: Set available ring address (64-bit)
;   - asm_vq_set_device: Set used ring address (64-bit)
;   - asm_vq_enable: Enable the virtqueue
;   - asm_vq_is_ready: Check if queue is ready
;   - asm_vq_setup: Full queue setup helper
;   - asm_vq_init_desc: Initialize a descriptor entry
;
; Virtqueue Memory Layout:
;   Descriptor Table: 16 bytes per descriptor
;     struct VirtqDesc {
;         u64 addr;    // Buffer physical address
;         u32 len;     // Buffer length
;         u16 flags;   // NEXT=1, WRITE=2, INDIRECT=4
;         u16 next;    // Next descriptor (if NEXT flag)
;     }
;
;   Available Ring: header + ring + event
;     struct VirtqAvail {
;         u16 flags;           // 0 = no interrupt suppress
;         u16 idx;             // Next available slot
;         u16 ring[queue_size];
;         u16 used_event;      // (if VIRTIO_F_EVENT_IDX)
;     }
;
;   Used Ring: header + ring + event
;     struct VirtqUsed {
;         u16 flags;
;         u16 idx;
;         struct VirtqUsedElem ring[queue_size];
;         u16 avail_event;
;     }
;     struct VirtqUsedElem { u32 id; u32 len; }
;
; Reference: VirtIO Spec 1.2 §2.6, NETWORK_IMPL_GUIDE.md §3.3
; ═══════════════════════════════════════════════════════════════════════════

section .data
    ; VirtIO MMIO register offsets (duplicated here for independence)
    VIRTIO_MMIO_QUEUE_SEL       equ 0x030
    VIRTIO_MMIO_QUEUE_NUM_MAX   equ 0x034
    VIRTIO_MMIO_QUEUE_NUM       equ 0x038
    VIRTIO_MMIO_QUEUE_READY     equ 0x044
    VIRTIO_MMIO_QUEUE_DESC_LOW  equ 0x080
    VIRTIO_MMIO_QUEUE_DESC_HIGH equ 0x084
    VIRTIO_MMIO_QUEUE_DRIVER_LOW equ 0x090
    VIRTIO_MMIO_QUEUE_DRIVER_HIGH equ 0x094
    VIRTIO_MMIO_QUEUE_DEVICE_LOW equ 0x0A0
    VIRTIO_MMIO_QUEUE_DEVICE_HIGH equ 0x0A4
    
    ; Descriptor flags
    VIRTQ_DESC_F_NEXT           equ 1
    VIRTQ_DESC_F_WRITE          equ 2
    VIRTQ_DESC_F_INDIRECT       equ 4

section .text

; Export symbols
global asm_vq_select
global asm_vq_get_max_size
global asm_vq_set_size
global asm_vq_set_desc
global asm_vq_set_driver
global asm_vq_set_device
global asm_vq_enable
global asm_vq_disable
global asm_vq_is_ready
global asm_vq_setup
global asm_vq_init_desc
global asm_vq_init_desc_chain

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_select
; ───────────────────────────────────────────────────────────────────────────
; Select a virtqueue for subsequent operations
;
; Parameters:
;   RCX = MMIO base address
;   EDX = queue index (0 = RX, 1 = TX for virtio-net)
; Returns: None
; ───────────────────────────────────────────────────────────────────────────
asm_vq_select:
    mov     [rcx + VIRTIO_MMIO_QUEUE_SEL], edx
    mfence
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_get_max_size
; ───────────────────────────────────────────────────────────────────────────
; Get maximum queue size supported by device
;
; Parameters:
;   RCX = MMIO base address
;   EDX = queue index
; Returns:
;   EAX = maximum queue size (0 if queue not available)
;
; Note: Must select queue first, or this function selects it.
; ───────────────────────────────────────────────────────────────────────────
asm_vq_get_max_size:
    ; Select queue
    mov     [rcx + VIRTIO_MMIO_QUEUE_SEL], edx
    mfence
    
    ; Read max size
    mov     eax, [rcx + VIRTIO_MMIO_QUEUE_NUM_MAX]
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_set_size
; ───────────────────────────────────────────────────────────────────────────
; Set queue size (must be <= max size, and power of 2)
;
; Parameters:
;   RCX = MMIO base address
;   EDX = queue size
; Returns: None
;
; Note: Queue must be selected first via asm_vq_select
; ───────────────────────────────────────────────────────────────────────────
asm_vq_set_size:
    mov     [rcx + VIRTIO_MMIO_QUEUE_NUM], edx
    mfence
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_set_desc
; ───────────────────────────────────────────────────────────────────────────
; Set descriptor table physical address (64-bit)
;
; Parameters:
;   RCX = MMIO base address
;   RDX = 64-bit physical address of descriptor table
; Returns: None
;
; Note: Queue must be selected first
; ───────────────────────────────────────────────────────────────────────────
asm_vq_set_desc:
    ; Write low 32 bits
    mov     eax, edx
    mov     [rcx + VIRTIO_MMIO_QUEUE_DESC_LOW], eax
    
    ; Write high 32 bits
    shr     rdx, 32
    mov     [rcx + VIRTIO_MMIO_QUEUE_DESC_HIGH], edx
    mfence
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_set_driver
; ───────────────────────────────────────────────────────────────────────────
; Set available ring physical address (64-bit)
;
; Parameters:
;   RCX = MMIO base address
;   RDX = 64-bit physical address of available ring
; Returns: None
; ───────────────────────────────────────────────────────────────────────────
asm_vq_set_driver:
    mov     eax, edx
    mov     [rcx + VIRTIO_MMIO_QUEUE_DRIVER_LOW], eax
    shr     rdx, 32
    mov     [rcx + VIRTIO_MMIO_QUEUE_DRIVER_HIGH], edx
    mfence
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_set_device
; ───────────────────────────────────────────────────────────────────────────
; Set used ring physical address (64-bit)
;
; Parameters:
;   RCX = MMIO base address
;   RDX = 64-bit physical address of used ring
; Returns: None
; ───────────────────────────────────────────────────────────────────────────
asm_vq_set_device:
    mov     eax, edx
    mov     [rcx + VIRTIO_MMIO_QUEUE_DEVICE_LOW], eax
    shr     rdx, 32
    mov     [rcx + VIRTIO_MMIO_QUEUE_DEVICE_HIGH], edx
    mfence
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_enable
; ───────────────────────────────────────────────────────────────────────────
; Enable the currently selected virtqueue
;
; Parameters:
;   RCX = MMIO base address
; Returns: None
;
; Note: Queue must be fully configured before enabling
; ───────────────────────────────────────────────────────────────────────────
asm_vq_enable:
    mov     dword [rcx + VIRTIO_MMIO_QUEUE_READY], 1
    mfence
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_disable
; ───────────────────────────────────────────────────────────────────────────
; Disable the currently selected virtqueue
;
; Parameters:
;   RCX = MMIO base address
; Returns: None
; ───────────────────────────────────────────────────────────────────────────
asm_vq_disable:
    mov     dword [rcx + VIRTIO_MMIO_QUEUE_READY], 0
    mfence
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_is_ready
; ───────────────────────────────────────────────────────────────────────────
; Check if currently selected queue is ready/enabled
;
; Parameters:
;   RCX = MMIO base address
; Returns:
;   EAX = 1 if ready, 0 if not
; ───────────────────────────────────────────────────────────────────────────
asm_vq_is_ready:
    mov     eax, [rcx + VIRTIO_MMIO_QUEUE_READY]
    and     eax, 1
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_setup
; ───────────────────────────────────────────────────────────────────────────
; Full queue setup: select, set size, set addresses, enable
;
; Parameters:
;   RCX = MMIO base address
;   EDX = queue index
;   R8  = queue size
;   R9  = pointer to VirtqueueConfig struct:
;         struct VirtqueueConfig {
;             u64 desc_addr;    // +0
;             u64 avail_addr;   // +8
;             u64 used_addr;    // +16
;         }
; Returns:
;   EAX = 0 on success, 1 if queue unavailable
; ───────────────────────────────────────────────────────────────────────────
asm_vq_setup:
    push    rbx
    push    r12
    push    r13
    push    r14
    push    r15
    
    mov     r12, rcx            ; MMIO base
    mov     r13d, edx           ; queue index
    mov     r14d, r8d           ; queue size
    mov     r15, r9             ; config pointer
    
    ; Select queue
    mov     [r12 + VIRTIO_MMIO_QUEUE_SEL], r13d
    mfence
    
    ; Check queue is available (max_size > 0)
    mov     eax, [r12 + VIRTIO_MMIO_QUEUE_NUM_MAX]
    test    eax, eax
    jz      .unavailable
    
    ; Verify requested size <= max
    cmp     r14d, eax
    ja      .unavailable
    
    ; Set queue size
    mov     [r12 + VIRTIO_MMIO_QUEUE_NUM], r14d
    mfence
    
    ; Set descriptor table address
    mov     rax, [r15 + 0]      ; desc_addr
    mov     [r12 + VIRTIO_MMIO_QUEUE_DESC_LOW], eax
    shr     rax, 32
    mov     [r12 + VIRTIO_MMIO_QUEUE_DESC_HIGH], eax
    mfence
    
    ; Set available ring address
    mov     rax, [r15 + 8]      ; avail_addr
    mov     [r12 + VIRTIO_MMIO_QUEUE_DRIVER_LOW], eax
    shr     rax, 32
    mov     [r12 + VIRTIO_MMIO_QUEUE_DRIVER_HIGH], eax
    mfence
    
    ; Set used ring address
    mov     rax, [r15 + 16]     ; used_addr
    mov     [r12 + VIRTIO_MMIO_QUEUE_DEVICE_LOW], eax
    shr     rax, 32
    mov     [r12 + VIRTIO_MMIO_QUEUE_DEVICE_HIGH], eax
    mfence
    
    ; Enable queue
    mov     dword [r12 + VIRTIO_MMIO_QUEUE_READY], 1
    mfence
    
    xor     eax, eax            ; Success
    jmp     .done
    
.unavailable:
    mov     eax, 1              ; Failure
    
.done:
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_init_desc
; ───────────────────────────────────────────────────────────────────────────
; Initialize a single descriptor entry
;
; Parameters:
;   RCX = pointer to descriptor (16 bytes)
;   RDX = buffer physical address
;   R8D = buffer length
;   R9W = flags (NEXT=1, WRITE=2, INDIRECT=4)
;   [RSP+40] = next descriptor index (word)
; Returns: None
;
; Simplified: use stack parameter or fixed next=0
; ───────────────────────────────────────────────────────────────────────────
asm_vq_init_desc:
    ; Write addr (offset 0, 8 bytes)
    mov     [rcx + 0], rdx
    
    ; Write len (offset 8, 4 bytes)
    mov     [rcx + 8], r8d
    
    ; Write flags (offset 12, 2 bytes)
    mov     [rcx + 12], r9w
    
    ; Write next (offset 14, 2 bytes)
    ; Get from stack if NEXT flag set, otherwise 0
    test    r9w, VIRTQ_DESC_F_NEXT
    jz      .no_next
    mov     ax, [rsp + 40]      ; 5th param from stack (after shadow space)
    mov     [rcx + 14], ax
    ret
    
.no_next:
    xor     eax, eax
    mov     [rcx + 14], ax
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_vq_init_desc_chain
; ───────────────────────────────────────────────────────────────────────────
; Initialize a chain of descriptors pointing to contiguous buffers
;
; Parameters:
;   RCX = pointer to first descriptor
;   RDX = base buffer physical address
;   R8D = number of descriptors
;   R9D = buffer size per descriptor
;   [RSP+40] = flags for all except last (typically NEXT)
;   [RSP+48] = flags for last descriptor (typically WRITE for RX)
; Returns:
;   EAX = number of descriptors initialized
; ───────────────────────────────────────────────────────────────────────────
asm_vq_init_desc_chain:
    push    rbx
    push    r12
    push    r13
    push    r14
    push    r15
    
    mov     r12, rcx            ; desc pointer
    mov     r13, rdx            ; buffer addr
    mov     r14d, r8d           ; count
    mov     r15d, r9d           ; buffer size
    
    ; Get flags from stack
    mov     bx, [rsp + 40 + 40] ; chain flags (after saves + shadow)
    mov     cx, [rsp + 48 + 40] ; last flags
    
    xor     eax, eax            ; index counter
    
.loop:
    cmp     eax, r14d
    jge     .done
    
    ; Calculate descriptor pointer: desc_base + (index * 16)
    mov     r8, rax
    shl     r8, 4
    add     r8, r12
    
    ; Write addr
    mov     [r8 + 0], r13
    
    ; Write len
    mov     [r8 + 8], r15d
    
    ; Check if last descriptor
    mov     edx, eax
    inc     edx
    cmp     edx, r14d
    jge     .last_desc
    
    ; Not last: write chain flags and next index
    mov     [r8 + 12], bx
    mov     [r8 + 14], dx       ; next = current + 1
    jmp     .next
    
.last_desc:
    ; Last descriptor: write last flags, next = 0
    mov     [r8 + 12], cx
    mov     word [r8 + 14], 0
    
.next:
    ; Advance buffer address
    add     r13, r15            ; Next buffer
    inc     eax
    jmp     .loop
    
.done:
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret
