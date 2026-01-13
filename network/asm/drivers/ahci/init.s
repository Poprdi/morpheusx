; ═══════════════════════════════════════════════════════════════════════════
; AHCI Host Bus Adapter Initialization
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Intel Wildcat Point-LP SATA Controller [AHCI Mode]
; Target: ThinkPad T450s (0x8086:0x9C83)
;
; Functions:
;   - asm_ahci_hba_reset: Reset AHCI controller
;   - asm_ahci_enable: Enable AHCI mode
;   - asm_ahci_read_cap: Read HBA capabilities
;   - asm_ahci_read_pi: Read ports implemented bitmap
;   - asm_ahci_read_version: Read AHCI version
;   - asm_ahci_disable_interrupts: Disable global interrupts
;
; Reference: AHCI 1.3.1 Specification
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
; asm_ahci_hba_reset
; ═══════════════════════════════════════════════════════════════════════════
; Reset the AHCI Host Bus Adapter.
;
; Parameters:
;   RCX = abar (AHCI Base Address - BAR5 mapped)
;   RDX = tsc_freq (for timeout calculation)
;
; Returns:
;   EAX = 0 on success, 1 on timeout
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_hba_reset
asm_ahci_hba_reset:
    push    rbx
    push    r12
    push    r13
    push    r14
    sub     rsp, 32             ; Shadow space

    mov     r12, rcx            ; r12 = abar
    mov     r13, rdx            ; r13 = tsc_freq

    ; First, ensure AHCI is enabled before reset
    ; Read GHC
    lea     rcx, [r12 + AHCI_HBA_GHC]
    call    asm_mmio_read32
    mov     r14d, eax           ; r14 = current GHC

    ; Set AE (AHCI Enable) bit if not set
    test    r14d, AHCI_GHC_AE
    jnz     .do_reset
    
    or      r14d, AHCI_GHC_AE
    lea     rcx, [r12 + AHCI_HBA_GHC]
    mov     edx, r14d
    call    asm_mmio_write32
    call    asm_bar_mfence

.do_reset:
    ; Set HR (HBA Reset) bit
    lea     rcx, [r12 + AHCI_HBA_GHC]
    call    asm_mmio_read32
    or      eax, AHCI_GHC_HR
    lea     rcx, [r12 + AHCI_HBA_GHC]
    mov     edx, eax
    call    asm_mmio_write32

    call    asm_bar_mfence

    ; Get start TSC for timeout
    call    asm_tsc_read
    mov     rbx, rax            ; rbx = start_tsc

    ; Calculate timeout: tsc_freq / 2 = 500ms (AHCI spec allows up to 1 second)
    mov     rax, r13
    shr     rax, 1
    mov     r14, rax            ; r14 = timeout_ticks

.wait_reset:
    ; Read GHC and check if HR cleared
    lea     rcx, [r12 + AHCI_HBA_GHC]
    call    asm_mmio_read32
    test    eax, AHCI_GHC_HR
    jz      .reset_done

    ; Check timeout
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r14
    jb      .wait_reset

    ; Timeout
    mov     eax, 1
    jmp     .exit

.reset_done:
    ; Re-enable AHCI after reset (reset clears AE in some controllers)
    lea     rcx, [r12 + AHCI_HBA_GHC]
    call    asm_mmio_read32
    or      eax, AHCI_GHC_AE
    lea     rcx, [r12 + AHCI_HBA_GHC]
    mov     edx, eax
    call    asm_mmio_write32

    call    asm_bar_mfence

    ; Success
    xor     eax, eax

.exit:
    add     rsp, 32
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_enable
; ═══════════════════════════════════════════════════════════════════════════
; Enable AHCI mode on the controller.
;
; Parameters:
;   RCX = abar
;
; Returns:
;   EAX = 0 on success
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_enable
asm_ahci_enable:
    push    r12
    sub     rsp, 32

    mov     r12, rcx            ; r12 = abar

    ; Read current GHC
    lea     rcx, [r12 + AHCI_HBA_GHC]
    call    asm_mmio_read32

    ; Set AHCI Enable bit
    or      eax, AHCI_GHC_AE
    lea     rcx, [r12 + AHCI_HBA_GHC]
    mov     edx, eax
    call    asm_mmio_write32

    call    asm_bar_mfence

    xor     eax, eax

    add     rsp, 32
    pop     r12
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_read_cap
; ═══════════════════════════════════════════════════════════════════════════
; Read HBA Capabilities register.
;
; Parameters:
;   RCX = abar
;
; Returns:
;   EAX = CAP register value
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_read_cap
asm_ahci_read_cap:
    sub     rsp, 32
    
    ; CAP is at offset 0
    ; rcx already has abar
    call    asm_mmio_read32
    
    add     rsp, 32
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_read_pi
; ═══════════════════════════════════════════════════════════════════════════
; Read Ports Implemented bitmap.
;
; Parameters:
;   RCX = abar
;
; Returns:
;   EAX = PI register (bitmap of implemented ports)
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_read_pi
asm_ahci_read_pi:
    sub     rsp, 32

    lea     rcx, [rcx + AHCI_HBA_PI]
    call    asm_mmio_read32

    add     rsp, 32
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_read_version
; ═══════════════════════════════════════════════════════════════════════════
; Read AHCI version.
;
; Parameters:
;   RCX = abar
;
; Returns:
;   EAX = VS register (major.minor as 0xMMmm0000)
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_read_version
asm_ahci_read_version:
    sub     rsp, 32

    lea     rcx, [rcx + AHCI_HBA_VS]
    call    asm_mmio_read32

    add     rsp, 32
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_disable_interrupts
; ═══════════════════════════════════════════════════════════════════════════
; Disable global AHCI interrupts (we use polling).
;
; Parameters:
;   RCX = abar
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_disable_interrupts
asm_ahci_disable_interrupts:
    push    r12
    sub     rsp, 32

    mov     r12, rcx

    ; Clear GHC.IE (Interrupt Enable)
    lea     rcx, [r12 + AHCI_HBA_GHC]
    call    asm_mmio_read32
    and     eax, ~AHCI_GHC_IE
    lea     rcx, [r12 + AHCI_HBA_GHC]
    mov     edx, eax
    call    asm_mmio_write32

    ; Clear global interrupt status (write to clear)
    lea     rcx, [r12 + AHCI_HBA_IS]
    mov     edx, 0xFFFFFFFF
    call    asm_mmio_write32

    call    asm_bar_mfence

    add     rsp, 32
    pop     r12
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_get_num_ports
; ═══════════════════════════════════════════════════════════════════════════
; Get number of ports supported by HBA.
;
; Parameters:
;   RCX = abar
;
; Returns:
;   EAX = number of ports (1-32)
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_get_num_ports
asm_ahci_get_num_ports:
    sub     rsp, 32

    ; Read CAP register
    call    asm_mmio_read32
    
    ; Extract NP field (bits 4:0) and add 1
    and     eax, AHCI_CAP_NP_MASK
    inc     eax

    add     rsp, 32
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_get_num_cmd_slots
; ═══════════════════════════════════════════════════════════════════════════
; Get number of command slots per port.
;
; Parameters:
;   RCX = abar
;
; Returns:
;   EAX = number of command slots (1-32)
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_get_num_cmd_slots
asm_ahci_get_num_cmd_slots:
    sub     rsp, 32

    ; Read CAP register
    call    asm_mmio_read32
    
    ; Extract NCS field (bits 12:8) and add 1
    shr     eax, AHCI_CAP_NCS_SHIFT
    and     eax, 0x1F
    inc     eax

    add     rsp, 32
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_supports_64bit
; ═══════════════════════════════════════════════════════════════════════════
; Check if HBA supports 64-bit addressing.
;
; Parameters:
;   RCX = abar
;
; Returns:
;   EAX = 1 if 64-bit supported, 0 otherwise
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_supports_64bit
asm_ahci_supports_64bit:
    sub     rsp, 32

    call    asm_mmio_read32
    
    ; Check S64A bit
    test    eax, AHCI_CAP_S64A
    setnz   al
    movzx   eax, al

    add     rsp, 32
    ret
