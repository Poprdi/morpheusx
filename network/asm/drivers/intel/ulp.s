; ═══════════════════════════════════════════════════════════════════════════
; Intel I218/PCH LPT Ultra Low Power (ULP) Management
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; These functions are CRITICAL for real I218 hardware (ThinkPad T450s, etc.)
; The PHY may be in ULP mode after BIOS handoff and must be explicitly woken.
;
; Functions:
;   - asm_intel_disable_ulp: Disable Ultra Low Power mode
;   - asm_intel_toggle_lanphypc: Power cycle PHY via LANPHYPC
;   - asm_intel_phy_is_accessible: Check if PHY responds to MDIC
;   - asm_intel_acquire_swflag: Acquire hardware semaphore
;   - asm_intel_release_swflag: Release hardware semaphore
;
; Reference: Linux kernel drivers/net/ethernet/intel/e1000e/ich8lan.c
;   - e1000_disable_ulp_lpt_lp()
;   - e1000_toggle_lanphypc_pch_lpt()
;   - e1000_phy_is_accessible_pchlan()
;   - e1000_acquire_swflag_ich8lan()
; ═══════════════════════════════════════════════════════════════════════════

section .data
    ; Register offsets
    CTRL            equ 0x0000
    STATUS          equ 0x0008
    CTRL_EXT        equ 0x0018
    MDIC            equ 0x0020
    FEXTNVM3        equ 0x003C
    EXTCNF_CTRL     equ 0x0F00
    PHPM            equ 0x0E14
    H2ME            equ 0x5B50
    FWSM            equ 0x5B54

    ; CTRL bits
    CTRL_LANPHYPC_OVERRIDE  equ (1 << 16)
    CTRL_LANPHYPC_VALUE     equ (1 << 17)

    ; CTRL_EXT bits
    CTRL_EXT_FORCE_SMBUS    equ (1 << 11)
    CTRL_EXT_LPCD           equ (1 << 14)
    CTRL_EXT_PHYPDEN        equ (1 << 20)

    ; EXTCNF_CTRL bits
    EXTCNF_CTRL_SWFLAG      equ (1 << 5)

    ; FWSM bits
    FWSM_FW_VALID           equ (1 << 15)
    FWSM_ULP_CFG_DONE       equ (1 << 18)

    ; H2ME bits
    H2ME_ULP_DISABLE        equ (1 << 1)
    H2ME_START_VME          equ (1 << 0)

    ; FEXTNVM3 bits
    FEXTNVM3_PHY_CFG_COUNTER_MASK   equ (0x3 << 12)
    FEXTNVM3_PHY_CFG_COUNTER_50MS   equ (0x1 << 12)

    ; PHPM bits
    PHPM_SPD_EN             equ (1 << 4)
    PHPM_D0A_LPLU           equ (1 << 1)

    ; MDIC bits
    MDIC_DATA_MASK          equ 0x0000FFFF
    MDIC_REG_SHIFT          equ 16
    MDIC_PHY_SHIFT          equ 21
    MDIC_OP_READ            equ (2 << 26)
    MDIC_READY              equ (1 << 28)
    MDIC_ERROR              equ (1 << 30)

    ; PHY registers
    PHY_ID1                 equ 0x02
    PHY_ID2                 equ 0x03
    PHY_ADDR                equ 1

section .text

; External: Core primitives
extern asm_tsc_read
extern asm_bar_sfence
extern asm_bar_lfence
extern asm_bar_mfence
extern asm_mmio_read32
extern asm_mmio_write32

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_disable_ulp
; Disable Ultra Low Power mode on I218 PHY.
;
; This is CRITICAL for I218-LM/V (ThinkPad T450s, etc.) - the PHY may be
; in ULP mode after BIOS handoff and won't respond to MDIC until ULP is
; disabled.
;
; The Linux kernel does this in e1000_disable_ulp_lpt_lp().
;
; Input:  RCX = mmio_base
;         RDX = tsc_freq (ticks per second)
; Output: RAX = 0 on success, 1 on timeout/error
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_disable_ulp
asm_intel_disable_ulp:
    push    rbx
    push    r12
    push    r13
    push    r14
    push    r15
    sub     rsp, 48

    mov     r12, rcx                ; r12 = mmio_base
    mov     r13, rdx                ; r13 = tsc_freq

    ; ═══════════════════════════════════════════════════════════════════
    ; STEP 1: Check if firmware is present (FWSM.FW_VALID)
    ; ═══════════════════════════════════════════════════════════════════
    mov     rcx, r12
    add     rcx, FWSM
    call    asm_mmio_read32
    mov     r14d, eax               ; r14 = fwsm

    test    r14d, FWSM_FW_VALID
    jz      .no_firmware

    ; ═══════════════════════════════════════════════════════════════════
    ; STEP 2: Request ULP disable via H2ME register
    ; ═══════════════════════════════════════════════════════════════════
    mov     rcx, r12
    add     rcx, H2ME
    call    asm_mmio_read32
    or      eax, H2ME_ULP_DISABLE
    mov     edx, eax
    mov     rcx, r12
    add     rcx, H2ME
    call    asm_mmio_write32

    call    asm_bar_mfence

    ; ═══════════════════════════════════════════════════════════════════
    ; STEP 3: Wait for FWSM.ULP_CFG_DONE (firmware acknowledges)
    ; Timeout: 2.5 seconds (per Linux kernel)
    ; ═══════════════════════════════════════════════════════════════════
    call    asm_tsc_read
    mov     r15, rax                ; r15 = start_tsc

    ; Timeout = tsc_freq * 2.5 = tsc_freq * 5 / 2
    mov     rax, r13
    mov     rcx, 5
    mul     rcx
    shr     rax, 1                  ; rax = tsc_freq * 2.5
    mov     r14, rax                ; r14 = timeout_ticks

.wait_ulp_done:
    mov     rcx, r12
    add     rcx, FWSM
    call    asm_mmio_read32

    test    eax, FWSM_ULP_CFG_DONE
    jnz     .ulp_disabled

    ; Check timeout
    call    asm_tsc_read
    sub     rax, r15
    cmp     rax, r14
    jb      .wait_ulp_done

    ; Timeout - try software disable as fallback
    jmp     .sw_ulp_disable

.no_firmware:
    ; ═══════════════════════════════════════════════════════════════════
    ; STEP 2b: No firmware - do software ULP disable
    ; ═══════════════════════════════════════════════════════════════════
.sw_ulp_disable:
    ; Clear CTRL_EXT.PHYPDEN (PHY Power Down Enable)
    mov     rcx, r12
    add     rcx, CTRL_EXT
    call    asm_mmio_read32
    and     eax, ~CTRL_EXT_PHYPDEN
    mov     edx, eax
    mov     rcx, r12
    add     rcx, CTRL_EXT
    call    asm_mmio_write32

    ; Clear PHPM.SPD_EN and PHPM.D0A_LPLU
    mov     rcx, r12
    add     rcx, PHPM
    call    asm_mmio_read32
    and     eax, ~(PHPM_SPD_EN | PHPM_D0A_LPLU)
    mov     edx, eax
    mov     rcx, r12
    add     rcx, PHPM
    call    asm_mmio_write32

    call    asm_bar_mfence

    ; Small stabilization delay (1ms)
    call    asm_tsc_read
    mov     r15, rax
    mov     rax, r13
    xor     rdx, rdx
    mov     rcx, 1000               ; 1ms = tsc_freq / 1000
    div     rcx
    mov     r14, rax

.sw_delay:
    call    asm_tsc_read
    sub     rax, r15
    cmp     rax, r14
    jb      .sw_delay

.ulp_disabled:
    ; Success
    xor     eax, eax
    jmp     .exit

.exit:
    add     rsp, 48
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_toggle_lanphypc
; Power cycle PHY via LANPHYPC control bits.
;
; This is used when the PHY is completely unresponsive. It toggles the
; LANPHYPC signal to power cycle the PHY.
;
; Linux kernel does this in e1000_toggle_lanphypc_pch_lpt().
;
; Input:  RCX = mmio_base
;         RDX = tsc_freq (ticks per second)
; Output: RAX = 0 on success, 1 on timeout
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_toggle_lanphypc
asm_intel_toggle_lanphypc:
    push    rbx
    push    r12
    push    r13
    push    r14
    push    r15
    sub     rsp, 48

    mov     r12, rcx                ; r12 = mmio_base
    mov     r13, rdx                ; r13 = tsc_freq

    ; ═══════════════════════════════════════════════════════════════════
    ; STEP 1: Set PHY config counter to 50ms in FEXTNVM3
    ; ═══════════════════════════════════════════════════════════════════
    mov     rcx, r12
    add     rcx, FEXTNVM3
    call    asm_mmio_read32
    and     eax, ~FEXTNVM3_PHY_CFG_COUNTER_MASK
    or      eax, FEXTNVM3_PHY_CFG_COUNTER_50MS
    mov     edx, eax
    mov     rcx, r12
    add     rcx, FEXTNVM3
    call    asm_mmio_write32

    ; ═══════════════════════════════════════════════════════════════════
    ; STEP 2: Set LANPHYPC Override, clear LANPHYPC Value (power off PHY)
    ; ═══════════════════════════════════════════════════════════════════
    mov     rcx, r12
    add     rcx, CTRL
    call    asm_mmio_read32
    or      eax, CTRL_LANPHYPC_OVERRIDE
    and     eax, ~CTRL_LANPHYPC_VALUE
    mov     edx, eax
    mov     rcx, r12
    add     rcx, CTRL
    call    asm_mmio_write32

    call    asm_bar_mfence

    ; Wait 10us for PHY to power down
    call    asm_tsc_read
    mov     r15, rax
    mov     rax, r13
    xor     rdx, rdx
    mov     rcx, 100000             ; 10us = tsc_freq / 100000
    div     rcx
    mov     r14, rax

.wait_power_off:
    call    asm_tsc_read
    sub     rax, r15
    cmp     rax, r14
    jb      .wait_power_off

    ; ═══════════════════════════════════════════════════════════════════
    ; STEP 3: Clear CTRL_EXT.LPCD (Link Power Cycle Done) before power on
    ; ═══════════════════════════════════════════════════════════════════
    mov     rcx, r12
    add     rcx, CTRL_EXT
    call    asm_mmio_read32
    and     eax, ~CTRL_EXT_LPCD
    mov     edx, eax
    mov     rcx, r12
    add     rcx, CTRL_EXT
    call    asm_mmio_write32

    ; ═══════════════════════════════════════════════════════════════════
    ; STEP 4: Set LANPHYPC Value to power on PHY
    ; ═══════════════════════════════════════════════════════════════════
    mov     rcx, r12
    add     rcx, CTRL
    call    asm_mmio_read32
    or      eax, CTRL_LANPHYPC_OVERRIDE | CTRL_LANPHYPC_VALUE
    mov     edx, eax
    mov     rcx, r12
    add     rcx, CTRL
    call    asm_mmio_write32

    call    asm_bar_mfence

    ; ═══════════════════════════════════════════════════════════════════
    ; STEP 5: Wait for CTRL_EXT.LPCD (Link Power Cycle Done)
    ; Timeout: 50ms
    ; ═══════════════════════════════════════════════════════════════════
    call    asm_tsc_read
    mov     r15, rax

    ; Timeout = tsc_freq / 20 = 50ms
    mov     rax, r13
    xor     rdx, rdx
    mov     rcx, 20
    div     rcx
    mov     r14, rax

.wait_lpcd:
    mov     rcx, r12
    add     rcx, CTRL_EXT
    call    asm_mmio_read32
    test    eax, CTRL_EXT_LPCD
    jnz     .lpcd_done

    call    asm_tsc_read
    sub     rax, r15
    cmp     rax, r14
    jb      .wait_lpcd

    ; Timeout - continue anyway
    jmp     .lanphypc_cleanup

.lpcd_done:
.lanphypc_cleanup:
    ; ═══════════════════════════════════════════════════════════════════
    ; STEP 6: Clear LANPHYPC Override (let hardware control PHY power)
    ; ═══════════════════════════════════════════════════════════════════
    mov     rcx, r12
    add     rcx, CTRL
    call    asm_mmio_read32
    and     eax, ~CTRL_LANPHYPC_OVERRIDE
    mov     edx, eax
    mov     rcx, r12
    add     rcx, CTRL
    call    asm_mmio_write32

    call    asm_bar_mfence

    ; ═══════════════════════════════════════════════════════════════════
    ; STEP 7: Wait 30ms for PHY to fully stabilize
    ; ═══════════════════════════════════════════════════════════════════
    call    asm_tsc_read
    mov     r15, rax

    ; 30ms = tsc_freq * 30 / 1000 = tsc_freq * 3 / 100
    mov     rax, r13
    mov     rcx, 3
    mul     rcx
    xor     rdx, rdx
    mov     rcx, 100
    div     rcx
    mov     r14, rax

.wait_stabilize:
    call    asm_tsc_read
    sub     rax, r15
    cmp     rax, r14
    jb      .wait_stabilize

    ; Success
    xor     eax, eax

    add     rsp, 48
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_phy_is_accessible
; Check if PHY responds to MDIC reads.
;
; Reads PHY_ID1 and PHY_ID2 to verify PHY is accessible.
; Returns success if either returns a valid (non-0xFFFF) value.
;
; Input:  RCX = mmio_base
;         RDX = tsc_freq
; Output: RAX = 1 if accessible, 0 if not accessible
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_phy_is_accessible
asm_intel_phy_is_accessible:
    push    rbx
    push    r12
    push    r13
    push    r14
    sub     rsp, 40

    mov     r12, rcx                ; r12 = mmio_base
    mov     r13, rdx                ; r13 = tsc_freq

    ; ═══════════════════════════════════════════════════════════════════
    ; Try reading PHY_ID1 (register 2)
    ; ═══════════════════════════════════════════════════════════════════
    ; Build MDIC read command for PHY_ID1
    mov     eax, PHY_ID1
    shl     eax, MDIC_REG_SHIFT
    mov     ecx, PHY_ADDR
    shl     ecx, MDIC_PHY_SHIFT
    or      eax, ecx
    or      eax, MDIC_OP_READ
    mov     r14d, eax               ; r14 = mdic command

    ; Write MDIC
    mov     rcx, r12
    add     rcx, MDIC
    mov     edx, r14d
    call    asm_mmio_write32

    call    asm_bar_mfence

    ; Wait for MDIC_READY with 10ms timeout
    call    asm_tsc_read
    mov     rbx, rax

    ; Timeout = tsc_freq / 100 = 10ms
    mov     rax, r13
    xor     rdx, rdx
    mov     rcx, 100
    div     rcx
    mov     r14, rax

.wait_id1:
    mov     rcx, r12
    add     rcx, MDIC
    call    asm_mmio_read32

    test    eax, MDIC_READY
    jnz     .id1_ready

    push    rax
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r14
    pop     rax
    jb      .wait_id1

    ; Timeout - not accessible
    jmp     .not_accessible

.id1_ready:
    ; Check for error
    test    eax, MDIC_ERROR
    jnz     .not_accessible

    ; Check if data is valid (not 0xFFFF)
    and     eax, MDIC_DATA_MASK
    cmp     ax, 0xFFFF
    je      .not_accessible

    ; PHY is accessible
    mov     eax, 1
    jmp     .exit

.not_accessible:
    xor     eax, eax

.exit:
    add     rsp, 40
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_acquire_swflag
; Acquire hardware semaphore (EXTCNF_CTRL.SWFLAG).
;
; Must be called before PHY or NVM access on ICH8/ICH9/ICH10/PCH.
;
; Input:  RCX = mmio_base
;         RDX = tsc_freq
; Output: RAX = 0 on success, 1 on timeout
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_acquire_swflag
asm_intel_acquire_swflag:
    push    rbx
    push    r12
    push    r13
    push    r14
    sub     rsp, 40

    mov     r12, rcx                ; r12 = mmio_base
    mov     r13, rdx                ; r13 = tsc_freq

    ; Get start time
    call    asm_tsc_read
    mov     rbx, rax

    ; Timeout = 1 second
    mov     r14, r13

.acquire_loop:
    ; Set SWFLAG
    mov     rcx, r12
    add     rcx, EXTCNF_CTRL
    call    asm_mmio_read32
    or      eax, EXTCNF_CTRL_SWFLAG
    mov     edx, eax
    mov     rcx, r12
    add     rcx, EXTCNF_CTRL
    call    asm_mmio_write32

    call    asm_bar_mfence

    ; Read back to check if we got it
    mov     rcx, r12
    add     rcx, EXTCNF_CTRL
    call    asm_mmio_read32

    test    eax, EXTCNF_CTRL_SWFLAG
    jnz     .acquired

    ; Check timeout
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r14
    jb      .acquire_loop

    ; Timeout
    mov     eax, 1
    jmp     .exit

.acquired:
    xor     eax, eax

.exit:
    add     rsp, 40
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_release_swflag
; Release hardware semaphore (EXTCNF_CTRL.SWFLAG).
;
; Input:  RCX = mmio_base
; Output: None
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_release_swflag
asm_intel_release_swflag:
    push    rbx
    sub     rsp, 32

    mov     rbx, rcx                ; rbx = mmio_base

    ; Clear SWFLAG
    mov     rcx, rbx
    add     rcx, EXTCNF_CTRL
    call    asm_mmio_read32
    and     eax, ~EXTCNF_CTRL_SWFLAG
    mov     edx, eax
    mov     rcx, rbx
    add     rcx, EXTCNF_CTRL
    call    asm_mmio_write32

    call    asm_bar_mfence

    add     rsp, 32
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_force_smbus_mode
; Force SMBus mode for PHY access if MDIO is not working.
;
; Some I218 variants require SMBus mode for PHY access. This is used as
; a fallback if MDIC reads fail.
;
; Input:  RCX = mmio_base
; Output: None
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_force_smbus_mode
asm_intel_force_smbus_mode:
    push    rbx
    sub     rsp, 32

    mov     rbx, rcx

    ; Set CTRL_EXT.FORCE_SMBUS
    mov     rcx, rbx
    add     rcx, CTRL_EXT
    call    asm_mmio_read32
    or      eax, CTRL_EXT_FORCE_SMBUS
    mov     edx, eax
    mov     rcx, rbx
    add     rcx, CTRL_EXT
    call    asm_mmio_write32

    call    asm_bar_mfence

    add     rsp, 32
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_clear_smbus_mode
; Clear SMBus mode for PHY access.
;
; Input:  RCX = mmio_base
; Output: None
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_clear_smbus_mode
asm_intel_clear_smbus_mode:
    push    rbx
    sub     rsp, 32

    mov     rbx, rcx

    ; Clear CTRL_EXT.FORCE_SMBUS
    mov     rcx, rbx
    add     rcx, CTRL_EXT
    call    asm_mmio_read32
    and     eax, ~CTRL_EXT_FORCE_SMBUS
    mov     edx, eax
    mov     rcx, rbx
    add     rcx, CTRL_EXT
    call    asm_mmio_write32

    call    asm_bar_mfence

    add     rsp, 32
    pop     rbx
    ret
