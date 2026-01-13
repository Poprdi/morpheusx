; ═══════════════════════════════════════════════════════════════════════════
; AHCI IDENTIFY DEVICE Command
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Send IDENTIFY DEVICE command to get drive info (capacity, model, etc.)
;
; Functions:
;   - asm_ahci_identify_device: Execute IDENTIFY DEVICE command
;   - asm_ahci_parse_identify: Extract fields from IDENTIFY data
;
; IDENTIFY DEVICE returns 512 bytes (256 words) of device information:
;   Word 0: General configuration
;   Words 10-19: Serial number (20 ASCII chars)
;   Words 23-26: Firmware revision (8 ASCII chars)
;   Words 27-46: Model number (40 ASCII chars)
;   Word 49: Capabilities
;   Word 60-61: Total sectors (28-bit LBA)
;   Word 83: Command set support
;   Word 86: Command set enabled
;   Words 100-103: Total sectors (48-bit LBA)
;
; Reference: ATA/ATAPI-8 §7.12
; ═══════════════════════════════════════════════════════════════════════════

section .data
    %include "asm/drivers/ahci/regs.s"

section .text

; External: Core and AHCI primitives
extern asm_tsc_read
extern asm_bar_sfence
extern asm_bar_lfence
extern asm_bar_mfence
extern asm_mmio_read32
extern asm_mmio_write32
extern asm_ahci_build_h2d_fis
extern asm_ahci_build_prdt
extern asm_ahci_setup_cmd_header
extern asm_ahci_issue_cmd
extern asm_ahci_poll_cmd

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_identify_device
; ═══════════════════════════════════════════════════════════════════════════
; Execute IDENTIFY DEVICE command to retrieve drive information.
;
; Parameters:
;   RCX = abar
;   EDX = port_num
;   R8  = identify_buf_phys (physical address for 512-byte IDENTIFY data)
;   R9  = cmd_header_ptr (CPU pointer to command header for slot 0)
;   [RSP+40] = cmd_table_ptr (CPU pointer to command table)
;   [RSP+48] = cmd_table_phys (physical address of command table)
;   [RSP+56] = tsc_freq
;
; Returns:
;   EAX = 0 on success, non-zero on error
;
; Note: Uses command slot 0. Port must be started.
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_identify_device
asm_ahci_identify_device:
    push    rbx
    push    rsi
    push    rdi
    push    r12
    push    r13
    push    r14
    push    r15
    sub     rsp, 64             ; Extra space for local storage

    ; Save parameters
    mov     r12, rcx            ; abar
    mov     r13d, edx           ; port_num
    mov     r14, r8             ; identify_buf_phys
    mov     r15, r9             ; cmd_header_ptr

    ; Get stack parameters (accounting for pushes)
    ; 7 pushes × 8 = 56, + sub rsp 64 = 120, + return addr = 128
    ; Original RSP+40 -> now RSP+128+40 = RSP+168
    mov     rsi, [rsp + 168]    ; cmd_table_ptr
    mov     rdi, [rsp + 176]    ; cmd_table_phys
    mov     rbx, [rsp + 184]    ; tsc_freq

    ; ───────────────────────────────────────────────────────────────────
    ; Build H2D FIS for IDENTIFY DEVICE (0xEC)
    ; FIS is at start of command table
    ; ───────────────────────────────────────────────────────────────────
    mov     rcx, rsi            ; fis_ptr = cmd_table
    mov     dl, ATA_CMD_IDENTIFY ; command = 0xEC
    xor     r8, r8              ; LBA = 0 (not used for IDENTIFY)
    mov     r9w, 1              ; sector_count = 1 (512 bytes)
    call    asm_ahci_build_h2d_fis

    ; ───────────────────────────────────────────────────────────────────
    ; Build PRDT entry (one entry for 512 bytes)
    ; PRDT starts at cmd_table + 0x80
    ; ───────────────────────────────────────────────────────────────────
    lea     rcx, [rsi + 0x80]   ; prdt_ptr
    mov     rdx, r14            ; data_phys = identify_buf_phys
    mov     r8d, 511            ; byte_count - 1 = 512 - 1
    call    asm_ahci_build_prdt

    ; ───────────────────────────────────────────────────────────────────
    ; Setup command header for slot 0
    ; Flags: CFL=5 (5 DWORDs), W=0 (read), PRDTL=1
    ; ───────────────────────────────────────────────────────────────────
    ; DW0 format:
    ;   CFL = 5 (bits 4:0) - H2D FIS is 5 DWORDs
    ;   W = 0 (bit 6) - this is a read from device
    ;   PRDTL = 1 (bits 31:16) - one PRDT entry
    mov     edx, (1 << 16) | 5  ; PRDTL=1, CFL=5
    mov     rcx, r15            ; cmd_header_ptr
    mov     r8, rdi             ; cmd_table_phys
    call    asm_ahci_setup_cmd_header

    ; ───────────────────────────────────────────────────────────────────
    ; Issue the command
    ; ───────────────────────────────────────────────────────────────────
    mov     rcx, r12            ; abar
    mov     edx, r13d           ; port_num
    mov     r8d, 1              ; slot_mask = bit 0
    call    asm_ahci_issue_cmd

    ; ───────────────────────────────────────────────────────────────────
    ; Poll for completion (5 second timeout)
    ; ───────────────────────────────────────────────────────────────────
    mov     rcx, r12            ; abar
    mov     edx, r13d           ; port_num
    mov     r8d, 1              ; slot_mask
    mov     r9, rbx             ; tsc_freq
    
    ; Push timeout_ms
    sub     rsp, 8
    mov     dword [rsp], 5000   ; 5 second timeout
    call    asm_ahci_poll_cmd
    add     rsp, 8

    ; Result in EAX (0=success, 1=timeout, 2=error)

    add     rsp, 64
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rdi
    pop     rsi
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_get_identify_capacity
; ═══════════════════════════════════════════════════════════════════════════
; Extract total sector count from IDENTIFY DEVICE data.
;
; Parameters:
;   RCX = identify_buf_ptr (CPU pointer to 512-byte IDENTIFY data)
;
; Returns:
;   RAX = total sectors (48-bit or 28-bit depending on support)
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_get_identify_capacity
asm_ahci_get_identify_capacity:
    ; Check if 48-bit LBA supported (word 83, bit 10)
    movzx   eax, word [rcx + 83*2]
    test    eax, (1 << 10)
    jz      .use_28bit

    ; 48-bit LBA: words 100-103 contain total sectors
    ; Word 100 = bits 15:0
    ; Word 101 = bits 31:16
    ; Word 102 = bits 47:32
    ; Word 103 = bits 63:48 (usually 0)
    movzx   eax, word [rcx + 100*2]
    movzx   edx, word [rcx + 101*2]
    shl     rdx, 16
    or      rax, rdx
    movzx   edx, word [rcx + 102*2]
    shl     rdx, 32
    or      rax, rdx
    movzx   edx, word [rcx + 103*2]
    shl     rdx, 48
    or      rax, rdx
    ret

.use_28bit:
    ; 28-bit LBA: words 60-61
    movzx   eax, word [rcx + 60*2]
    movzx   edx, word [rcx + 61*2]
    shl     edx, 16
    or      eax, edx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_get_identify_sector_size
; ═══════════════════════════════════════════════════════════════════════════
; Get logical sector size from IDENTIFY data.
;
; Parameters:
;   RCX = identify_buf_ptr
;
; Returns:
;   EAX = logical sector size (usually 512)
;
; Note: Word 106 indicates physical/logical sector size support.
;       Words 117-118 contain logical sector size if bit 12 of word 106 is set.
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_get_identify_sector_size
asm_ahci_get_identify_sector_size:
    ; Check word 106 bit 14 (valid) and bit 12 (logical sector size available)
    movzx   eax, word [rcx + 106*2]
    
    ; If bit 14 not set, data invalid - assume 512
    test    eax, (1 << 14)
    jz      .default_512
    
    ; If bit 12 set, logical sector size is in words 117-118
    test    eax, (1 << 12)
    jz      .default_512
    
    ; Read logical sector size (words per sector × 2)
    movzx   eax, word [rcx + 117*2]
    movzx   edx, word [rcx + 118*2]
    shl     edx, 16
    or      eax, edx
    ; Convert words to bytes
    shl     eax, 1
    
    ; Sanity check - if 0, use 512
    test    eax, eax
    jz      .default_512
    ret

.default_512:
    mov     eax, 512
    ret
