; ═══════════════════════════════════════════════════════════════════════════
; Intel e1000e PHY Management
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_intel_phy_read: Read PHY register via MDIC
;   - asm_intel_phy_write: Write PHY register via MDIC
;   - asm_intel_link_status: Get fast link status from STATUS register
;   - asm_intel_wait_link: Wait for link up with timeout
;
; MDIC Register Layout (0x0020):
;   Bits 0-15:  Data
;   Bits 16-20: Register Address
;   Bits 21-25: PHY Address (usually 1)
;   Bits 26-27: Op (01=write, 10=read)
;   Bit 28:     Ready
;   Bit 29:     Interrupt Enable
;   Bit 30:     Error
;   Bit 31:     Reserved
;
; PHY Registers (MII Standard):
;   0x00: BMCR (Basic Mode Control)
;   0x01: BMSR (Basic Mode Status)
;   0x02: PHYID1
;   0x03: PHYID2
;   0x04: ANAR (Auto-Neg Advertisement)
;   0x05: ANLPAR (Auto-Neg Link Partner Ability)
;
; Reference: Intel 82579 Datasheet Section 8.2
; ═══════════════════════════════════════════════════════════════════════════

section .data
    ; Register offsets
    STATUS      equ 0x0008
    MDIC        equ 0x0020

    ; MDIC bits
    MDIC_DATA_MASK  equ 0x0000FFFF
    MDIC_REG_SHIFT  equ 16
    MDIC_REG_MASK   equ (0x1F << 16)
    MDIC_PHY_SHIFT  equ 21
    MDIC_PHY_MASK   equ (0x1F << 21)
    MDIC_OP_WRITE   equ (1 << 26)
    MDIC_OP_READ    equ (2 << 26)
    MDIC_READY      equ (1 << 28)
    MDIC_ERROR      equ (1 << 30)

    ; STATUS bits
    STATUS_FD       equ (1 << 0)
    STATUS_LU       equ (1 << 1)
    STATUS_SPEED_MASK equ (3 << 6)
    STATUS_SPEED_10   equ (0 << 6)
    STATUS_SPEED_100  equ (1 << 6)
    STATUS_SPEED_1000 equ (2 << 6)

    ; Default PHY address
    PHY_ADDR        equ 1

section .text

; External: Core primitives
extern asm_tsc_read
extern asm_bar_mfence
extern asm_mmio_read32
extern asm_mmio_write32

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_phy_read
; Read PHY register via MDIC.
;
; Input:  RCX = mmio_base
;         RDX = PHY register address (0-31)
;         R8  = tsc_freq (for timeout)
; Output: RAX = register value (16-bit), or 0xFFFFFFFF on error
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_phy_read
asm_intel_phy_read:
    push    rbx
    push    r12
    push    r13
    push    r14
    sub     rsp, 40

    mov     r12, rcx            ; r12 = mmio_base
    mov     r13d, edx           ; r13 = phy_reg
    mov     r14, r8             ; r14 = tsc_freq

    ; Build MDIC command: OP_READ | PHY_ADDR | REG | 0
    mov     eax, MDIC_OP_READ
    mov     ebx, PHY_ADDR
    shl     ebx, MDIC_PHY_SHIFT
    or      eax, ebx
    mov     ebx, r13d
    shl     ebx, MDIC_REG_SHIFT
    or      eax, ebx

    ; Write to MDIC
    mov     rcx, r12
    add     rcx, MDIC
    mov     edx, eax
    call    asm_mmio_write32

    ; Get start TSC for timeout
    call    asm_tsc_read
    mov     rbx, rax            ; rbx = start_tsc

    ; Timeout = tsc_freq / 1000 = 1ms
    mov     rax, r14
    xor     rdx, rdx
    mov     rcx, 1000
    div     rcx
    mov     r14, rax            ; r14 = timeout_ticks

.wait_ready:
    ; Read MDIC
    mov     rcx, r12
    add     rcx, MDIC
    call    asm_mmio_read32

    ; Check if ready
    test    eax, MDIC_READY
    jnz     .check_error

    ; Check timeout
    push    rax
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r14
    pop     rax
    jb      .wait_ready

    ; Timeout - return error
    mov     eax, 0xFFFFFFFF
    jmp     .exit

.check_error:
    ; Check for error
    test    eax, MDIC_ERROR
    jnz     .error

    ; Extract data (bits 0-15)
    and     eax, MDIC_DATA_MASK
    jmp     .exit

.error:
    mov     eax, 0xFFFFFFFF

.exit:
    add     rsp, 40
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_phy_write
; Write PHY register via MDIC.
;
; Input:  RCX = mmio_base
;         RDX = PHY register address (0-31)
;         R8  = value to write (16-bit)
;         R9  = tsc_freq (for timeout)
; Output: RAX = 0 on success, 1 on error/timeout
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_phy_write
asm_intel_phy_write:
    push    rbx
    push    r12
    push    r13
    push    r14
    push    r15
    sub     rsp, 48

    mov     r12, rcx            ; r12 = mmio_base
    mov     r13d, edx           ; r13 = phy_reg
    mov     r14d, r8d           ; r14 = value
    mov     r15, r9             ; r15 = tsc_freq

    ; Build MDIC command: OP_WRITE | PHY_ADDR | REG | DATA
    mov     eax, MDIC_OP_WRITE
    mov     ebx, PHY_ADDR
    shl     ebx, MDIC_PHY_SHIFT
    or      eax, ebx
    mov     ebx, r13d
    shl     ebx, MDIC_REG_SHIFT
    or      eax, ebx
    or      eax, r14d           ; Add data

    ; Write to MDIC
    mov     rcx, r12
    add     rcx, MDIC
    mov     edx, eax
    call    asm_mmio_write32

    ; Get start TSC for timeout
    call    asm_tsc_read
    mov     rbx, rax            ; rbx = start_tsc

    ; Timeout = tsc_freq / 1000 = 1ms
    mov     rax, r15
    xor     rdx, rdx
    mov     rcx, 1000
    div     rcx
    mov     r15, rax            ; r15 = timeout_ticks

.wait_ready:
    ; Read MDIC
    mov     rcx, r12
    add     rcx, MDIC
    call    asm_mmio_read32

    ; Check if ready
    test    eax, MDIC_READY
    jnz     .check_error

    ; Check timeout
    push    rax
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r15
    pop     rax
    jb      .wait_ready

    ; Timeout
    mov     eax, 1
    jmp     .exit

.check_error:
    ; Check for error
    test    eax, MDIC_ERROR
    jnz     .error

    xor     eax, eax            ; Success
    jmp     .exit

.error:
    mov     eax, 1

.exit:
    add     rsp, 48
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_link_status
; Get link status from STATUS register (fast path).
;
; Input:  RCX = mmio_base
;         RDX = result struct pointer:
;               [0]: link_up (1 byte, 0/1)
;               [1]: full_duplex (1 byte, 0/1)
;               [2]: speed (1 byte, 0=10, 1=100, 2=1000)
; Output: RAX = link_up (0/1)
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_link_status
asm_intel_link_status:
    push    rbx
    push    r12
    sub     rsp, 32

    mov     r12, rdx            ; r12 = result struct

    ; Read STATUS
    add     rcx, STATUS
    call    asm_mmio_read32

    ; Extract link up (bit 1)
    mov     ebx, eax
    shr     ebx, 1
    and     ebx, 1
    mov     [r12], bl           ; link_up

    ; Extract full duplex (bit 0)
    mov     ebx, eax
    and     ebx, 1
    mov     [r12+1], bl         ; full_duplex

    ; Extract speed (bits 6-7)
    mov     ebx, eax
    shr     ebx, 6
    and     ebx, 3
    mov     [r12+2], bl         ; speed

    ; Return link_up
    movzx   eax, byte [r12]

    add     rsp, 32
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_wait_link
; Wait for link up with timeout.
;
; Input:  RCX = mmio_base
;         RDX = timeout_us (microseconds)
;         R8  = tsc_freq
; Output: RAX = 0 if link up, 1 if timeout
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_wait_link
asm_intel_wait_link:
    push    rbx
    push    r12
    push    r13
    push    r14
    sub     rsp, 40

    mov     r12, rcx            ; r12 = mmio_base
    mov     r13, rdx            ; r13 = timeout_us
    mov     r14, r8             ; r14 = tsc_freq

    ; Calculate timeout ticks = timeout_us * tsc_freq / 1000000
    mov     rax, r13
    mul     r14                 ; rdx:rax = timeout_us * tsc_freq
    mov     rcx, 1000000
    div     rcx                 ; rax = timeout_ticks
    mov     r13, rax            ; r13 = timeout_ticks

    ; Get start TSC
    call    asm_tsc_read
    mov     rbx, rax            ; rbx = start_tsc

.poll_loop:
    ; Read STATUS
    mov     rcx, r12
    add     rcx, STATUS
    call    asm_mmio_read32

    ; Check link up (bit 1)
    test    eax, STATUS_LU
    jnz     .link_up

    ; Check timeout
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r13
    jb      .poll_loop

    ; Timeout
    mov     eax, 1
    jmp     .exit

.link_up:
    xor     eax, eax            ; Success

.exit:
    add     rsp, 40
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret
