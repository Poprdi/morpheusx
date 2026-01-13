; ═══════════════════════════════════════════════════════════════════════════
; AHCI Read/Write DMA Operations
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; High-level block I/O operations for read and write.
;
; Functions:
;   - asm_ahci_submit_read: Submit a read request (non-blocking)
;   - asm_ahci_submit_write: Submit a write request (non-blocking)
;
; Reference: AHCI 1.3.1 Specification, ATA/ATAPI-8
; ═══════════════════════════════════════════════════════════════════════════

section .data
    %include "asm/drivers/ahci/regs.s"

section .text

; External
extern asm_bar_sfence
extern asm_bar_mfence
extern asm_ahci_build_h2d_fis
extern asm_ahci_build_prdt
extern asm_ahci_setup_cmd_header
extern asm_ahci_issue_cmd

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_submit_read
; ═══════════════════════════════════════════════════════════════════════════
; Submit a DMA read request (fire-and-forget).
;
; Parameters:
;   RCX = abar
;   EDX = port_num
;   R8  = lba (starting sector)
;   R9  = data_buf_phys (physical address of data buffer)
;   [RSP+40] = num_sectors (number of 512-byte sectors)
;   [RSP+48] = cmd_slot (command slot to use, 0-31)
;   [RSP+56] = cmd_header_ptr (CPU pointer to this slot's command header)
;   [RSP+64] = cmd_table_ptr (CPU pointer to this slot's command table)
;   [RSP+72] = cmd_table_phys (physical address of command table)
;
; Returns:
;   EAX = 0 on submit success, 1 if port busy
;
; Note: This is fire-and-forget. Caller must poll for completion.
;       Buffer must remain valid until completion.
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_submit_read
asm_ahci_submit_read:
    push    rbx
    push    rsi
    push    rdi
    push    r12
    push    r13
    push    r14
    push    r15
    sub     rsp, 48

    ; Save register parameters
    mov     r12, rcx            ; abar
    mov     r13d, edx           ; port_num
    mov     r14, r8             ; lba
    mov     r15, r9             ; data_buf_phys

    ; Get stack parameters
    ; 7 pushes × 8 = 56, + sub 48 = 104 total
    ; Stack params at RSP+104+40, etc.
    mov     ebx, [rsp + 104 + 40]   ; num_sectors
    mov     esi, [rsp + 104 + 48]   ; cmd_slot
    mov     rdi, [rsp + 104 + 56]   ; cmd_header_ptr
    mov     rax, [rsp + 104 + 64]   ; cmd_table_ptr
    mov     [rsp], rax              ; save locally
    mov     rax, [rsp + 104 + 72]   ; cmd_table_phys
    mov     [rsp + 8], rax          ; save locally

    ; ───────────────────────────────────────────────────────────────────
    ; Build H2D FIS for READ DMA EXT (0x25)
    ; ───────────────────────────────────────────────────────────────────
    mov     rcx, [rsp]          ; cmd_table_ptr (FIS at offset 0)
    mov     dl, ATA_CMD_READ_DMA_EXT  ; 0x25
    mov     r8, r14             ; lba
    mov     r9w, bx             ; num_sectors
    call    asm_ahci_build_h2d_fis

    ; ───────────────────────────────────────────────────────────────────
    ; Build PRDT (single entry for contiguous buffer)
    ; ───────────────────────────────────────────────────────────────────
    mov     rcx, [rsp]
    add     rcx, 0x80           ; PRDT at cmd_table + 0x80
    mov     rdx, r15            ; data_buf_phys
    
    ; Calculate byte count - 1
    mov     eax, ebx            ; num_sectors
    shl     eax, 9              ; * 512
    dec     eax                 ; - 1
    mov     r8d, eax
    call    asm_ahci_build_prdt

    ; ───────────────────────────────────────────────────────────────────
    ; Setup command header
    ; CFL = 5, W = 0 (read), PRDTL = 1
    ; ───────────────────────────────────────────────────────────────────
    mov     rcx, rdi            ; cmd_header_ptr
    mov     edx, (1 << 16) | 5  ; PRDTL=1, CFL=5
    mov     r8, [rsp + 8]       ; cmd_table_phys
    call    asm_ahci_setup_cmd_header

    ; ───────────────────────────────────────────────────────────────────
    ; Issue command
    ; ───────────────────────────────────────────────────────────────────
    mov     rcx, r12            ; abar
    mov     edx, r13d           ; port_num
    mov     eax, 1
    mov     ecx, esi            ; cmd_slot
    shl     eax, cl             ; slot_mask = 1 << slot
    mov     r8d, eax
    mov     rcx, r12
    call    asm_ahci_issue_cmd

    xor     eax, eax            ; success

    add     rsp, 48
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rdi
    pop     rsi
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_submit_write
; ═══════════════════════════════════════════════════════════════════════════
; Submit a DMA write request (fire-and-forget).
;
; Parameters:
;   RCX = abar
;   EDX = port_num
;   R8  = lba (starting sector)
;   R9  = data_buf_phys (physical address of data buffer)
;   [RSP+40] = num_sectors
;   [RSP+48] = cmd_slot
;   [RSP+56] = cmd_header_ptr
;   [RSP+64] = cmd_table_ptr
;   [RSP+72] = cmd_table_phys
;
; Returns:
;   EAX = 0 on submit success
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_submit_write
asm_ahci_submit_write:
    push    rbx
    push    rsi
    push    rdi
    push    r12
    push    r13
    push    r14
    push    r15
    sub     rsp, 48

    mov     r12, rcx            ; abar
    mov     r13d, edx           ; port_num
    mov     r14, r8             ; lba
    mov     r15, r9             ; data_buf_phys

    ; 7 pushes × 8 = 56, + sub 48 = 104 total
    mov     ebx, [rsp + 104 + 40]   ; num_sectors
    mov     esi, [rsp + 104 + 48]   ; cmd_slot
    mov     rdi, [rsp + 104 + 56]   ; cmd_header_ptr
    mov     rax, [rsp + 104 + 64]   ; cmd_table_ptr
    mov     [rsp], rax
    mov     rax, [rsp + 104 + 72]   ; cmd_table_phys
    mov     [rsp + 8], rax

    ; ───────────────────────────────────────────────────────────────────
    ; Build H2D FIS for WRITE DMA EXT (0x35)
    ; ───────────────────────────────────────────────────────────────────
    mov     rcx, [rsp]
    mov     dl, ATA_CMD_WRITE_DMA_EXT ; 0x35
    mov     r8, r14
    mov     r9w, bx
    call    asm_ahci_build_h2d_fis

    ; ───────────────────────────────────────────────────────────────────
    ; Build PRDT
    ; ───────────────────────────────────────────────────────────────────
    mov     rcx, [rsp]
    add     rcx, 0x80
    mov     rdx, r15
    mov     eax, ebx
    shl     eax, 9
    dec     eax
    mov     r8d, eax
    call    asm_ahci_build_prdt

    ; ───────────────────────────────────────────────────────────────────
    ; Setup command header
    ; CFL = 5, W = 1 (write), PRDTL = 1
    ; ───────────────────────────────────────────────────────────────────
    mov     rcx, rdi
    ; Set W bit (bit 6) for write direction
    mov     edx, (1 << 16) | AHCI_CMD_WRITE | 5  ; PRDTL=1, W=1, CFL=5
    mov     r8, [rsp + 8]
    call    asm_ahci_setup_cmd_header

    ; ───────────────────────────────────────────────────────────────────
    ; Issue command
    ; ───────────────────────────────────────────────────────────────────
    mov     rcx, r12
    mov     edx, r13d
    mov     eax, 1
    mov     ecx, esi
    shl     eax, cl
    mov     r8d, eax
    mov     rcx, r12
    call    asm_ahci_issue_cmd

    xor     eax, eax

    add     rsp, 48
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rdi
    pop     rsi
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_flush_cache
; ═══════════════════════════════════════════════════════════════════════════
; Issue FLUSH CACHE EXT command.
;
; Parameters:
;   RCX = abar
;   EDX = port_num
;   R8  = cmd_slot
;   R9  = cmd_header_ptr
;   [RSP+40] = cmd_table_ptr
;   [RSP+48] = cmd_table_phys
;
; Returns:
;   EAX = 0 on success
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_flush_cache
asm_ahci_flush_cache:
    push    rbx
    push    r12
    push    r13
    push    r14
    sub     rsp, 40

    mov     r12, rcx            ; abar
    mov     r13d, edx           ; port_num
    mov     ebx, r8d            ; cmd_slot
    mov     r14, r9             ; cmd_header_ptr

    mov     rax, [rsp + 40 + 32 + 40]  ; cmd_table_ptr
    mov     [rsp], rax
    mov     rax, [rsp + 40 + 32 + 48]  ; cmd_table_phys
    mov     [rsp + 8], rax

    ; Build H2D FIS for FLUSH CACHE EXT (0xEA)
    mov     rcx, [rsp]
    mov     dl, ATA_CMD_FLUSH_CACHE_EXT
    xor     r8, r8              ; LBA = 0
    xor     r9w, r9w            ; count = 0
    call    asm_ahci_build_h2d_fis

    ; Setup command header (no data transfer, no PRDT)
    mov     rcx, r14
    mov     edx, 5              ; CFL=5, no W, PRDTL=0
    mov     r8, [rsp + 8]
    call    asm_ahci_setup_cmd_header

    ; Issue command
    mov     rcx, r12
    mov     edx, r13d
    mov     eax, 1
    mov     ecx, ebx
    shl     eax, cl
    mov     r8d, eax
    mov     rcx, r12
    call    asm_ahci_issue_cmd

    xor     eax, eax

    add     rsp, 40
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret
