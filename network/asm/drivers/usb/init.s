; ═══════════════════════════════════════════════════════════════════════════
; USB host controller probe/reset primitives (scaffold)
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════

section .text

extern asm_mmio_read8
extern asm_tsc_read

global asm_usb_host_probe
global asm_usb_host_reset

; RCX = mmio_base
; EAX = 0 success, 1 invalid controller signature
asm_usb_host_probe:
    sub     rsp, 32
    ; Read capability length register (common on xHCI-compatible mmio layout).
    call    asm_mmio_read8
    test    al, al
    jz      .bad
    xor     eax, eax
    jmp     .out
.bad:
    mov     eax, 1
.out:
    add     rsp, 32
    ret

; RCX = mmio_base, RDX = tsc_freq
; EAX = 0 success (stub reset sequence placeholder)
asm_usb_host_reset:
    ; Reserved for full xHCI reset sequence in next phase.
    xor     eax, eax
    ret
