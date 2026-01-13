; ═══════════════════════════════════════════════════════════════════════════
; AHCI Port Management
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Port-level initialization, detection, and control.
;
; Functions:
;   - asm_ahci_port_detect: Check if device attached to port
;   - asm_ahci_port_stop: Stop port command engine
;   - asm_ahci_port_start: Start port command engine
;   - asm_ahci_port_setup: Configure command list and FIS base
;   - asm_ahci_port_clear_errors: Clear port error state
;   - asm_ahci_port_read_sig: Read device signature
;   - asm_ahci_port_get_status: Read port status registers
;
; Reference: AHCI 1.3.1 Specification §3.3
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
; Helper: Calculate port register base
; ═══════════════════════════════════════════════════════════════════════════
; Input:  RAX = abar, RBX = port_num
; Output: RAX = port base address
; ═══════════════════════════════════════════════════════════════════════════
%macro CALC_PORT_BASE 0
    ; Port base = abar + 0x100 + (port * 0x80)
    push    rcx
    mov     ecx, ebx
    shl     ecx, 7              ; port * 0x80
    add     ecx, AHCI_PORT_BASE ; + 0x100
    add     rax, rcx
    pop     rcx
%endmacro

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_port_detect
; ═══════════════════════════════════════════════════════════════════════════
; Check if a device is connected to the specified port.
;
; Parameters:
;   RCX = abar
;   EDX = port_num (0-31)
;
; Returns:
;   EAX = detection status:
;         0 = no device
;         1 = device present, not ready
;         3 = device present and ready (PHY established)
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_port_detect
asm_ahci_port_detect:
    push    rbx
    sub     rsp, 32

    mov     rax, rcx
    mov     ebx, edx

    ; Calculate port register base
    CALC_PORT_BASE

    ; Read PxSSTS (SATA Status)
    add     rax, AHCI_PxSSTS
    mov     rcx, rax
    call    asm_mmio_read32

    ; Extract DET field (bits 3:0)
    and     eax, AHCI_SSTS_DET_MASK

    add     rsp, 32
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_port_stop
; ═══════════════════════════════════════════════════════════════════════════
; Stop the port's command engine (must do before setting up CLB/FB).
;
; Parameters:
;   RCX = abar
;   EDX = port_num
;   R8  = tsc_freq (for timeout)
;
; Returns:
;   EAX = 0 on success, 1 on timeout
;
; Procedure:
;   1. Clear PxCMD.ST (Start)
;   2. Wait for PxCMD.CR (Command List Running) to clear
;   3. Clear PxCMD.FRE (FIS Receive Enable)
;   4. Wait for PxCMD.FR (FIS Receive Running) to clear
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_port_stop
asm_ahci_port_stop:
    push    rbx
    push    r12
    push    r13
    push    r14
    push    r15
    sub     rsp, 32

    mov     r12, rcx            ; abar
    mov     r13d, edx           ; port_num
    mov     r14, r8             ; tsc_freq

    ; Calculate port base
    mov     rax, r12
    mov     ebx, r13d
    CALC_PORT_BASE
    mov     r15, rax            ; r15 = port base

    ; ───────────────────────────────────────────────────────────────────
    ; Step 1: Clear ST (Start)
    ; ───────────────────────────────────────────────────────────────────
    lea     rcx, [r15 + AHCI_PxCMD]
    call    asm_mmio_read32
    and     eax, ~AHCI_PXCMD_ST
    lea     rcx, [r15 + AHCI_PxCMD]
    mov     edx, eax
    call    asm_mmio_write32
    call    asm_bar_mfence

    ; ───────────────────────────────────────────────────────────────────
    ; Step 2: Wait for CR to clear (500ms timeout)
    ; ───────────────────────────────────────────────────────────────────
    call    asm_tsc_read
    mov     rbx, rax            ; start time

    mov     rax, r14
    shr     rax, 1              ; 500ms
    mov     r13, rax            ; timeout

.wait_cr:
    lea     rcx, [r15 + AHCI_PxCMD]
    call    asm_mmio_read32
    test    eax, AHCI_PXCMD_CR
    jz      .cr_cleared

    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r13
    jb      .wait_cr

    ; Timeout waiting for CR
    mov     eax, 1
    jmp     .exit

.cr_cleared:
    ; ───────────────────────────────────────────────────────────────────
    ; Step 3: Clear FRE (FIS Receive Enable)
    ; ───────────────────────────────────────────────────────────────────
    lea     rcx, [r15 + AHCI_PxCMD]
    call    asm_mmio_read32
    and     eax, ~AHCI_PXCMD_FRE
    lea     rcx, [r15 + AHCI_PxCMD]
    mov     edx, eax
    call    asm_mmio_write32
    call    asm_bar_mfence

    ; ───────────────────────────────────────────────────────────────────
    ; Step 4: Wait for FR to clear
    ; ───────────────────────────────────────────────────────────────────
    call    asm_tsc_read
    mov     rbx, rax

.wait_fr:
    lea     rcx, [r15 + AHCI_PxCMD]
    call    asm_mmio_read32
    test    eax, AHCI_PXCMD_FR
    jz      .stop_done

    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r13
    jb      .wait_fr

    ; Timeout waiting for FR
    mov     eax, 1
    jmp     .exit

.stop_done:
    xor     eax, eax

.exit:
    add     rsp, 32
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_port_start
; ═══════════════════════════════════════════════════════════════════════════
; Start the port's command engine.
;
; Parameters:
;   RCX = abar
;   EDX = port_num
;
; Returns:
;   EAX = 0
;
; Procedure:
;   1. Set PxCMD.FRE (FIS Receive Enable)
;   2. Set PxCMD.ST (Start)
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_port_start
asm_ahci_port_start:
    push    rbx
    push    r12
    sub     rsp, 32

    mov     r12, rcx            ; abar
    mov     ebx, edx            ; port_num

    ; Calculate port base
    mov     rax, r12
    CALC_PORT_BASE
    mov     r12, rax            ; r12 = port base

    ; ═══════════════════════════════════════════════════════════════════
    ; CRITICAL: Disable Link Power Management BEFORE starting port
    ; Post-EBS we have no ACPI/SMM to handle wake-ups from Slumber
    ; ═══════════════════════════════════════════════════════════════════

    ; 1. Disable DIPM via PxSCTL (SATA Control)
    ;    Set IPM bits [11:8] = 3 to disallow Partial & Slumber
    lea     rcx, [r12 + AHCI_PxSCTL]
    call    asm_mmio_read32
    and     eax, ~0xF00         ; Clear IPM field (bits 11:8)
    or      eax, 0x300          ; Set IPM=3 (disable Partial + Slumber)
    lea     rcx, [r12 + AHCI_PxSCTL]
    mov     edx, eax
    call    asm_mmio_write32
    call    asm_bar_mfence

    ; 2. Disable HIPM and aggressive PM via PxCMD
    ;    Clear ALPE, ASP, APSTE and set ICC to Active
    lea     rcx, [r12 + AHCI_PxCMD]
    call    asm_mmio_read32
    ; Clear: ALPE (26), ASP (27), APSTE (23), ICC_MASK (31:28)
    and     eax, ~(AHCI_PXCMD_ALPE | AHCI_PXCMD_ASP | AHCI_PXCMD_APSTE | AHCI_PXCMD_ICC_MASK)
    ; Set ICC = Active (1)
    or      eax, (AHCI_ICC_ACTIVE << AHCI_PXCMD_ICC_SHIFT)
    lea     rcx, [r12 + AHCI_PxCMD]
    mov     edx, eax
    call    asm_mmio_write32
    call    asm_bar_mfence

    ; ═══════════════════════════════════════════════════════════════════
    ; Now start the port normally
    ; ═══════════════════════════════════════════════════════════════════

    ; Set FRE first
    lea     rcx, [r12 + AHCI_PxCMD]
    call    asm_mmio_read32
    or      eax, AHCI_PXCMD_FRE
    lea     rcx, [r12 + AHCI_PxCMD]
    mov     edx, eax
    call    asm_mmio_write32
    call    asm_bar_mfence

    ; Then set ST
    lea     rcx, [r12 + AHCI_PxCMD]
    call    asm_mmio_read32
    or      eax, AHCI_PXCMD_ST
    lea     rcx, [r12 + AHCI_PxCMD]
    mov     edx, eax
    call    asm_mmio_write32
    call    asm_bar_mfence

    xor     eax, eax

    add     rsp, 32
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_port_setup
; ═══════════════════════════════════════════════════════════════════════════
; Configure command list base and FIS receive buffer for a port.
;
; Parameters:
;   RCX = abar
;   EDX = port_num
;   R8  = clb_phys (Command List Base physical address, 1K aligned)
;   R9  = fb_phys (FIS Base physical address, 256-byte aligned)
;
; Returns:
;   EAX = 0
;
; Note: Port must be stopped (ST=0, CR=0) before calling this!
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_port_setup
asm_ahci_port_setup:
    push    rbx
    push    r12
    push    r13
    push    r14
    sub     rsp, 32

    mov     r12, rcx            ; abar
    mov     ebx, edx            ; port_num
    mov     r13, r8             ; clb_phys
    mov     r14, r9             ; fb_phys

    ; Calculate port base
    mov     rax, r12
    CALC_PORT_BASE
    mov     r12, rax            ; r12 = port base

    ; ───────────────────────────────────────────────────────────────────
    ; Set Command List Base (CLB/CLBU)
    ; ───────────────────────────────────────────────────────────────────
    ; Low 32 bits
    lea     rcx, [r12 + AHCI_PxCLB]
    mov     edx, r13d           ; Low 32 bits of clb_phys
    call    asm_mmio_write32

    ; High 32 bits
    lea     rcx, [r12 + AHCI_PxCLBU]
    mov     rax, r13
    shr     rax, 32
    mov     edx, eax
    call    asm_mmio_write32

    ; ───────────────────────────────────────────────────────────────────
    ; Set FIS Base (FB/FBU)
    ; ───────────────────────────────────────────────────────────────────
    ; Low 32 bits
    lea     rcx, [r12 + AHCI_PxFB]
    mov     edx, r14d
    call    asm_mmio_write32

    ; High 32 bits
    lea     rcx, [r12 + AHCI_PxFBU]
    mov     rax, r14
    shr     rax, 32
    mov     edx, eax
    call    asm_mmio_write32

    call    asm_bar_mfence

    xor     eax, eax

    add     rsp, 32
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_port_clear_errors
; ═══════════════════════════════════════════════════════════════════════════
; Clear all error bits for a port.
;
; Parameters:
;   RCX = abar
;   EDX = port_num
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_port_clear_errors
asm_ahci_port_clear_errors:
    push    rbx
    push    r12
    sub     rsp, 32

    mov     r12, rcx
    mov     ebx, edx

    ; Calculate port base
    mov     rax, r12
    CALC_PORT_BASE
    mov     r12, rax

    ; Clear PxSERR (write 0xFFFFFFFF to clear all)
    lea     rcx, [r12 + AHCI_PxSERR]
    mov     edx, 0xFFFFFFFF
    call    asm_mmio_write32

    ; Clear PxIS (interrupt status, write to clear)
    lea     rcx, [r12 + AHCI_PxIS]
    mov     edx, 0xFFFFFFFF
    call    asm_mmio_write32

    call    asm_bar_mfence

    add     rsp, 32
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_port_read_sig
; ═══════════════════════════════════════════════════════════════════════════
; Read device signature from port.
;
; Parameters:
;   RCX = abar
;   EDX = port_num
;
; Returns:
;   EAX = signature (0x00000101 for ATA, 0xEB140101 for ATAPI, etc.)
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_port_read_sig
asm_ahci_port_read_sig:
    push    rbx
    sub     rsp, 32

    mov     rax, rcx
    mov     ebx, edx

    CALC_PORT_BASE

    lea     rcx, [rax + AHCI_PxSIG]
    call    asm_mmio_read32

    add     rsp, 32
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_port_read_tfd
; ═══════════════════════════════════════════════════════════════════════════
; Read Task File Data (status/error registers).
;
; Parameters:
;   RCX = abar
;   EDX = port_num
;
; Returns:
;   EAX = TFD (low byte = status, next byte = error)
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_port_read_tfd
asm_ahci_port_read_tfd:
    push    rbx
    sub     rsp, 32

    mov     rax, rcx
    mov     ebx, edx

    CALC_PORT_BASE

    lea     rcx, [rax + AHCI_PxTFD]
    call    asm_mmio_read32

    add     rsp, 32
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_port_read_ssts
; ═══════════════════════════════════════════════════════════════════════════
; Read SATA Status (link status, speed, power state).
;
; Parameters:
;   RCX = abar
;   EDX = port_num
;
; Returns:
;   EAX = PxSSTS value
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_port_read_ssts
asm_ahci_port_read_ssts:
    push    rbx
    sub     rsp, 32

    mov     rax, rcx
    mov     ebx, edx

    CALC_PORT_BASE

    lea     rcx, [rax + AHCI_PxSSTS]
    call    asm_mmio_read32

    add     rsp, 32
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_port_read_is
; ═══════════════════════════════════════════════════════════════════════════
; Read port interrupt status.
;
; Parameters:
;   RCX = abar
;   EDX = port_num
;
; Returns:
;   EAX = PxIS value
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_port_read_is
asm_ahci_port_read_is:
    push    rbx
    sub     rsp, 32

    mov     rax, rcx
    mov     ebx, edx

    CALC_PORT_BASE

    lea     rcx, [rax + AHCI_PxIS]
    call    asm_mmio_read32

    add     rsp, 32
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_port_clear_is
; ═══════════════════════════════════════════════════════════════════════════
; Clear specific interrupt status bits.
;
; Parameters:
;   RCX = abar
;   EDX = port_num
;   R8D = bits to clear (write-1-to-clear)
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_port_clear_is
asm_ahci_port_clear_is:
    push    rbx
    push    r12
    sub     rsp, 32

    mov     rax, rcx
    mov     ebx, edx
    mov     r12d, r8d

    CALC_PORT_BASE

    lea     rcx, [rax + AHCI_PxIS]
    mov     edx, r12d
    call    asm_mmio_write32

    add     rsp, 32
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_ahci_port_disable_interrupts
; ═══════════════════════════════════════════════════════════════════════════
; Disable all interrupts for a port (we use polling).
;
; Parameters:
;   RCX = abar
;   EDX = port_num
;
; Calling Convention: Microsoft x64
; ═══════════════════════════════════════════════════════════════════════════
global asm_ahci_port_disable_interrupts
asm_ahci_port_disable_interrupts:
    push    rbx
    sub     rsp, 32

    mov     rax, rcx
    mov     ebx, edx

    CALC_PORT_BASE

    ; Clear PxIE (disable all interrupts)
    lea     rcx, [rax + AHCI_PxIE]
    xor     edx, edx
    call    asm_mmio_write32

    add     rsp, 32
    pop     rbx
    ret
