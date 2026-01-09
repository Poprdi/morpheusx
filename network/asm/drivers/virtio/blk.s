; ═══════════════════════════════════════════════════════════════════════════
; VirtIO Block Device Operations
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; VirtIO-blk uses the same virtqueue mechanism as virtio-net but with
; different request structures for block I/O operations.
;
; Request Structure (3-descriptor chain):
;   Descriptor 0: VirtioBlkReqHeader (16 bytes, device-readable)
;   Descriptor 1: Data buffer (n bytes, read or write depending on op)
;   Descriptor 2: Status byte (1 byte, device-writable)
;
; VirtioBlkReqHeader:
;   u32 type     - VIRTIO_BLK_T_IN (read) or VIRTIO_BLK_T_OUT (write)
;   u32 reserved
;   u64 sector   - Starting sector number
;
; Status:
;   0 = VIRTIO_BLK_S_OK
;   1 = VIRTIO_BLK_S_IOERR  
;   2 = VIRTIO_BLK_S_UNSUPP
;
; Reference: VirtIO Spec 1.2 §5.2
; ═══════════════════════════════════════════════════════════════════════════

section .data
    ; VirtIO-blk request types
    VIRTIO_BLK_T_IN         equ 0   ; Read from device
    VIRTIO_BLK_T_OUT        equ 1   ; Write to device
    VIRTIO_BLK_T_FLUSH      equ 4   ; Flush (if supported)
    VIRTIO_BLK_T_GET_ID     equ 8   ; Get device ID
    VIRTIO_BLK_T_DISCARD    equ 11  ; Discard (if supported)
    VIRTIO_BLK_T_WRITE_ZEROES equ 13 ; Write zeros (if supported)
    
    ; VirtIO-blk status values
    VIRTIO_BLK_S_OK         equ 0   ; Success
    VIRTIO_BLK_S_IOERR      equ 1   ; I/O error
    VIRTIO_BLK_S_UNSUPP     equ 2   ; Unsupported operation
    
    ; VirtIO-blk feature bits (low 32)
    VIRTIO_BLK_F_SIZE_MAX   equ (1 << 1)   ; Max segment size
    VIRTIO_BLK_F_SEG_MAX    equ (1 << 2)   ; Max segments
    VIRTIO_BLK_F_GEOMETRY   equ (1 << 4)   ; Geometry available
    VIRTIO_BLK_F_RO         equ (1 << 5)   ; Read-only device
    VIRTIO_BLK_F_BLK_SIZE   equ (1 << 6)   ; Block size available
    VIRTIO_BLK_F_FLUSH      equ (1 << 9)   ; Flush command supported
    VIRTIO_BLK_F_TOPOLOGY   equ (1 << 10)  ; Topology info available
    VIRTIO_BLK_F_MQ         equ (1 << 12)  ; Multi-queue
    VIRTIO_BLK_F_DISCARD    equ (1 << 13)  ; Discard supported
    VIRTIO_BLK_F_WRITE_ZEROES equ (1 << 14) ; Write zeroes supported
    
    ; VirtIO-blk config space offsets (from VIRTIO_MMIO_CONFIG = 0x100)
    VIRTIO_BLK_CFG_CAPACITY     equ 0x00  ; u64 - total sectors
    VIRTIO_BLK_CFG_SIZE_MAX     equ 0x08  ; u32 - max segment size
    VIRTIO_BLK_CFG_SEG_MAX      equ 0x0C  ; u32 - max segments
    VIRTIO_BLK_CFG_GEOMETRY     equ 0x10  ; geometry struct (if F_GEOMETRY)
    VIRTIO_BLK_CFG_BLK_SIZE     equ 0x14  ; u32 - logical block size
    
    ; Descriptor flags
    VIRTQ_DESC_F_NEXT       equ 1   ; More descriptors in chain
    VIRTQ_DESC_F_WRITE      equ 2   ; Device writes (vs reads)
    VIRTQ_DESC_F_INDIRECT   equ 4   ; Indirect descriptor

    ; MMIO offsets (from init.s)
    VIRTIO_MMIO_CONFIG      equ 0x100

section .text

; External: Core primitives
extern asm_tsc_read
extern asm_bar_mfence
extern asm_bar_sfence
extern asm_bar_lfence

; ═══════════════════════════════════════════════════════════════════════════
; asm_virtio_blk_read_capacity
; ═══════════════════════════════════════════════════════════════════════════
; Read total capacity (sectors) from VirtIO-blk config space.
;
; Parameters:
;   RCX = mmio_base (VirtIO MMIO base address)
;
; Returns:
;   RAX = capacity in 512-byte sectors (u64)
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_virtio_blk_read_capacity
asm_virtio_blk_read_capacity:
    ; Config space at mmio_base + 0x100 + offset
    ; Capacity is at offset 0 within config space
    
    ; Read low 32 bits
    mov rax, [rcx + VIRTIO_MMIO_CONFIG + VIRTIO_BLK_CFG_CAPACITY]
    ; Read high 32 bits
    mov rdx, [rcx + VIRTIO_MMIO_CONFIG + VIRTIO_BLK_CFG_CAPACITY + 4]
    shl rdx, 32
    or rax, rdx
    
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_virtio_blk_read_blk_size
; ═══════════════════════════════════════════════════════════════════════════
; Read logical block size from config space (if F_BLK_SIZE negotiated).
;
; Parameters:
;   RCX = mmio_base
;
; Returns:
;   EAX = block size (usually 512)
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_virtio_blk_read_blk_size
asm_virtio_blk_read_blk_size:
    mov eax, [rcx + VIRTIO_MMIO_CONFIG + VIRTIO_BLK_CFG_BLK_SIZE]
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_virtio_blk_submit_read
; ═══════════════════════════════════════════════════════════════════════════
; Submit a read request to VirtIO-blk queue.
;
; Parameters:
;   RCX = *VirtqueueState (queue state pointer)
;   RDX = sector (u64 - starting sector number)
;   R8  = data_buf_phys (physical address of data buffer)
;   R9  = num_sectors (number of 512-byte sectors to read)
;   [RSP+40] = header_buf_phys (physical address of request header)
;   [RSP+48] = status_buf_phys (physical address of status byte)
;   [RSP+56] = desc_idx (first of 3 consecutive descriptor indices)
;
; Returns:
;   EAX = 0 if success, 1 if queue full
;
; VirtqueueState layout (must match Rust):
;   +0x00: desc_base (u64)      - physical addr of descriptor table
;   +0x08: avail_base (u64)     - physical addr of available ring
;   +0x10: used_base (u64)      - physical addr of used ring
;   +0x18: queue_size (u16)
;   +0x1A: queue_index (u16)
;   +0x1C: _pad (u32)
;   +0x20: notify_addr (u64)    - MMIO address for notify
;   +0x28: last_used_idx (u16)
;   +0x2A: next_avail_idx (u16)
;
; Descriptor layout (16 bytes each):
;   +0x00: addr (u64)   - buffer physical address
;   +0x08: len (u32)    - buffer length
;   +0x0C: flags (u16)  - NEXT, WRITE, INDIRECT
;   +0x0E: next (u16)   - next descriptor index
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_virtio_blk_submit_read
asm_virtio_blk_submit_read:
    push rbx
    push rsi
    push rdi
    push r12
    push r13
    push r14
    push r15
    
    ; Save parameters
    mov r12, rcx            ; VirtqueueState*
    mov r13, rdx            ; sector
    mov r14, r8             ; data_buf_phys
    mov r15, r9             ; num_sectors
    
    ; Get stack parameters (accounting for pushes + shadow space)
    ; 7 pushes * 8 = 56, return addr = 8, total = 64
    ; Shadow space on entry = 32
    ; So stack params start at RSP + 64 + 32 + 8 = RSP + 104... actually simpler:
    ; After pushes: RSP+56 (7 regs) points to return addr
    ; Stack params at original: RSP+40, RSP+48, RSP+56
    ; After push: RSP+40+56=RSP+96, RSP+48+56=RSP+104, RSP+56+56=RSP+112
    mov rax, [rsp + 96]     ; header_buf_phys
    mov rbx, [rsp + 104]    ; status_buf_phys  
    movzx esi, word [rsp + 112] ; desc_idx
    
    ; --- Check if queue has space ---
    ; Load queue_size and next_avail_idx
    movzx ecx, word [r12 + 0x18]  ; queue_size
    movzx edx, word [r12 + 0x2A]  ; next_avail_idx
    movzx edi, word [r12 + 0x28]  ; last_used_idx
    
    ; Check if (avail - used) >= queue_size (queue full)
    sub edx, edi                  ; pending = avail - used
    cmp edx, ecx
    jge .queue_full
    
    ; --- Build request header (VIRTIO_BLK_T_IN = 0 for read) ---
    ; header_buf_phys in rax
    mov dword [rax], VIRTIO_BLK_T_IN  ; type = IN (read)
    mov dword [rax + 4], 0            ; reserved
    mov [rax + 8], r13                ; sector
    
    ; --- Setup 3-descriptor chain ---
    mov rdi, [r12]                    ; desc_base
    
    ; Descriptor 0: Header (device-readable)
    ; Index = desc_idx
    movzx ecx, si                     ; desc_idx
    shl ecx, 4                        ; * 16 for offset
    add rdi, rcx                      ; desc[0] pointer
    
    mov [rdi], rax                    ; addr = header_buf_phys
    mov dword [rdi + 8], 16           ; len = 16 (header size)
    lea eax, [esi + 1]                ; next = desc_idx + 1
    mov word [rdi + 0x0C], VIRTQ_DESC_F_NEXT  ; flags = NEXT
    mov word [rdi + 0x0E], ax         ; next
    
    ; Descriptor 1: Data buffer (device-writable for read)
    add rdi, 16                       ; move to next descriptor
    mov [rdi], r14                    ; addr = data_buf_phys
    
    ; Calculate data length: num_sectors * 512
    mov eax, r15d
    shl eax, 9                        ; * 512
    mov [rdi + 8], eax                ; len
    
    lea eax, [esi + 2]                ; next = desc_idx + 2
    mov word [rdi + 0x0C], VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE  ; NEXT | WRITE
    mov word [rdi + 0x0E], ax         ; next
    
    ; Descriptor 2: Status byte (device-writable)
    add rdi, 16                       ; move to next descriptor
    mov [rdi], rbx                    ; addr = status_buf_phys
    mov dword [rdi + 8], 1            ; len = 1 (status byte)
    mov word [rdi + 0x0C], VIRTQ_DESC_F_WRITE  ; flags = WRITE (no NEXT)
    mov word [rdi + 0x0E], 0          ; next = 0 (unused)
    
    ; --- Store fence before updating available ring ---
    sfence
    
    ; --- Update available ring ---
    mov rdi, [r12 + 0x08]             ; avail_base
    movzx ecx, word [r12 + 0x2A]      ; next_avail_idx
    movzx eax, word [r12 + 0x18]      ; queue_size
    dec eax                           ; mask = size - 1
    and ecx, eax                      ; ring_idx = avail_idx & mask
    
    ; avail->ring[ring_idx] = desc_idx (head of chain)
    lea rdi, [rdi + 4]                ; &avail->ring[0]
    mov word [rdi + rcx*2], si        ; ring[idx] = desc_idx
    
    ; Store fence before incrementing index
    sfence
    
    ; Increment next_avail_idx
    movzx eax, word [r12 + 0x2A]
    inc ax
    mov [r12 + 0x2A], ax
    
    ; Update avail->idx
    mov rdi, [r12 + 0x08]             ; avail_base
    mov [rdi + 2], ax                 ; avail->idx = next_avail
    
    ; Full fence before notify
    mfence
    
    ; Success
    xor eax, eax
    jmp .done
    
.queue_full:
    mov eax, 1
    
.done:
    pop r15
    pop r14
    pop r13
    pop r12
    pop rdi
    pop rsi
    pop rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_virtio_blk_submit_write
; ═══════════════════════════════════════════════════════════════════════════
; Submit a write request to VirtIO-blk queue.
;
; Parameters:
;   RCX = *VirtqueueState
;   RDX = sector (u64)
;   R8  = data_buf_phys
;   R9  = num_sectors
;   [RSP+40] = header_buf_phys
;   [RSP+48] = status_buf_phys
;   [RSP+56] = desc_idx
;
; Returns:
;   EAX = 0 if success, 1 if queue full
;
; Same as read, but:
;   - type = VIRTIO_BLK_T_OUT (1)
;   - Data descriptor is device-readable (no WRITE flag)
; ═══════════════════════════════════════════════════════════════════════════
global asm_virtio_blk_submit_write
asm_virtio_blk_submit_write:
    push rbx
    push rsi
    push rdi
    push r12
    push r13
    push r14
    push r15
    
    mov r12, rcx
    mov r13, rdx
    mov r14, r8
    mov r15, r9
    
    mov rax, [rsp + 96]     ; header_buf_phys
    mov rbx, [rsp + 104]    ; status_buf_phys
    movzx esi, word [rsp + 112] ; desc_idx
    
    ; Check queue space
    movzx ecx, word [r12 + 0x18]
    movzx edx, word [r12 + 0x2A]
    movzx edi, word [r12 + 0x28]
    sub edx, edi
    cmp edx, ecx
    jge .queue_full_w
    
    ; Build header (VIRTIO_BLK_T_OUT = 1 for write)
    mov dword [rax], VIRTIO_BLK_T_OUT
    mov dword [rax + 4], 0
    mov [rax + 8], r13
    
    ; Setup descriptors
    mov rdi, [r12]
    movzx ecx, si
    shl ecx, 4
    add rdi, rcx
    
    ; Desc 0: Header (readable)
    mov [rdi], rax
    mov dword [rdi + 8], 16
    lea eax, [esi + 1]
    mov word [rdi + 0x0C], VIRTQ_DESC_F_NEXT
    mov word [rdi + 0x0E], ax
    
    ; Desc 1: Data (readable - no WRITE flag for writes!)
    add rdi, 16
    mov [rdi], r14
    mov eax, r15d
    shl eax, 9
    mov [rdi + 8], eax
    lea eax, [esi + 2]
    mov word [rdi + 0x0C], VIRTQ_DESC_F_NEXT  ; NO WRITE flag
    mov word [rdi + 0x0E], ax
    
    ; Desc 2: Status (writable)
    add rdi, 16
    mov [rdi], rbx
    mov dword [rdi + 8], 1
    mov word [rdi + 0x0C], VIRTQ_DESC_F_WRITE
    mov word [rdi + 0x0E], 0
    
    ; Barriers and update avail ring
    sfence
    
    mov rdi, [r12 + 0x08]
    movzx ecx, word [r12 + 0x2A]
    movzx eax, word [r12 + 0x18]
    dec eax
    and ecx, eax
    lea rdi, [rdi + 4]
    mov word [rdi + rcx*2], si
    
    sfence
    
    movzx eax, word [r12 + 0x2A]
    inc ax
    mov [r12 + 0x2A], ax
    mov rdi, [r12 + 0x08]
    mov [rdi + 2], ax
    
    mfence
    
    xor eax, eax
    jmp .done_w
    
.queue_full_w:
    mov eax, 1
    
.done_w:
    pop r15
    pop r14
    pop r13
    pop r12
    pop rdi
    pop rsi
    pop rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_virtio_blk_poll_complete
; ═══════════════════════════════════════════════════════════════════════════
; Poll for completed block I/O requests.
;
; Parameters:
;   RCX = *VirtqueueState
;   RDX = *BlkPollResult - output struct:
;         +0x00: desc_idx (u16) - head descriptor index
;         +0x02: status (u8)    - VirtIO-blk status
;         +0x03: _pad (u8)
;         +0x04: bytes_written (u32) - bytes device wrote
;
; Returns:
;   EAX = 0 if no completion available
;   EAX = 1 if completion found (result populated)
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_virtio_blk_poll_complete
asm_virtio_blk_poll_complete:
    push rbx
    
    ; Load used ring info
    mov rax, [rcx + 0x10]             ; used_base
    movzx r8d, word [rax + 2]         ; used->idx
    movzx r9d, word [rcx + 0x28]      ; last_used_idx
    
    ; Check if any completions
    cmp r8w, r9w
    je .no_completion
    
    ; Load fence before reading used ring entry
    lfence
    
    ; Get ring entry: used->ring[last_used & mask]
    movzx eax, word [rcx + 0x18]      ; queue_size
    dec eax                           ; mask
    and r9d, eax                      ; idx & mask
    
    mov rax, [rcx + 0x10]             ; used_base
    lea rax, [rax + 4]                ; &used->ring[0]
    
    ; Each used element is 8 bytes: u32 id, u32 len
    shl r9d, 3                        ; idx * 8
    add rax, r9
    
    ; Read completion
    mov r10d, [rax]                   ; desc_idx (id)
    mov r11d, [rax + 4]               ; bytes_written (len)
    
    ; Load fence after reading ring entry
    lfence
    
    ; Increment last_used_idx
    movzx eax, word [rcx + 0x28]
    inc ax
    mov [rcx + 0x28], ax
    
    ; Fill result struct
    mov [rdx], r10w                   ; desc_idx
    ; Status is in the status buffer - caller must read it
    ; We just return the descriptor info
    mov byte [rdx + 2], 0xFF          ; status placeholder (caller reads from buffer)
    mov byte [rdx + 3], 0             ; pad
    mov [rdx + 4], r11d               ; bytes_written
    
    mov eax, 1
    jmp .done_poll
    
.no_completion:
    xor eax, eax
    
.done_poll:
    pop rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_virtio_blk_notify
; ═══════════════════════════════════════════════════════════════════════════
; Notify VirtIO-blk device that requests are available.
;
; Parameters:
;   RCX = *VirtqueueState
;
; Returns:
;   Nothing
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_virtio_blk_notify
asm_virtio_blk_notify:
    ; Full fence before notify
    mfence
    
    ; Write queue index to notify address
    mov rax, [rcx + 0x20]             ; notify_addr
    movzx edx, word [rcx + 0x1A]      ; queue_index
    mov dword [rax], edx              ; MMIO write
    
    ret
