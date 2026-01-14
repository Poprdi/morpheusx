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
    push    r15
    sub     rsp, 40

    mov     r12, rcx            ; r12 = mmio_base
    mov     r13d, edx           ; r13 = phy_reg
    mov     r14, r8             ; r14 = tsc_freq

    ; ═══════════════════════════════════════════════════════════════════
    ; Pre-access delay: 10us minimum between MDIO operations
    ; Real hardware MDIO bus needs time between accesses.
    ; QEMU doesn't need this, but real I218-LM does.
    ; ═══════════════════════════════════════════════════════════════════
    call    asm_tsc_read
    mov     r15, rax            ; r15 = delay_start
    mov     rax, r14
    xor     rdx, rdx
    mov     rcx, 100000         ; 10us = tsc_freq / 100000
    div     rcx
    mov     rcx, rax            ; rcx = delay_ticks (10us)
.pre_delay:
    pause
    call    asm_tsc_read
    sub     rax, r15
    cmp     rax, rcx
    jb      .pre_delay

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
    
    ; Memory barrier after MMIO write
    mfence

    ; Get start TSC for timeout
    call    asm_tsc_read
    mov     rbx, rax            ; rbx = start_tsc

    ; Timeout = tsc_freq / 100 = 10ms (was 1ms - too short for real I218)
    ; Real hardware needs more time, especially after power-on or ULP exit.
    mov     rax, r14
    xor     rdx, rdx
    mov     rcx, 100
    div     rcx
    mov     r14, rax            ; r14 = timeout_ticks

.wait_ready:
    ; Small delay between polls (helps real hardware)
    pause
    pause
    pause
    pause

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
    pop     r15
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
    push    rsi
    sub     rsp, 48

    mov     r12, rcx            ; r12 = mmio_base
    mov     r13d, edx           ; r13 = phy_reg
    mov     r14d, r8d           ; r14 = value
    mov     r15, r9             ; r15 = tsc_freq

    ; ═══════════════════════════════════════════════════════════════════
    ; Pre-access delay: 10us minimum between MDIO operations
    ; Real hardware MDIO bus needs time between accesses.
    ; ═══════════════════════════════════════════════════════════════════
    call    asm_tsc_read
    mov     rsi, rax            ; rsi = delay_start
    mov     rax, r15
    xor     rdx, rdx
    mov     rcx, 100000         ; 10us = tsc_freq / 100000
    div     rcx
    mov     rcx, rax            ; rcx = delay_ticks (10us)
.pre_delay:
    pause
    call    asm_tsc_read
    sub     rax, rsi
    cmp     rax, rcx
    jb      .pre_delay

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
    
    ; Memory barrier after MMIO write
    mfence

    ; Get start TSC for timeout
    call    asm_tsc_read
    mov     rbx, rax            ; rbx = start_tsc

    ; Timeout = tsc_freq / 100 = 10ms (was 1ms - too short for real I218)
    ; Real hardware needs more time, especially after power-on or ULP exit.
    mov     rax, r15
    xor     rdx, rdx
    mov     rcx, 100
    div     rcx
    mov     r15, rax            ; r15 = timeout_ticks

.wait_ready_w:
    ; Small delay between polls (helps real hardware)
    pause
    pause
    pause
    pause

    ; Read MDIC
    mov     rcx, r12
    add     rcx, MDIC
    call    asm_mmio_read32

    ; Check if ready
    test    eax, MDIC_READY
    jnz     .check_error_w

    ; Check timeout
    push    rax
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r15
    pop     rax
    jb      .wait_ready_w

    ; Timeout
    mov     eax, 1
    jmp     .exit_w

.check_error_w:
    ; Check for error
    test    eax, MDIC_ERROR
    jnz     .error_w

    xor     eax, eax            ; Success
    jmp     .exit_w

.error_w:
    mov     eax, 1

.exit_w:
    add     rsp, 48
    pop     rsi
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
; Checks BOTH:
;   1. MAC STATUS.LU (bit 1) - fast path
;   2. PHY BMSR.LSTATUS (bit 2) - authoritative source
;
; On real hardware, MAC STATUS.LU may not reflect PHY state during init.
; We check PHY BMSR as authoritative source if MAC doesn't show link.
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
    push    r15
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
    xor     r15d, r15d          ; r15 = loop counter for PHY check interval

.poll_loop:
    ; Small delay between polls
    pause
    pause

    ; ═══════════════════════════════════════════════════════════════════
    ; Check 1: MAC STATUS register (fast path)
    ; ═══════════════════════════════════════════════════════════════════
    mov     rcx, r12
    add     rcx, STATUS
    call    asm_mmio_read32

    ; Check link up (bit 1)
    test    eax, STATUS_LU
    jnz     .link_up

    ; ═══════════════════════════════════════════════════════════════════
    ; Check 2: PHY BMSR register (authoritative, every 100 iterations)
    ; Reading BMSR is slower (MDIO bus), so we don't do it every loop
    ; ═══════════════════════════════════════════════════════════════════
    inc     r15d
    cmp     r15d, 100
    jb      .skip_phy_check
    xor     r15d, r15d          ; Reset counter

    ; Read PHY BMSR (register 1) via MDIC
    ; BMSR.LSTATUS is bit 2
    mov     rcx, r12
    mov     edx, 1              ; BMSR = register 1
    mov     r8, r14             ; tsc_freq
    call    asm_intel_phy_read

    ; Check for read error
    cmp     eax, 0xFFFFFFFF
    je      .skip_phy_check

    ; Check BMSR.LSTATUS (bit 2)
    test    eax, 0x0004         ; BMSR_LSTATUS = 1 << 2
    jnz     .link_up

.skip_phy_check:
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
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret
