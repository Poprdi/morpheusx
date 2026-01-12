; ═══════════════════════════════════════════════════════════════════════════
; Intel e1000e RX Path
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_intel_rx_init_desc: Initialize an RX descriptor with buffer
;   - asm_intel_rx_poll: Poll for received packet
;   - asm_intel_rx_update_tail: Update RDT register
;   - asm_intel_rx_read_head: Read RDH register
;   - asm_intel_rx_clear_desc: Clear descriptor for reuse
;
; RX Descriptor Layout (16 bytes, legacy format):
;   Offset 0: Buffer Address (64-bit)
;   Offset 8: Length (16-bit), Checksum (16-bit)
;   Offset 12: Status (8-bit), Errors (8-bit), Special (16-bit)
;
; Status bits:
;   Bit 0 (DD):   Descriptor Done
;   Bit 1 (EOP):  End of Packet
;   Bit 2 (IXSM): Ignore Checksum Indication
;   Bit 5 (VP):   Packet is 802.1Q
;
; Error bits:
;   Bit 0 (CE):   CRC Error
;   Bit 1 (SE):   Symbol Error
;   Bit 2 (SEQ):  Sequence Error
;   Bit 5 (RXE):  RX Data Error
;
; Reference: Intel 82579 Datasheet Section 3.2.3
; ═══════════════════════════════════════════════════════════════════════════

section .data
    ; Register offsets
    RDT         equ 0x2818
    RDH         equ 0x2810

    ; Descriptor status bits
    RX_STA_DD   equ (1 << 0)
    RX_STA_EOP  equ (1 << 1)

    ; Descriptor error bits
    RX_ERR_CE   equ (1 << 0)
    RX_ERR_SE   equ (1 << 1)
    RX_ERR_SEQ  equ (1 << 2)
    RX_ERR_RXE  equ (1 << 5)
    RX_ERR_MASK equ (RX_ERR_CE | RX_ERR_SE | RX_ERR_SEQ | RX_ERR_RXE)

section .text

; External: Core primitives
extern asm_bar_sfence
extern asm_bar_lfence
extern asm_mmio_read32
extern asm_mmio_write32

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_rx_init_desc
; Initialize an RX descriptor with buffer address.
;
; Input:  RCX = descriptor pointer
;         RDX = buffer bus address
; Output: None
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_rx_init_desc
asm_intel_rx_init_desc:
    ; Write buffer address (offset 0)
    mov     [rcx], rdx

    ; Clear status fields (offset 8-15)
    xor     eax, eax
    mov     [rcx+8], rax

    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_rx_poll
; Poll an RX descriptor for received packet.
;
; Input:  RCX = descriptor pointer
;         RDX = result struct pointer:
;               [0:1]  = length (16-bit)
;               [2]    = status (8-bit)
;               [3]    = errors (8-bit)
; Output: RAX = 1 if packet received (DD set), 0 if not
;
; Result struct is only valid if RAX=1.
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_rx_poll
asm_intel_rx_poll:
    push    rbx
    push    r12
    push    r13
    sub     rsp, 32

    mov     r12, rcx            ; r12 = descriptor
    mov     r13, rdx            ; r13 = result

    ; Load fence before reading descriptor
    call    asm_bar_lfence

    ; Read status/length dwords
    mov     eax, [r12+8]        ; length (16-bit) + checksum (16-bit)
    mov     ebx, [r12+12]       ; status (8-bit) + errors (8-bit) + special (16-bit)

    ; Check DD bit
    test    bl, RX_STA_DD
    jz      .no_packet

    ; Packet received - fill result struct
    mov     [r13], ax           ; length (16-bit)
    mov     [r13+2], bl         ; status (8-bit)
    ; Extract errors byte (bits 8-15 of ebx) without using bh
    mov     ecx, ebx
    shr     ecx, 8
    mov     [r13+3], cl         ; errors (8-bit)

    mov     eax, 1
    jmp     .exit

.no_packet:
    xor     eax, eax

.exit:
    add     rsp, 32
    pop     r13
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_rx_update_tail
; Update RDT register.
;
; Input:  RCX = mmio_base
;         RDX = new tail value
; Output: None
;
; Includes sfence before MMIO write.
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_rx_update_tail
asm_intel_rx_update_tail:
    push    rbx
    push    r12
    sub     rsp, 32

    mov     r12, rcx            ; r12 = mmio_base
    mov     ebx, edx            ; ebx = tail value

    ; Store fence before tail update
    call    asm_bar_sfence

    ; Write RDT
    mov     rcx, r12
    add     rcx, RDT
    mov     edx, ebx
    call    asm_mmio_write32

    add     rsp, 32
    pop     r12
    pop     rbx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_rx_read_head
; Read RDH register (head pointer).
;
; Input:  RCX = mmio_base
; Output: RAX = head value
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_rx_read_head
asm_intel_rx_read_head:
    add     rcx, RDH
    jmp     asm_mmio_read32

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_rx_clear_desc
; Clear descriptor status for reuse.
;
; Input:  RCX = descriptor pointer
; Output: None
;
; Preserves buffer address, clears status.
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_rx_clear_desc
asm_intel_rx_clear_desc:
    ; Clear status fields (offset 8-15), keep buffer address
    xor     eax, eax
    mov     [rcx+8], rax
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_rx_get_length
; Get packet length from descriptor.
;
; Input:  RCX = descriptor pointer
; Output: RAX = length (16-bit, zero-extended)
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_rx_get_length
asm_intel_rx_get_length:
    movzx   eax, word [rcx+8]
    ret

; ═══════════════════════════════════════════════════════════════════════════
; asm_intel_rx_check_errors
; Check if descriptor has errors.
;
; Input:  RCX = descriptor pointer
; Output: RAX = 0 if no errors, non-zero if errors
; ═══════════════════════════════════════════════════════════════════════════
global asm_intel_rx_check_errors
asm_intel_rx_check_errors:
    movzx   eax, byte [rcx+13]  ; errors byte
    and     eax, RX_ERR_MASK
    ret
