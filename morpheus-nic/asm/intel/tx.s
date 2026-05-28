; ═══════════════════════════════════════════════════════════════════════════
; Intel e1000e TX Path
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_intel_tx_init_desc: Initialize a TX descriptor
;   - asm_intel_tx_submit: Submit a packet for transmission
;   - asm_intel_tx_poll: Poll for TX completion
;   - asm_intel_tx_update_tail: Update TDT register
;   - asm_intel_tx_read_head: Read TDH register
;
; TX Descriptor Layout (16 bytes, legacy format):
;   Offset 0: Buffer Address (64-bit)
;   Offset 8: Length (16-bit), CSO (8-bit), CMD (8-bit)
;   Offset 12: STA (4-bit), Reserved, CSS (8-bit), Special (16-bit)
;
; CMD bits:
;   Bit 0 (EOP):  End of Packet
;   Bit 1 (IFCS): Insert FCS/CRC
;   Bit 3 (RS):   Report Status
;   Bit 5 (DEXT): Descriptor Extension (0 for legacy)
;
; STA bits:
;   Bit 0 (DD): Descriptor Done
;
; Reference: Intel 82579 Datasheet Section 3.3.3
; ═══════════════════════════════════════════════════════════════════════════

section .data
    ; Register offsets
    TDT         equ 0x3818
    TDH         equ 0x3810

    ; Descriptor CMD bits
    TX_CMD_EOP  equ (1 << 0)
    TX_CMD_IFCS equ (1 << 1)
    TX_CMD_RS   equ (1 << 3)

    ; Descriptor STA bits
    TX_STA_DD   equ (1 << 0)

section .text

; External: Core primitives
extern asm_bar_sfence
extern asm_bar_lfence
extern asm_mmio_read32
extern asm_mmio_write32

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_tx_init_desc
; Initialize a TX descriptor to zero.
;
; Input:  RCX = descriptor pointer
; Output: None
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_tx_init_desc
asm_intel_tx_init_desc:
    ; Zero all 16 bytes
    xor     eax, eax
    mov     [rcx], rax          ; bytes 0-7
    mov     [rcx+8], rax        ; bytes 8-15
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_tx_submit
; Submit a packet for transmission.
;
; Input:  RCX = descriptor pointer
;         RDX = buffer bus address
;         R8  = packet length
; Output: None
;
; Sets EOP, IFCS, RS command bits.
; Includes sfence to ensure descriptor is visible before tail update.
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_tx_submit
asm_intel_tx_submit:
    push    rbx
    sub     rsp, 32

    ; Write buffer address (offset 0)
    mov     [rcx], rdx

    ; Build command/length dword
    ; [8:9]   = length (16-bit)
    ; [10]    = CSO (0)
    ; [11]    = CMD (EOP | IFCS | RS)
    mov     eax, r8d            ; length in low 16 bits
    and     eax, 0xFFFF
    mov     ebx, TX_CMD_EOP | TX_CMD_IFCS | TX_CMD_RS
    shl     ebx, 24             ; CMD in high byte
    or      eax, ebx

    ; Write command/length (offset 8)
    mov     [rcx+8], eax

    ; Clear status/special (offset 12)
    xor     eax, eax
    mov     [rcx+12], eax

    ; Store fence to ensure descriptor visible
    call    asm_bar_sfence

    add     rsp, 32
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_tx_poll
; Poll a TX descriptor for completion.
;
; Input:  RCX = descriptor pointer
; Output: RAX = 1 if DD set (complete), 0 if not
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_tx_poll
asm_intel_tx_poll:
    ; Load fence first
    push    rcx
    sub     rsp, 32
    call    asm_bar_lfence
    add     rsp, 32
    pop     rcx

    ; Read status byte (offset 12, bits 0-3)
    mov     eax, [rcx+12]
    and     eax, TX_STA_DD
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_tx_update_tail
; Update TDT register.
;
; Input:  RCX = mmio_base
;         RDX = new tail value
; Output: None
;
; Includes sfence before MMIO write.
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_tx_update_tail
asm_intel_tx_update_tail:
    push    rbx
    push    r12
    sub     rsp, 32

    mov     r12, rcx            ; r12 = mmio_base
    mov     ebx, edx            ; ebx = tail value

    ; Store fence before tail update
    call    asm_bar_sfence

    ; Write TDT
    mov     rcx, r12
    add     rcx, TDT
    mov     edx, ebx
    call    asm_mmio_write32

    add     rsp, 32
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_tx_read_head
; Read TDH register (head pointer).
;
; Input:  RCX = mmio_base
; Output: RAX = head value
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_tx_read_head
asm_intel_tx_read_head:
    add     rcx, TDH
    jmp     asm_mmio_read32

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_tx_clear_desc
; Clear DD bit in descriptor (for reuse).
;
; Input:  RCX = descriptor pointer
; Output: None
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_tx_clear_desc
asm_intel_tx_clear_desc:
    ; Clear status (offset 12)
    xor     eax, eax
    mov     [rcx+12], eax
    ret
