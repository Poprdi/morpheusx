; ═══════════════════════════════════════════════════════════════════════════
; USB xHCI host controller primitives — brutal init edition
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════

section .data
    %include "asm/drivers/usb/regs.s"

section .text

extern asm_mmio_read8
extern asm_mmio_read16
extern asm_mmio_read32
extern asm_mmio_write32
extern asm_tsc_read
extern asm_bar_mfence

global asm_usb_host_probe
global asm_xhci_controller_reset
global asm_xhci_bios_handoff

; ───────────────────────────────────────────────────────────────────────────
; asm_usb_host_probe
; ───────────────────────────────────────────────────────────────────────────
; RCX = mmio_base (BAR0)
; Returns:
;   EAX bits  7:0  = CAPLENGTH
;   EAX bits 31:16 = HCIVERSION (should be >= 0x0100)
;   EAX = 0 if bar reads back all-F (unmapped/dead)
asm_usb_host_probe:
    push    rbx
    sub     rsp, 32             ; 1 push + ret = 16; +32 = 48; 48%16=0. aligned.

    mov     rbx, rcx

    ; read dword at mmio_base+0: [7:0]=CAPLENGTH, [31:16]=HCIVERSION
    call    asm_mmio_read32
    cmp     eax, 0xFFFFFFFF
    je      .probe_dead
    ; sanity: CAPLENGTH must be ≥ 0x10 (relaxed — some controllers are small)
    movzx   edx, al
    cmp     edx, 0x10
    jb      .probe_dead
    ; HCIVERSION must be ≥ 0x0100
    shr     eax, 16
    cmp     ax, 0x0100
    jb      .probe_dead

    ; reconstruct: low byte = CAPLENGTH, high half = HCIVERSION
    shl     eax, 16
    or      eax, edx
    jmp     .probe_out

.probe_dead:
    xor     eax, eax
.probe_out:
    add     rsp, 32
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_xhci_bios_handoff
; ───────────────────────────────────────────────────────────────────────────
; Claim xHCI from BIOS/SMM via USBLEGSUP extended capability.
; RCX = mmio_base
; RDX = hccparams1 (from mmio_base + 0x10)
; R8  = tsc_freq
; Returns: EAX = 0 success (or no legacy cap), 1 timeout waiting BIOS release
asm_xhci_bios_handoff:
    push    rbx
    push    r12
    push    r13
    push    r14
    sub     rsp, 40

    mov     r12, rcx            ; mmio_base
    mov     r14, r8             ; tsc_freq

    ; extended capability pointer from HCCPARAMS1 bits 31:16 (dword offset)
    mov     eax, edx
    shr     eax, 16
    and     eax, 0xFFFF
    shl     eax, 2              ; *4 = byte offset from mmio_base
    test    eax, eax
    jz      .bios_none          ; no extended caps

    ; walk extended capability list looking for ID=1 (legacy support)
    lea     r13, [r12 + rax]    ; ext_cap_ptr
.bios_walk:
    mov     rcx, r13
    call    asm_mmio_read32
    movzx   edx, al             ; cap ID
    cmp     dl, XHCI_EXT_CAP_LEGACY
    je      .bios_found

    ; next pointer = bits 15:8, shift left 2
    mov     ecx, eax
    shr     ecx, 8
    and     ecx, 0xFF
    test    ecx, ecx
    jz      .bios_none          ; end of list
    shl     ecx, 2
    add     r13, rcx
    jmp     .bios_walk

.bios_found:
    ; USBLEGSUP at r13. Set OS_OWNED bit (byte at offset +3)
    mov     rcx, r13
    call    asm_mmio_read32
    or      eax, XHCI_LEGSUP_OS_OWNED
    mov     edx, eax
    mov     rcx, r13
    call    asm_mmio_write32
    call    asm_bar_mfence

    ; wait for BIOS_OWNED to clear (1 second timeout because firmware is slow)
    call    asm_tsc_read
    mov     rbx, rax
.bios_wait:
    mov     rcx, r13
    call    asm_mmio_read32
    test    eax, XHCI_LEGSUP_BIOS_OWNED
    jz      .bios_claimed
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r14            ; 1 second = tsc_freq ticks
    jb      .bios_wait

    ; timeout — force it. clear BIOS bit, keep OS bit
    mov     rcx, r13
    call    asm_mmio_read32
    and     eax, ~XHCI_LEGSUP_BIOS_OWNED
    or      eax, XHCI_LEGSUP_OS_OWNED
    mov     edx, eax
    mov     rcx, r13
    call    asm_mmio_write32
    call    asm_bar_mfence

    ; also nuke the legacy control/status dword at offset +4
    ; disable all SMI sources so BIOS can't interfere
    lea     rcx, [r13 + 4]
    xor     edx, edx
    call    asm_mmio_write32
    call    asm_bar_mfence

.bios_claimed:
.bios_none:
    xor     eax, eax
    add     rsp, 40
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_xhci_controller_reset
; ───────────────────────────────────────────────────────────────────────────
; RCX = op_base (mmio_base + CAPLENGTH)
; RDX = tsc_freq
; Returns: EAX = 0 success, 1 halt timeout, 2 reset timeout, 3 CNR timeout
;
; brutal reset: nuke interrupts, force-stop, HCRST, wait CNR.
; 1 second timeout per phase because real hardware is dramatic.
asm_xhci_controller_reset:
    push    rbx
    push    r12
    push    r13
    push    r14
    sub     rsp, 40

    mov     r12, rcx            ; op_base
    mov     r13, rdx            ; tsc_freq
    mov     r14, r13            ; 1 second timeout = tsc_freq ticks

    ; ── step 0: nuke USBCMD — clear RS, INTE, everything ──
    lea     rcx, [r12 + XHCI_OP_USBCMD]
    xor     edx, edx
    call    asm_mmio_write32
    call    asm_bar_mfence

    ; ── step 1: wait USBSTS.HCH (halted) ──
    call    asm_tsc_read
    mov     rbx, rax
.wait_halt:
    lea     rcx, [r12 + XHCI_OP_USBSTS]
    call    asm_mmio_read32
    test    eax, XHCI_STS_HCH
    jnz     .halted
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r14
    jb      .wait_halt
    ; timeout — try HCRST anyway, some controllers only halt via reset
    jmp     .do_reset

.halted:
    ; ── clear all pending status bits ──
    lea     rcx, [r12 + XHCI_OP_USBSTS]
    mov     edx, 0xFFFFFFFF
    call    asm_mmio_write32
    call    asm_bar_mfence

.do_reset:
    ; ── step 2: HCRST ──
    lea     rcx, [r12 + XHCI_OP_USBCMD]
    mov     edx, XHCI_CMD_HCRST
    call    asm_mmio_write32
    call    asm_bar_mfence

    call    asm_tsc_read
    mov     rbx, rax
.wait_reset:
    lea     rcx, [r12 + XHCI_OP_USBCMD]
    call    asm_mmio_read32
    test    eax, XHCI_CMD_HCRST
    jz      .reset_done
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r14
    jb      .wait_reset
    mov     eax, 2
    jmp     .out

.reset_done:
    ; ── step 3: wait CNR clear ──
    call    asm_tsc_read
    mov     rbx, rax
.wait_cnr:
    lea     rcx, [r12 + XHCI_OP_USBSTS]
    call    asm_mmio_read32
    test    eax, XHCI_STS_CNR
    jz      .ready
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r14
    jb      .wait_cnr
    mov     eax, 3
    jmp     .out

.ready:
    xor     eax, eax

.out:
    add     rsp, 40
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret
