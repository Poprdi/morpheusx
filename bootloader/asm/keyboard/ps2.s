; PS/2 port I/O primitives — ABI: Microsoft x64

section .text

global asm_ps2_read_status
global asm_ps2_write_cmd
global asm_ps2_write_data
global asm_ps2_poll
global asm_ps2_poll_any
global asm_ps2_flush

PS2_DATA   equ 0x60
PS2_STATUS equ 0x64
PS2_CMD    equ 0x64
OBF        equ 0x01
IBF        equ 0x02
AUXB       equ 0x20        ; mouse OBF (bit 5)

IBF_WAIT   equ 65535
FLUSH_MAX  equ 256

asm_ps2_read_status:
    xor     eax, eax
    in      al, PS2_STATUS
    ret

; waits IBF=0 then writes CL to command port
asm_ps2_write_cmd:
    mov     r8, IBF_WAIT
.wcmd_ibf_wait:
    in      al, PS2_STATUS
    test    al, IBF
    jz      .wcmd_send
    pause
    dec     r8
    jnz     .wcmd_ibf_wait
    ret
.wcmd_send:
    mov     al, cl
    out     PS2_CMD, al
    ret

; waits IBF=0 then writes CL to data port
asm_ps2_write_data:
    mov     r8, IBF_WAIT
.wdat_ibf_wait:
    in      al, PS2_STATUS
    test    al, IBF
    jz      .wdat_send
    pause
    dec     r8
    jnz     .wdat_ibf_wait
    ret
.wdat_send:
    mov     al, cl
    out     PS2_DATA, al
    ret

; asm_ps2_poll() -> u32   0=empty, 0x1xx=keyboard byte xx
asm_ps2_poll:
    xor     eax, eax
    in      al, PS2_STATUS
    test    al, OBF
    jz      .poll_empty
    test    al, AUXB
    jnz     .poll_mouse
    xor     eax, eax
    in      al, PS2_DATA
    or      eax, 0x100
    ret
.poll_mouse:
    in      al, PS2_DATA
.poll_empty:
    xor     eax, eax
    ret

; asm_ps2_poll_any() -> u32   0=empty, 0x1xx=keyboard xx, 0x3xx=mouse xx
asm_ps2_poll_any:
    xor     eax, eax
    in      al, PS2_STATUS
    test    al, OBF
    jz      .pollany_empty
    test    al, AUXB               ; AUXB=1 → mouse port
    jnz     .pollany_mouse
    ; keyboard byte
    xor     eax, eax
    in      al, PS2_DATA
    or      eax, 0x100             ; 0x1xx
    ret
.pollany_mouse:
    xor     eax, eax
    in      al, PS2_DATA
    or      eax, 0x300             ; 0x3xx
    ret
.pollany_empty:
    xor     eax, eax
    ret

; asm_ps2_flush()   drain output buffer (max FLUSH_MAX reads)
asm_ps2_flush:
    mov     ecx, FLUSH_MAX
.flush_loop:
    in      al, PS2_STATUS
    test    al, OBF
    jz      .flush_done
    in      al, PS2_DATA
    pause
    dec     ecx
    jnz     .flush_loop
.flush_done:
    ret
