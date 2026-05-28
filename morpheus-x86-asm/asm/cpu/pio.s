; ═══════════════════════════════════════════════════════════════════════════
; Port I/O primitives
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_pio_read8: Read 8-bit from I/O port
;   - asm_pio_write8: Write 8-bit to I/O port
;   - asm_pio_read16: Read 16-bit from I/O port
;   - asm_pio_write16: Write 16-bit to I/O port
;   - asm_pio_read32: Read 32-bit from I/O port
;   - asm_pio_write32: Write 32-bit to I/O port
;
; Port I/O is used for PCI Legacy configuration space access (CF8/CFC)
; and some older hardware. The IN/OUT instructions are inherently serializing.
;
; Reference: ARCHITECTURE_V3.md - PIO layer
; ═══════════════════════════════════════════════════════════════════════════

section .text

; Export symbols
global asm_pio_read8
global asm_pio_write8
global asm_pio_read16
global asm_pio_write16
global asm_pio_read32
global asm_pio_write32

; ───────────────────────────────────────────────────────────────────────────
; asm_pio_read8
; ───────────────────────────────────────────────────────────────────────────
; Read 8-bit value from I/O port
;
; Parameters:
;   RCX = port number (only low 16 bits used)
; Returns:
;   RAX = 8-bit value (zero-extended)
; Clobbers: None (DX used internally but restored via implicit behavior)
; ───────────────────────────────────────────────────────────────────────────
asm_pio_read8:
    mov     dx, cx              ; Port number to DX (IN uses DX for port > 255)
    xor     eax, eax            ; Clear RAX
    in      al, dx              ; Read 8-bit from port
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pio_write8
; ───────────────────────────────────────────────────────────────────────────
; Write 8-bit value to I/O port
;
; Parameters:
;   RCX = port number (only low 16 bits used)
;   RDX = value to write (only low 8 bits used)
; Returns: None
; Clobbers: RAX (used for value)
; ───────────────────────────────────────────────────────────────────────────
asm_pio_write8:
    mov     al, dl              ; Value to AL
    mov     dx, cx              ; Port to DX
    out     dx, al              ; Write 8-bit to port
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pio_read16
; ───────────────────────────────────────────────────────────────────────────
; Read 16-bit value from I/O port
;
; Parameters:
;   RCX = port number (only low 16 bits used)
; Returns:
;   RAX = 16-bit value (zero-extended)
; Clobbers: None
; ───────────────────────────────────────────────────────────────────────────
asm_pio_read16:
    mov     dx, cx              ; Port number to DX
    xor     eax, eax            ; Clear RAX
    in      ax, dx              ; Read 16-bit from port
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pio_write16
; ───────────────────────────────────────────────────────────────────────────
; Write 16-bit value to I/O port
;
; Parameters:
;   RCX = port number (only low 16 bits used)
;   RDX = value to write (only low 16 bits used)
; Returns: None
; Clobbers: RAX
; ───────────────────────────────────────────────────────────────────────────
asm_pio_write16:
    mov     ax, dx              ; Value to AX (note: this gets low 16 of RDX)
    mov     dx, cx              ; Port to DX
    out     dx, ax              ; Write 16-bit to port
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pio_read32
; ───────────────────────────────────────────────────────────────────────────
; Read 32-bit value from I/O port
;
; Parameters:
;   RCX = port number (only low 16 bits used)
; Returns:
;   RAX = 32-bit value (zero-extended to 64-bit)
; Clobbers: None
; ───────────────────────────────────────────────────────────────────────────
asm_pio_read32:
    mov     dx, cx              ; Port number to DX
    in      eax, dx             ; Read 32-bit from port (clears upper RAX)
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pio_write32
; ───────────────────────────────────────────────────────────────────────────
; Write 32-bit value to I/O port
;
; Parameters:
;   RCX = port number (only low 16 bits used)
;   RDX = value to write (only low 32 bits used)
; Returns: None
; Clobbers: RAX
; ───────────────────────────────────────────────────────────────────────────
asm_pio_write32:
    mov     eax, edx            ; Value to EAX
    mov     dx, cx              ; Port to DX
    out     dx, eax             ; Write 32-bit to port
    ret
