; ═══════════════════════════════════════════════════════════════════════════
; Intel e1000e Device Initialization
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_intel_reset: Software reset and wait
;   - asm_intel_read_status: Read STATUS register
;   - asm_intel_read_mac: Read MAC from RAL/RAH or EEPROM
;   - asm_intel_write_mac: Write MAC to RAL/RAH
;   - asm_intel_clear_mta: Clear multicast table
;   - asm_intel_clear_stats: Clear statistics registers
;   - asm_intel_setup_rx_ring: Configure RX ring
;   - asm_intel_setup_tx_ring: Configure TX ring
;   - asm_intel_enable_rx: Enable receiver
;   - asm_intel_enable_tx: Enable transmitter
;   - asm_intel_set_link_up: Force link up
;   - asm_intel_disable_interrupts: Disable all interrupts
;
; Register Offsets (from Intel 82579 datasheet):
;   CTRL   = 0x0000  Device Control
;   STATUS = 0x0008  Device Status
;   EECD   = 0x0010  EEPROM Control
;   EERD   = 0x0014  EEPROM Read
;   IMC    = 0x00D8  Interrupt Mask Clear
;   RCTL   = 0x0100  Receive Control
;   TCTL   = 0x0400  Transmit Control
;   RDBAL  = 0x2800  RX Descriptor Base Low
;   RDBAH  = 0x2804  RX Descriptor Base High
;   RDLEN  = 0x2808  RX Descriptor Length
;   RDH    = 0x2810  RX Descriptor Head
;   RDT    = 0x2818  RX Descriptor Tail
;   RXDCTL = 0x2828  RX Descriptor Control
;   TDBAL  = 0x3800  TX Descriptor Base Low
;   TDBAH  = 0x3804  TX Descriptor Base High
;   TDLEN  = 0x3808  TX Descriptor Length
;   TDH    = 0x3810  TX Descriptor Head
;   TDT    = 0x3818  TX Descriptor Tail
;   TXDCTL = 0x3828  TX Descriptor Control
;   RAL0   = 0x5400  Receive Address Low
;   RAH0   = 0x5404  Receive Address High
;   MTA    = 0x5200  Multicast Table Array (128 dwords)
;
; Reference: Intel 82579 Datasheet, NETWORK_IMPL_GUIDE.md
; ═══════════════════════════════════════════════════════════════════════════

section .data
    ; Register offsets
    CTRL        equ 0x0000
    STATUS      equ 0x0008
    EECD        equ 0x0010
    EERD        equ 0x0014
    ICR         equ 0x00C0
    IMS         equ 0x00D0
    IMC         equ 0x00D8
    RCTL        equ 0x0100
    TCTL        equ 0x0400
    RDBAL       equ 0x2800
    RDBAH       equ 0x2804
    RDLEN       equ 0x2808
    RDH         equ 0x2810
    RDT         equ 0x2818
    RXDCTL      equ 0x2828
    TDBAL       equ 0x3800
    TDBAH       equ 0x3804
    TDLEN       equ 0x3808
    TDH         equ 0x3810
    TDT         equ 0x3818
    TXDCTL      equ 0x3828
    RAL0        equ 0x5400
    RAH0        equ 0x5404
    MTA         equ 0x5200

    ; CTRL bits
    CTRL_FD         equ (1 << 0)    ; Full Duplex
    CTRL_LRST       equ (1 << 3)    ; Link Reset
    CTRL_ASDE       equ (1 << 5)    ; Auto-Speed Detection Enable
    CTRL_SLU        equ (1 << 6)    ; Set Link Up
    CTRL_ILOS       equ (1 << 7)    ; Invert Loss-of-Signal
    CTRL_SPEED_1000 equ (2 << 8)    ; Speed 1000 Mb/s
    CTRL_FRCSPD     equ (1 << 11)   ; Force Speed
    CTRL_FRCDPLX    equ (1 << 12)   ; Force Duplex
    CTRL_RST        equ (1 << 26)   ; Device Reset
    CTRL_PHY_RST    equ (1 << 31)   ; PHY Reset

    ; STATUS bits
    STATUS_FD       equ (1 << 0)    ; Full Duplex
    STATUS_LU       equ (1 << 1)    ; Link Up
    STATUS_SPEED    equ (3 << 6)    ; Speed mask

    ; RCTL bits
    RCTL_EN         equ (1 << 1)    ; Receiver Enable
    RCTL_SBP        equ (1 << 2)    ; Store Bad Packets
    RCTL_UPE        equ (1 << 3)    ; Unicast Promiscuous
    RCTL_MPE        equ (1 << 4)    ; Multicast Promiscuous
    RCTL_LPE        equ (1 << 5)    ; Long Packet Enable
    RCTL_BAM        equ (1 << 15)   ; Broadcast Accept Mode
    RCTL_BSIZE_2048 equ (0 << 16)   ; Buffer size 2048
    RCTL_SECRC      equ (1 << 26)   ; Strip Ethernet CRC

    ; TCTL bits
    TCTL_EN         equ (1 << 1)    ; Transmit Enable
    TCTL_PSP        equ (1 << 3)    ; Pad Short Packets
    TCTL_CT_SHIFT   equ 4           ; Collision Threshold shift
    TCTL_COLD_SHIFT equ 12          ; Collision Distance shift

    ; RAH bits
    RAH_AV          equ (1 << 31)   ; Address Valid

    ; EERD bits
    EERD_START      equ (1 << 0)    ; Start Read
    EERD_DONE       equ (1 << 4)    ; Read Done

    ; Interrupt bits (IMC clears all)
    INT_ALL         equ 0xFFFFFFFF

section .text

; External: Core primitives
extern asm_tsc_read
extern asm_bar_sfence
extern asm_bar_lfence
extern asm_bar_mfence
extern asm_mmio_read32
extern asm_mmio_write32

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_reset
; Reset the device and wait for completion.
;
; Input:  RCX = mmio_base
;         RDX = tsc_freq (ticks per second)
; Output: RAX = 0 on success, 1 on timeout
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_reset
asm_intel_reset:
    push    rbx
    push    r12
    push    r13
    push    r14
    sub     rsp, 40             ; Shadow space + alignment

    mov     r12, rcx            ; r12 = mmio_base
    mov     r13, rdx            ; r13 = tsc_freq

    ; Disable interrupts first
    mov     rcx, r12
    add     rcx, IMC
    mov     edx, INT_ALL
    call    asm_mmio_write32

    ; Read current CTRL
    mov     rcx, r12
    add     rcx, CTRL
    call    asm_mmio_read32
    mov     r14d, eax           ; r14 = current CTRL

    ; Set RST bit
    or      r14d, CTRL_RST
    mov     rcx, r12
    add     rcx, CTRL
    mov     edx, r14d
    call    asm_mmio_write32

    ; Memory barrier after write
    call    asm_bar_mfence

    ; Get start TSC
    call    asm_tsc_read
    mov     rbx, rax            ; rbx = start_tsc

    ; Timeout = tsc_freq / 10 = 100ms
    mov     rax, r13
    xor     rdx, rdx
    mov     rcx, 10
    div     rcx
    mov     r14, rax            ; r14 = timeout ticks

.wait_reset:
    ; Read CTRL
    mov     rcx, r12
    add     rcx, CTRL
    call    asm_mmio_read32

    ; Check if RST bit cleared
    test    eax, CTRL_RST
    jz      .reset_done

    ; Check timeout
    call    asm_tsc_read
    sub     rax, rbx            ; elapsed = now - start
    cmp     rax, r14
    jb      .wait_reset

    ; Timeout
    mov     eax, 1
    jmp     .exit

.reset_done:
    ; Memory barrier
    call    asm_bar_mfence

    ; Small delay (~10ms) for device to stabilize
    mov     rax, r13
    xor     rdx, rdx
    mov     rcx, 100
    div     rcx                 ; rax = tsc_freq / 100 = 10ms worth of ticks
    mov     r14, rax

    call    asm_tsc_read
    mov     rbx, rax            ; rbx = start

.stabilize_delay:
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r14
    jb      .stabilize_delay

    ; Disable interrupts again after reset
    mov     rcx, r12
    add     rcx, IMC
    mov     edx, INT_ALL
    call    asm_mmio_write32

    xor     eax, eax            ; Success

.exit:
    add     rsp, 40
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_read_status
; Read the STATUS register.
;
; Input:  RCX = mmio_base
; Output: RAX = STATUS register value
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_read_status
asm_intel_read_status:
    add     rcx, STATUS
    jmp     asm_mmio_read32

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_read_mac
; Read MAC address from RAL/RAH registers.
;
; Input:  RCX = mmio_base
;         RDX = pointer to 6-byte MAC buffer
; Output: RAX = 0 on success, 1 if MAC invalid (all zeros or all ones)
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_read_mac
asm_intel_read_mac:
    push    rbx
    push    r12
    push    r13
    sub     rsp, 32

    mov     r12, rcx            ; r12 = mmio_base
    mov     r13, rdx            ; r13 = mac_out

    ; Read RAL0
    mov     rcx, r12
    add     rcx, RAL0
    call    asm_mmio_read32
    mov     ebx, eax            ; rbx = RAL (low 4 bytes of MAC)

    ; Read RAH0
    mov     rcx, r12
    add     rcx, RAH0
    call    asm_mmio_read32
                                ; eax = RAH (high 2 bytes of MAC + flags)

    ; Check if Address Valid bit is set
    test    eax, RAH_AV
    jz      .try_eeprom

    ; Store MAC bytes: RAL[0-3] = bytes 0-3, RAH[0-1] = bytes 4-5
    ; Use shifts instead of high-byte registers (incompatible with REX prefix)
    mov     ecx, ebx
    mov     [r13], cl           ; Byte 0
    shr     ecx, 8
    mov     [r13+1], cl         ; Byte 1
    shr     ebx, 16
    mov     ecx, ebx
    mov     [r13+2], cl         ; Byte 2
    shr     ecx, 8
    mov     [r13+3], cl         ; Byte 3
    mov     ecx, eax
    mov     [r13+4], cl         ; Byte 4
    shr     ecx, 8
    mov     [r13+5], cl         ; Byte 5

    ; Validate MAC (not all zeros, not all ones)
    mov     eax, [r13]
    movzx   ecx, word [r13+4]
    or      eax, ecx
    jz      .invalid            ; All zeros

    mov     eax, [r13]
    and     eax, [r13+4]
    cmp     eax, 0xFFFFFFFF
    je      .invalid            ; All ones

    xor     eax, eax            ; Success
    jmp     .exit

.try_eeprom:
    ; TODO: Read from EEPROM if RAL/RAH invalid
    ; For now, return error
    mov     eax, 1
    jmp     .exit

.invalid:
    mov     eax, 1

.exit:
    add     rsp, 32
    pop     r13
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_write_mac
; Write MAC address to RAL/RAH registers.
;
; Input:  RCX = mmio_base
;         RDX = pointer to 6-byte MAC buffer
; Output: None
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_write_mac
asm_intel_write_mac:
    push    rbx
    push    r12
    push    r13
    sub     rsp, 32

    mov     r12, rcx            ; r12 = mmio_base
    mov     r13, rdx            ; r13 = mac

    ; Build RAL value (bytes 0-3)
    movzx   eax, byte [r13]
    movzx   ecx, byte [r13+1]
    shl     ecx, 8
    or      eax, ecx
    movzx   ecx, byte [r13+2]
    shl     ecx, 16
    or      eax, ecx
    movzx   ecx, byte [r13+3]
    shl     ecx, 24
    or      eax, ecx
    mov     ebx, eax            ; ebx = RAL value

    ; Write RAL0
    mov     rcx, r12
    add     rcx, RAL0
    mov     edx, ebx
    call    asm_mmio_write32

    ; Build RAH value (bytes 4-5 + AV bit)
    movzx   eax, byte [r13+4]
    movzx   ecx, byte [r13+5]
    shl     ecx, 8
    or      eax, ecx
    or      eax, RAH_AV         ; Set Address Valid

    ; Write RAH0
    mov     rcx, r12
    add     rcx, RAH0
    mov     edx, eax
    call    asm_mmio_write32

    add     rsp, 32
    pop     r13
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_clear_mta
; Clear multicast table array (128 dwords).
;
; Input:  RCX = mmio_base
; Output: None
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_clear_mta
asm_intel_clear_mta:
    push    rbx
    push    r12
    sub     rsp, 32

    mov     r12, rcx            ; r12 = mmio_base
    xor     ebx, ebx            ; ebx = index

.loop:
    ; Write 0 to MTA[index]
    mov     rcx, r12
    add     rcx, MTA
    mov     eax, ebx
    shl     eax, 2              ; index * 4
    add     rcx, rax
    xor     edx, edx
    call    asm_mmio_write32

    inc     ebx
    cmp     ebx, 128
    jb      .loop

    add     rsp, 32
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_disable_interrupts
; Disable all interrupts.
;
; Input:  RCX = mmio_base
; Output: None
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_disable_interrupts
asm_intel_disable_interrupts:
    add     rcx, IMC
    mov     edx, INT_ALL
    jmp     asm_mmio_write32

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_setup_rx_ring
; Configure RX descriptor ring.
;
; Input:  RCX = mmio_base
;         RDX = ring bus address (64-bit)
;         R8  = ring length in bytes
; Output: None
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_setup_rx_ring
asm_intel_setup_rx_ring:
    push    rbx
    push    r12
    push    r13
    push    r14
    sub     rsp, 40

    mov     r12, rcx            ; r12 = mmio_base
    mov     r13, rdx            ; r13 = ring_bus_addr
    mov     r14d, r8d           ; r14 = ring_len

    ; Disable RX first
    mov     rcx, r12
    add     rcx, RCTL
    call    asm_mmio_read32
    and     eax, ~RCTL_EN
    mov     edx, eax
    mov     rcx, r12
    add     rcx, RCTL
    call    asm_mmio_write32

    ; Write RDBAL (low 32 bits)
    mov     rcx, r12
    add     rcx, RDBAL
    mov     edx, r13d
    call    asm_mmio_write32

    ; Write RDBAH (high 32 bits)
    mov     rcx, r12
    add     rcx, RDBAH
    mov     rax, r13
    shr     rax, 32
    mov     edx, eax
    call    asm_mmio_write32

    ; Write RDLEN
    mov     rcx, r12
    add     rcx, RDLEN
    mov     edx, r14d
    call    asm_mmio_write32

    ; Reset head and tail
    mov     rcx, r12
    add     rcx, RDH
    xor     edx, edx
    call    asm_mmio_write32

    mov     rcx, r12
    add     rcx, RDT
    xor     edx, edx
    call    asm_mmio_write32

    ; Configure RXDCTL (enable prefetch, etc.)
    mov     rcx, r12
    add     rcx, RXDCTL
    mov     edx, (1 << 25)      ; Enable bit
    call    asm_mmio_write32

    add     rsp, 40
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_setup_tx_ring
; Configure TX descriptor ring.
;
; Input:  RCX = mmio_base
;         RDX = ring bus address (64-bit)
;         R8  = ring length in bytes
; Output: None
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_setup_tx_ring
asm_intel_setup_tx_ring:
    push    rbx
    push    r12
    push    r13
    push    r14
    sub     rsp, 40

    mov     r12, rcx            ; r12 = mmio_base
    mov     r13, rdx            ; r13 = ring_bus_addr
    mov     r14d, r8d           ; r14 = ring_len

    ; Disable TX first
    mov     rcx, r12
    add     rcx, TCTL
    call    asm_mmio_read32
    and     eax, ~TCTL_EN
    mov     edx, eax
    mov     rcx, r12
    add     rcx, TCTL
    call    asm_mmio_write32

    ; Write TDBAL (low 32 bits)
    mov     rcx, r12
    add     rcx, TDBAL
    mov     edx, r13d
    call    asm_mmio_write32

    ; Write TDBAH (high 32 bits)
    mov     rcx, r12
    add     rcx, TDBAH
    mov     rax, r13
    shr     rax, 32
    mov     edx, eax
    call    asm_mmio_write32

    ; Write TDLEN
    mov     rcx, r12
    add     rcx, TDLEN
    mov     edx, r14d
    call    asm_mmio_write32

    ; Reset head and tail
    mov     rcx, r12
    add     rcx, TDH
    xor     edx, edx
    call    asm_mmio_write32

    mov     rcx, r12
    add     rcx, TDT
    xor     edx, edx
    call    asm_mmio_write32

    ; Configure TXDCTL (enable bit)
    mov     rcx, r12
    add     rcx, TXDCTL
    mov     edx, (1 << 25)      ; Enable bit
    call    asm_mmio_write32

    add     rsp, 40
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_enable_rx
; Enable receiver with standard configuration.
;
; Input:  RCX = mmio_base
; Output: None
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_enable_rx
asm_intel_enable_rx:
    push    rbx
    sub     rsp, 32

    mov     rbx, rcx            ; rbx = mmio_base

    ; RCTL = EN | BAM | BSIZE_2048 | SECRC
    mov     edx, RCTL_EN | RCTL_BAM | RCTL_BSIZE_2048 | RCTL_SECRC
    mov     rcx, rbx
    add     rcx, RCTL
    call    asm_mmio_write32

    add     rsp, 32
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_enable_tx
; Enable transmitter with standard configuration.
;
; Input:  RCX = mmio_base
; Output: None
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_enable_tx
asm_intel_enable_tx:
    push    rbx
    sub     rsp, 32

    mov     rbx, rcx            ; rbx = mmio_base

    ; TCTL = EN | PSP | CT=0x10 | COLD=0x40
    mov     edx, TCTL_EN | TCTL_PSP | (0x10 << TCTL_CT_SHIFT) | (0x40 << TCTL_COLD_SHIFT)
    mov     rcx, rbx
    add     rcx, TCTL
    call    asm_mmio_write32

    add     rsp, 32
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_set_link_up
; Force link up via CTRL register.
;
; Input:  RCX = mmio_base
; Output: None
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_set_link_up
asm_intel_set_link_up:
    push    rbx
    sub     rsp, 32

    mov     rbx, rcx            ; rbx = mmio_base

    ; Read current CTRL
    mov     rcx, rbx
    add     rcx, CTRL
    call    asm_mmio_read32

    ; Set SLU (Set Link Up) bit
    or      eax, CTRL_SLU
    mov     edx, eax
    mov     rcx, rbx
    add     rcx, CTRL
    call    asm_mmio_write32

    add     rsp, 32
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_read_reg
; Generic register read (for Rust fallback).
;
; Input:  RCX = mmio_base
;         RDX = offset
; Output: RAX = register value
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_read_reg
asm_intel_read_reg:
    add     rcx, rdx
    jmp     asm_mmio_read32

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_write_reg
; Generic register write (for Rust fallback).
;
; Input:  RCX = mmio_base
;         RDX = offset
;         R8  = value
; Output: None
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_write_reg
asm_intel_write_reg:
    add     rcx, rdx
    mov     edx, r8d
    jmp     asm_mmio_write32
