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
global asm_xhci_controller_soft_restart
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
    mov     eax, 1              ; timeout waiting owner release
    jmp     .bios_out

.bios_claimed:
.bios_none:
    xor     eax, eax
.bios_out:
    add     rsp, 40
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_xhci_controller_soft_restart
; ───────────────────────────────────────────────────────────────────────────
; Restart xHCI without hard reset (preserves port state from UEFI handoff).
; Assumes UEFI left controller in a valid state. We just stop and restart.
; RCX = op_base (mmio_base + CAPLENGTH)
; RDX = tsc_freq
; Returns: EAX = 0 success, 1 halt timeout, 2 start timeout
asm_xhci_controller_soft_restart:
    push    rbx
    push    r12
    push    r13
    push    r14
    sub     rsp, 40

    mov     r12, rcx            ; op_base
    mov     r13, rdx            ; tsc_freq
    mov     r14, r13            ; 1 second timeout = tsc_freq ticks

    ; ── step 1: stop controller (RS = 0) ──
    lea     rcx, [r12 + XHCI_OP_USBCMD]
    xor     edx, edx
    call    asm_mmio_write32
    call    asm_bar_mfence

    ; ── wait for HCH (halted) ──
    call    asm_tsc_read
    mov     rbx, rax
.wait_halt_soft:
    lea     rcx, [r12 + XHCI_OP_USBSTS]
    call    asm_mmio_read32
    test    eax, XHCI_STS_HCH
    jnz     .halted_soft
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r14
    jb      .wait_halt_soft
    mov     eax, 1              ; halt timeout
    jmp     .out_soft

.halted_soft:
    ; ── clear status bits ──
    lea     rcx, [r12 + XHCI_OP_USBSTS]
    mov     edx, 0xFFFFFFFF
    call    asm_mmio_write32
    call    asm_bar_mfence

    ; ── start controller (RS = 1, INTE = 1) — NO HCRST ──
    lea     rcx, [r12 + XHCI_OP_USBCMD]
    mov     edx, XHCI_CMD_RS | XHCI_CMD_INTE
    call    asm_mmio_write32
    call    asm_bar_mfence

    ; ── wait for HCH to clear (running) ──
    call    asm_tsc_read
    mov     rbx, rax
.wait_running:
    lea     rcx, [r12 + XHCI_OP_USBSTS]
    call    asm_mmio_read32
    test    eax, XHCI_STS_HCH
    jz      .running_soft
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r14
    jb      .wait_running
    mov     eax, 2              ; start timeout
    jmp     .out_soft

.running_soft:
    xor     eax, eax

.out_soft:
    add     rsp, 40
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret
