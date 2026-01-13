; ═══════════════════════════════════════════════════════════════════════════
; AHCI Command Submission and I/O Operations
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Block-level read/write operations using AHCI command infrastructure.
;
; Functions:
;   - asm_ahci_setup_cmd_header: Configure a command header entry
;   - asm_ahci_setup_cmd_table: Build command FIS and PRDT
;   - asm_ahci_issue_cmd: Issue a command to the port
;   - asm_ahci_poll_cmd: Poll for command completion
;   - asm_ahci_submit_read: High-level read submission
;   - asm_ahci_submit_write: High-level write submission
;
; DMA Structures:
;
; Command List (1K aligned, one per port):
;   32 Command Headers × 32 bytes each = 1024 bytes
;
; Command Header (32 bytes):
;   +0x00: DW0 - Flags (PRDTL[15:0], PMP[15:12], C, B, R, P, W, A, CFL[4:0])
;   +0x04: DW1 - PRDBC (Physical Region Descriptor Byte Count)
;   +0x08: DW2 - CTBA (Command Table Base Address low)
;   +0x0C: DW3 - CTBAU (Command Table Base Address high)
;   +0x10-0x1F: Reserved
;
; Command Table (128-byte aligned, one per command slot):
;   +0x00-0x3F: CFIS (Command FIS, 64 bytes)
;   +0x40-0x4F: ACMD (ATAPI Command, 16 bytes)
;   +0x50-0x7F: Reserved
;   +0x80+:     PRDT entries (16 bytes each)
;
; PRDT Entry (16 bytes):
;   +0x00: DBA (Data Base Address low)
;   +0x04: DBAU (Data Base Address high)
;   +0x08: Reserved
;   +0x0C: DBC (Data Byte Count [21:0], I bit [31])
;
; FIS Receive Area (256-byte aligned):
;   +0x00: DSFIS (DMA Setup FIS)
;   +0x20: PSFIS (PIO Setup FIS)
;   +0x40: RFIS (Register - Device to Host FIS)
;   +0x58: SDBFIS (Set Device Bits FIS)
;   +0x60: UFIS (Unknown FIS)
;   +0xA0-0xFF: Reserved
;
; Reference: AHCI 1.3.1 Specification §4
; ═══════════════════════════════════════════════════════════════════════════

section .data
    %include "asm/drivers/ahci/regs.s"

section .text

; External: Core primitives
extern asm_tsc_read
extern asm_bar_sfence
extern asm_bar_lfence
extern asm_bar_mfence
extern asm_mmio_read32
extern asm_mmio_write32

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_setup_cmd_header
; ═══════════════════════════════════════════════════════════════════════════
; Configure a command header in the command list.
;
; Parameters:
;   RCX = cmd_header_ptr (CPU pointer to command header)
;   EDX = flags (CFL, W, P, C, PRDTL in upper 16 bits)
;   R8  = ctba_phys (Command Table Base Address, physical)
;
; Command Header DW0 Format:
;   Bits 4:0   - CFL (Command FIS Length in DWORDs, min 2 for H2D)
;   Bit 5     - A (ATAPI)
;   Bit 6     - W (Write, 1 = device reads data)
;   Bit 7     - P (Prefetchable)
;   Bit 8     - R (Reset)
;   Bit 9     - B (BIST)
;   Bit 10    - C (Clear Busy upon R_OK)
;   Bits 15:12 - PMP (Port Multiplier Port)
;   Bits 31:16 - PRDTL (Physical Region Descriptor Table Length)
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_setup_cmd_header
asm_ahci_setup_cmd_header:
    ; DW0: flags
    mov     [rcx], edx

    ; DW1: PRDBC = 0 (updated by HBA after completion)
    mov     dword [rcx + 4], 0

    ; DW2: CTBA (low 32 bits)
    mov     eax, r8d
    mov     [rcx + 8], eax

    ; DW3: CTBAU (high 32 bits)
    mov     rax, r8
    shr     rax, 32
    mov     [rcx + 12], eax

    ; DW4-7: Reserved (zero)
    mov     qword [rcx + 16], 0
    mov     qword [rcx + 24], 0

    sfence
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_build_h2d_fis
; ═══════════════════════════════════════════════════════════════════════════
; Build a Register Host-to-Device FIS for a read/write command.
;
; Parameters:
;   RCX = fis_ptr (CPU pointer to FIS area in command table)
;   DL  = command (ATA command: 0x25=READ DMA EXT, 0x35=WRITE DMA EXT)
;   R8  = lba (48-bit LBA)
;   R9W = sector_count (number of sectors, 0 = 65536)
;
; H2D FIS Format (20 bytes, 5 DWORDs):
;   Byte 0: FIS Type (0x27 for H2D)
;   Byte 1: C|PM|reserved (C=1 means command register update)
;   Byte 2: Command
;   Byte 3: Features (low)
;   Byte 4: LBA[7:0]
;   Byte 5: LBA[15:8]
;   Byte 6: LBA[23:16]
;   Byte 7: Device (0x40 for LBA mode)
;   Byte 8: LBA[31:24]
;   Byte 9: LBA[39:32]
;   Byte 10: LBA[47:40]
;   Byte 11: Features (high)
;   Byte 12: Count[7:0]
;   Byte 13: Count[15:8]
;   Byte 14: ICC
;   Byte 15: Control
;   Bytes 16-19: Reserved
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_build_h2d_fis
asm_ahci_build_h2d_fis:
    push    rbx

    ; Save parameters
    mov     rbx, rcx            ; fis_ptr
    mov     al, dl              ; command
    mov     r10, r8             ; lba
    mov     r11w, r9w           ; sector_count

    ; Clear FIS area first (20 bytes, zero the full CFIS area for safety)
    mov     qword [rbx], 0
    mov     qword [rbx + 8], 0
    mov     dword [rbx + 16], 0

    ; Byte 0: FIS Type = 0x27 (H2D)
    mov     byte [rbx], FIS_TYPE_REG_H2D

    ; Byte 1: C=1 (command update), PM=0
    mov     byte [rbx + 1], 0x80

    ; Byte 2: Command
    mov     [rbx + 2], al

    ; Byte 3: Features low = 0
    mov     byte [rbx + 3], 0

    ; Bytes 4-6: LBA low 24 bits
    mov     rax, r10
    mov     [rbx + 4], al       ; LBA[7:0]
    shr     rax, 8
    mov     [rbx + 5], al       ; LBA[15:8]
    shr     rax, 8
    mov     [rbx + 6], al       ; LBA[23:16]

    ; Byte 7: Device = 0x40 (LBA mode, no device select for SATA)
    mov     byte [rbx + 7], 0x40

    ; Bytes 8-10: LBA high 24 bits
    mov     rax, r10
    shr     rax, 24
    mov     [rbx + 8], al       ; LBA[31:24]
    shr     rax, 8
    mov     [rbx + 9], al       ; LBA[39:32]
    shr     rax, 8
    mov     [rbx + 10], al      ; LBA[47:40]

    ; Byte 11: Features high = 0
    mov     byte [rbx + 11], 0

    ; Bytes 12-13: Sector count
    mov     [rbx + 12], r11b    ; Count[7:0]
    shr     r11w, 8
    mov     [rbx + 13], r11b    ; Count[15:8]

    ; Bytes 14-15: ICC=0, Control=0
    mov     byte [rbx + 14], 0
    mov     byte [rbx + 15], 0

    sfence

    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_build_prdt
; ═══════════════════════════════════════════════════════════════════════════
; Build a single PRDT entry for data transfer.
;
; Parameters:
;   RCX = prdt_ptr (CPU pointer to PRDT entry)
;   RDX = data_phys (physical address of data buffer)
;   R8D = byte_count (bytes to transfer - 1, max 4MB-1 per entry)
;
; PRDT Entry (16 bytes):
;   +0x00: DBA (Data Base Address low)
;   +0x04: DBAU (Data Base Address high)
;   +0x08: Reserved
;   +0x0C: DBC (bits 21:0 = byte count - 1, bit 31 = interrupt on completion)
;
; Note: byte_count should be actual bytes - 1 (e.g., 512 bytes -> pass 511)
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_build_prdt
asm_ahci_build_prdt:
    ; DBA low
    mov     eax, edx
    mov     [rcx], eax

    ; DBAU high
    mov     rax, rdx
    shr     rax, 32
    mov     [rcx + 4], eax

    ; Reserved
    mov     dword [rcx + 8], 0

    ; DBC (byte count - 1, no interrupt)
    ; Ensure bit 31 (I) is clear for no interrupt
    and     r8d, 0x3FFFFF       ; Mask to 22 bits
    mov     [rcx + 12], r8d

    sfence
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_issue_cmd
; ═══════════════════════════════════════════════════════════════════════════
; Issue a command by setting the CI (Command Issue) bit.
;
; Parameters:
;   RCX = abar
;   EDX = port_num
;   R8D = slot_mask (bit mask of slots to issue, e.g., 1<<0 for slot 0)
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_issue_cmd
asm_ahci_issue_cmd:
    push    rbx
    push    r12
    sub     rsp, 32

    mov     r12, rcx            ; abar
    mov     ebx, edx            ; port_num

    ; Calculate port base
    mov     eax, ebx
    shl     eax, 7
    add     eax, AHCI_PORT_BASE
    add     r12, rax            ; r12 = port base

    ; Write to PxCI to issue command
    lea     rcx, [r12 + AHCI_PxCI]
    mov     edx, r8d
    call    asm_mmio_write32

    call    asm_bar_mfence

    add     rsp, 32
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_poll_cmd
; ═══════════════════════════════════════════════════════════════════════════
; Poll for command completion.
;
; Parameters:
;   RCX = abar
;   EDX = port_num
;   R8D = slot_mask (bit mask of slot to wait for)
;   R9  = tsc_freq (for timeout)
;   [RSP+40] = timeout_ms
;
; Returns:
;   EAX = result:
;         0 = success (command completed without error)
;         1 = timeout
;         2 = error (task file error)
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_poll_cmd
asm_ahci_poll_cmd:
    push    rbx
    push    r12
    push    r13
    push    r14
    push    r15
    sub     rsp, 32

    mov     r12, rcx            ; abar
    mov     r13d, edx           ; port_num
    mov     r14d, r8d           ; slot_mask
    mov     r15, r9             ; tsc_freq

    ; Get timeout_ms from stack
    mov     eax, [rsp + 32 + 40 + 40]  ; After pushes + shadow + return
    
    ; Calculate timeout ticks: (tsc_freq * timeout_ms) / 1000
    imul    rax, r15
    xor     edx, edx
    mov     ecx, 1000
    div     rcx
    mov     r15, rax            ; r15 = timeout ticks

    ; Calculate port base
    mov     eax, r13d
    shl     eax, 7
    add     eax, AHCI_PORT_BASE
    add     r12, rax            ; r12 = port base

    ; Get start time
    call    asm_tsc_read
    mov     rbx, rax            ; rbx = start

.poll_loop:
    ; Check PxCI - command completes when bit clears
    lea     rcx, [r12 + AHCI_PxCI]
    call    asm_mmio_read32
    test    eax, r14d
    jz      .cmd_complete

    ; Check for errors in PxIS
    lea     rcx, [r12 + AHCI_PxIS]
    call    asm_mmio_read32
    test    eax, AHCI_PXIS_TFES     ; Task File Error Status
    jnz     .error

    ; Check timeout
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r15
    jb      .poll_loop

    ; Timeout
    mov     eax, 1
    jmp     .exit

.cmd_complete:
    ; Double-check for errors
    lea     rcx, [r12 + AHCI_PxIS]
    call    asm_mmio_read32
    test    eax, AHCI_PXIS_TFES
    jnz     .error

    ; Success
    xor     eax, eax
    jmp     .exit

.error:
    mov     eax, 2

.exit:
    add     rsp, 32
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_check_cmd_complete
; ═══════════════════════════════════════════════════════════════════════════
; Non-blocking check if a command slot has completed.
;
; Parameters:
;   RCX = abar
;   EDX = port_num
;   R8D = slot_mask
;
; Returns:
;   EAX = 0 if still pending, 1 if complete, 2 if error
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_check_cmd_complete
asm_ahci_check_cmd_complete:
    push    rbx
    push    r12
    sub     rsp, 32

    mov     r12, rcx            ; abar
    mov     ebx, edx            ; port_num

    ; Calculate port base
    mov     eax, ebx
    shl     eax, 7
    add     eax, AHCI_PORT_BASE
    add     r12, rax

    ; Save slot mask
    mov     ebx, r8d

    ; Check PxIS for errors first
    lea     rcx, [r12 + AHCI_PxIS]
    call    asm_mmio_read32
    test    eax, AHCI_PXIS_TFES
    jnz     .error

    ; Check PxCI
    lea     rcx, [r12 + AHCI_PxCI]
    call    asm_mmio_read32
    test    eax, ebx
    jnz     .pending

    ; Complete
    mov     eax, 1
    jmp     .exit

.pending:
    xor     eax, eax
    jmp     .exit

.error:
    mov     eax, 2

.exit:
    add     rsp, 32
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_read_prdbc
; ═══════════════════════════════════════════════════════════════════════════
; Read Physical Region Descriptor Byte Count after command completion.
;
; Parameters:
;   RCX = cmd_header_ptr (CPU pointer to command header)
;
; Returns:
;   EAX = PRDBC (bytes transferred)
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_read_prdbc
asm_ahci_read_prdbc:
    ; PRDBC is at offset 4 in command header
    mov     eax, [rcx + 4]
    ret
