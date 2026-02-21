; ═══════════════════════════════════════════════════════════════════════════
; PS/2 Keyboard Controller — port I/O primitives
; ABI: Microsoft x64 (RCX = first arg, return in RAX)
; ═══════════════════════════════════════════════════════════════════════════
;
; Ports:
;   0x60 = PS/2 data port   (read: byte from device, write: byte to device)
;   0x64 = PS/2 status/cmd  (read: status register, write: controller command)
;
; Status register (port 0x64) bits:
;   Bit 0 (OBF):  Output Buffer Full — 1 = data ready to read from 0x60
;   Bit 1 (IBF):  Input Buffer Full  — 1 = controller busy, do not write
;   Bit 5 (AUXB): Aux (mouse) OBF   — if set, data in 0x60 is from mouse
;
; Functions exported:
;   asm_ps2_read_status  — read status register (port 0x64)
;   asm_ps2_write_cmd    — write to command port (0x64), waits IBF=0
;   asm_ps2_write_data   — write to data port   (0x60), waits IBF=0
;   asm_ps2_poll         — non-blocking: 0 if empty, (0x100|byte) if ready
;   asm_ps2_flush        — drain output buffer (bounded 256 reads)
;
; Reuse policy:
;   Port I/O instructions (in/out) are used directly here — same as
;   hwinit/asm/cpu/pio.s. The keyboard driver lives in the bootloader crate
;   and cannot call hwinit's asm symbols across crate boundaries, so we own
;   our own port-touching layer here. Pattern is identical to pio.s.
;
; Reference: hwinit/asm/cpu/pio.s, hwinit/asm/cpu/delay.s
; ═══════════════════════════════════════════════════════════════════════════

section .text

global asm_ps2_read_status
global asm_ps2_write_cmd
global asm_ps2_write_data
global asm_ps2_poll
global asm_ps2_flush

PS2_DATA   equ 0x60        ; Data port
PS2_STATUS equ 0x64        ; Status register (read)
PS2_CMD    equ 0x64        ; Command port (write)
OBF        equ 0x01        ; Output Buffer Full — bit 0
IBF        equ 0x02        ; Input Buffer Full  — bit 1
AUXB       equ 0x20        ; Aux (mouse) OBF    — bit 5

IBF_WAIT   equ 65535       ; Max spins before giving up on IBF=0
FLUSH_MAX  equ 256         ; Max bytes to discard in flush

; ───────────────────────────────────────────────────────────────────────────
; asm_ps2_read_status() -> u8
; ───────────────────────────────────────────────────────────────────────────
; Read the PS/2 status register from port 0x64.
;
; Returns:
;   AL (RAX zero-extended) = status byte
; Clobbers: RAX
; ───────────────────────────────────────────────────────────────────────────
asm_ps2_read_status:
    xor     eax, eax
    in      al, PS2_STATUS
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_ps2_write_cmd(cmd: u8)
; ───────────────────────────────────────────────────────────────────────────
; Write a command byte to the PS/2 controller command port (0x64).
; Spins until IBF=0 (controller not busy) before writing.
; Gives up after IBF_WAIT iterations if controller stays busy.
;
; Parameters:
;   RCX = command byte (low 8 bits = CL)
; Returns: none
; Clobbers: RAX, R8
; ───────────────────────────────────────────────────────────────────────────
asm_ps2_write_cmd:
    mov     r8, IBF_WAIT            ; Timeout counter
.wcmd_ibf_wait:
    in      al, PS2_STATUS          ; Read status
    test    al, IBF                 ; IBF set? (controller busy)
    jz      .wcmd_send              ; IBF=0 — ready to write
    pause                           ; Spin hint (mirrors asm_spin_hint)
    dec     r8
    jnz     .wcmd_ibf_wait          ; Keep waiting
.wcmd_send:
    mov     al, cl                  ; Low byte of RCX = command
    out     PS2_CMD, al             ; Write to command port
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_ps2_write_data(data: u8)
; ───────────────────────────────────────────────────────────────────────────
; Write a data byte to the PS/2 data port (0x60).
; Spins until IBF=0 before writing. Same IBF-wait pattern as write_cmd
; but targets port 0x60 (data) instead of 0x64 (command).
;
; Parameters:
;   RCX = data byte (low 8 bits = CL)
; Returns: none
; Clobbers: RAX, R8
; ───────────────────────────────────────────────────────────────────────────
asm_ps2_write_data:
    mov     r8, IBF_WAIT
.wdat_ibf_wait:
    in      al, PS2_STATUS
    test    al, IBF
    jz      .wdat_send
    pause
    dec     r8
    jnz     .wdat_ibf_wait
.wdat_send:
    mov     al, cl                  ; Low byte of RCX = data
    out     PS2_DATA, al            ; Write to data port
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_ps2_poll() -> u32
; ───────────────────────────────────────────────────────────────────────────
; Non-blocking check for an available keyboard scancode.
;
; Mouse bytes (AUXB set in status) are silently discarded — read and dropped
; so they don't block keyboard data.
;
; Returns:
;   0x00000000        — output buffer empty (OBF=0), no data
;   0x000001xx        — keyboard byte ready; xx = scan byte from port 0x60
;
; Caller pattern:
;   let r = asm_ps2_poll();
;   if r & 0x100 != 0 { let byte = (r & 0xFF) as u8; ... }
;
; Clobbers: RAX
; ───────────────────────────────────────────────────────────────────────────
asm_ps2_poll:
    xor     eax, eax
    in      al, PS2_STATUS          ; Read status
    test    al, OBF                 ; Output buffer has data?
    jz      .poll_empty             ; OBF=0 — nothing to read
    test    al, AUXB                ; Is it mouse data?
    jnz     .poll_mouse             ; Yes — drain and return empty
    ; Keyboard data available — read and flag it
    xor     eax, eax
    in      al, PS2_DATA            ; AL = scan byte
    or      eax, 0x100              ; Set has-data flag (bit 8)
    ret
.poll_mouse:
    in      al, PS2_DATA            ; Read and discard mouse byte
.poll_empty:
    xor     eax, eax                ; Return 0 = no keyboard data
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_ps2_flush()
; ───────────────────────────────────────────────────────────────────────────
; Drain the PS/2 output buffer by reading until OBF=0, discarding all bytes.
; Bounded to FLUSH_MAX reads to prevent infinite loops on broken hardware.
;
; Call after PS/2 init and after ExitBootServices to discard any stale
; scancodes or self-test bytes left by UEFI.
;
; Parameters: none
; Returns: none
; Clobbers: RAX, RCX
; ───────────────────────────────────────────────────────────────────────────
asm_ps2_flush:
    mov     ecx, FLUSH_MAX
.flush_loop:
    in      al, PS2_STATUS          ; Read status
    test    al, OBF                 ; Data available?
    jz      .flush_done             ; OBF=0 — buffer clear
    in      al, PS2_DATA            ; Read and discard
    pause                           ; Spin hint between reads
    dec     ecx
    jnz     .flush_loop
.flush_done:
    ret
