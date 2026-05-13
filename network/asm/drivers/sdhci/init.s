; ═══════════════════════════════════════════════════════════════════════════
; SDHCI initialization primitives
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════

section .data
    %include "asm/drivers/sdhci/regs.s"

section .text

extern asm_mmio_read32
extern asm_mmio_write32
extern asm_mmio_write16
extern asm_mmio_read8
extern asm_mmio_write8
extern asm_tsc_read
extern asm_bar_mfence

global asm_sdhci_read_caps
global asm_sdhci_card_present
global asm_sdhci_controller_reset
global asm_sdhci_basic_power_clock
global asm_sdhci_read_block_pio

; RCX = mmio_base
; EAX = capabilities
asm_sdhci_read_caps:
    sub     rsp, 32
    lea     rcx, [rcx + SDHCI_REG_CAPABILITIES]
    call    asm_mmio_read32
    add     rsp, 32
    ret

; RCX = mmio_base
; EAX = 1 if card inserted, else 0
asm_sdhci_card_present:
    sub     rsp, 32
    lea     rcx, [rcx + SDHCI_REG_PRESENT_STATE]
    call    asm_mmio_read32
    and     eax, SDHCI_PRESENT_CARD_INSERTED
    xor     edx, edx
    test    eax, eax
    setnz   dl
    mov     eax, edx
    add     rsp, 32
    ret

; RCX = mmio_base, RDX = tsc_freq
; EAX = 0 success, 1 timeout
asm_sdhci_controller_reset:
    push    rbx
    push    r12
    push    r13
    sub     rsp, 32

    mov     r12, rcx            ; base
    mov     r13, rdx            ; tsc_freq

    ; Write software reset all.
    lea     rcx, [r12 + SDHCI_REG_SOFTWARE_RESET]
    mov     dl, SDHCI_RESET_ALL
    call    asm_mmio_write8
    call    asm_bar_mfence

    ; Timeout ~100ms.
    call    asm_tsc_read
    mov     rbx, rax
    mov     rax, r13
    xor     rdx, rdx
    mov     rcx, 10
    div     rcx                 ; tsc_freq / 10
    mov     r13, rax

.reset_wait:
    lea     rcx, [r12 + SDHCI_REG_SOFTWARE_RESET]
    call    asm_mmio_read8
    test    al, SDHCI_RESET_ALL
    jz      .ok

    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r13
    jb      .reset_wait

    mov     eax, 1
    jmp     .out

.ok:
    xor     eax, eax

.out:
    add     rsp, 32
    pop     r13
    pop     r12
    pop     rbx
    ret

; RCX = mmio_base
; EAX = 0 on success (minimal setup)
asm_sdhci_basic_power_clock:
    sub     rsp, 32

    ; Best-effort minimal power enable for bring-up path.
    lea     rcx, [rcx + SDHCI_REG_POWER_CTRL]
    mov     dl, 0x0F            ; bus power + nominal voltage bits
    call    asm_mmio_write8

    ; Enable internal clock + SD clock (controller-specific, minimal).
    lea     rcx, [rcx + SDHCI_REG_CLOCK_CTRL]
    mov     dx, 0x0005
    call    asm_mmio_write16

    xor     eax, eax
    add     rsp, 32
    ret

; RCX = mmio_base, RDX = lba, R8 = dst_ptr, R9 = tsc_freq
; EAX = 0 success
;       1 timeout waiting inhibit clear
;       2 command phase error/timeout
;       3 buffer-read phase error/timeout
;       4 transfer-complete phase error/timeout
asm_sdhci_read_block_pio:
    push    rbx
    push    r12
    push    r13
    push    r14
    push    r15
    sub     rsp, 32

    mov     r12, rcx            ; base
    mov     r13, rdx            ; lba
    mov     r14, r8             ; dst

    ; 100ms timeout ticks
    mov     rax, r9
    xor     rdx, rdx
    mov     rcx, 10
    div     rcx
    mov     r15, rax

    ; wait until command/data lines are idle
    call    asm_tsc_read
    mov     rbx, rax
.wait_idle:
    lea     rcx, [r12 + SDHCI_REG_PRESENT_STATE]
    call    asm_mmio_read32
    mov     edx, SDHCI_PRESENT_CMD_INHIBIT | SDHCI_PRESENT_DAT_INHIBIT
    test    eax, edx
    jz      .idle_ok
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r15
    jb      .wait_idle
    mov     eax, 1
    jmp     .out

.idle_ok:
    ; clear stale interrupts
    lea     rcx, [r12 + SDHCI_REG_INT_STATUS]
    mov     edx, 0xFFFF_FFFF
    call    asm_mmio_write32

    ; block size = 512, block count = 1
    lea     rcx, [r12 + SDHCI_REG_BLOCK_SIZE]
    mov     dx, 512
    call    asm_mmio_write16

    lea     rcx, [r12 + SDHCI_REG_BLOCK_COUNT]
    mov     dx, 1
    call    asm_mmio_write16

    ; timeout control: max
    lea     rcx, [r12 + SDHCI_REG_TIMEOUT_CTRL]
    mov     dl, 0x0E
    call    asm_mmio_write8

    ; argument = LBA (SDHC/SDXC block addressing)
    lea     rcx, [r12 + SDHCI_REG_ARGUMENT]
    mov     edx, r13d
    call    asm_mmio_write32

    ; transfer mode: read + block count enable
    lea     rcx, [r12 + SDHCI_REG_TRANSFER_MODE]
    mov     dx, SDHCI_TRNS_READ | SDHCI_TRNS_BLK_CNT_EN
    call    asm_mmio_write16

    ; CMD17 READ_SINGLE_BLOCK, short resp + crc + index + data
    lea     rcx, [r12 + SDHCI_REG_COMMAND]
    mov     dx, (17 << 8) | SDHCI_CMD_RESP_SHORT | SDHCI_CMD_CRC | SDHCI_CMD_INDEX | SDHCI_CMD_DATA
    call    asm_mmio_write16

    ; wait command complete
    call    asm_tsc_read
    mov     rbx, rax
.wait_cmd:
    lea     rcx, [r12 + SDHCI_REG_INT_STATUS]
    call    asm_mmio_read32
    test    eax, SDHCI_INT_ERROR
    jnz     .cmd_err
    test    eax, SDHCI_INT_CMD_COMPLETE
    jnz     .cmd_ok
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r15
    jb      .wait_cmd
.cmd_err:
    lea     rcx, [r12 + SDHCI_REG_INT_STATUS]
    mov     edx, 0xFFFF_FFFF
    call    asm_mmio_write32
    mov     eax, 2
    jmp     .out

.cmd_ok:
    lea     rcx, [r12 + SDHCI_REG_INT_STATUS]
    mov     edx, SDHCI_INT_CMD_COMPLETE
    call    asm_mmio_write32

    ; wait buffer read ready
    call    asm_tsc_read
    mov     rbx, rax
.wait_buf:
    lea     rcx, [r12 + SDHCI_REG_INT_STATUS]
    call    asm_mmio_read32
    test    eax, SDHCI_INT_ERROR
    jnz     .buf_err
    test    eax, SDHCI_INT_BUF_READ_READY
    jnz     .buf_ok
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r15
    jb      .wait_buf
.buf_err:
    lea     rcx, [r12 + SDHCI_REG_INT_STATUS]
    mov     edx, 0xFFFF_FFFF
    call    asm_mmio_write32
    mov     eax, 3
    jmp     .out

.buf_ok:
    lea     rcx, [r12 + SDHCI_REG_INT_STATUS]
    mov     edx, SDHCI_INT_BUF_READ_READY
    call    asm_mmio_write32

    ; read 512 bytes (128 dwords)
    xor     r11d, r11d
.copy_loop:
    lea     rcx, [r12 + SDHCI_REG_BUFFER_DATA]
    call    asm_mmio_read32
    mov     [r14 + r11*4], eax
    inc     r11d
    cmp     r11d, 128
    jb      .copy_loop

    ; wait transfer complete
    call    asm_tsc_read
    mov     rbx, rax
.wait_xfer:
    lea     rcx, [r12 + SDHCI_REG_INT_STATUS]
    call    asm_mmio_read32
    test    eax, SDHCI_INT_ERROR
    jnz     .xfer_err
    test    eax, SDHCI_INT_XFER_COMPLETE
    jnz     .xfer_ok
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r15
    jb      .wait_xfer
.xfer_err:
    lea     rcx, [r12 + SDHCI_REG_INT_STATUS]
    mov     edx, 0xFFFF_FFFF
    call    asm_mmio_write32
    mov     eax, 4
    jmp     .out

.xfer_ok:
    lea     rcx, [r12 + SDHCI_REG_INT_STATUS]
    mov     edx, SDHCI_INT_XFER_COMPLETE
    call    asm_mmio_write32

    xor     eax, eax

.out:
    add     rsp, 32
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret
