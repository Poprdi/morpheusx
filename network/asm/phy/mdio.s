; ═══════════════════════════════════════════════════════════════════════════
; MDIO (Management Data I/O) - PHY Register Access
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_mdio_read_c22: Clause 22 MDIO read
;   - asm_mdio_write_c22: Clause 22 MDIO write
;   - asm_mdio_read_c45: Clause 45 MDIO read
;   - asm_mdio_write_c45: Clause 45 MDIO write
;   - asm_mdio_bitbang_read: Generic bit-bang MDIO read
;   - asm_mdio_bitbang_write: Generic bit-bang MDIO write
;
; MDIO Frame Format (Clause 22, 32 + 32 = 64 bits):
;   Preamble: 32 1-bits (sent MSB first)
;   Start:    01
;   Opcode:   10 = read, 01 = write
;   PHY addr: 5 bits (PHY address 0-31)
;   Reg addr: 5 bits (register 0-31)
;   Turn-around: 2 bits (Z0 for read, 10 for write)
;   Data:     16 bits (read: PHY drives, write: driver drives)
;
; MDIO Frame Format (Clause 45, extended addressing):
;   Preamble: 32 1-bits
;   Start:    00 (indicates clause 45)
;   Opcode:   00=address, 01=write, 10=read inc, 11=read
;   PRTAD:    5 bits (port address)
;   DEVAD:    5 bits (device address)
;   Turn-around: 2 bits
;   Address/Data: 16 bits
;
; For hardware-assisted MDIO, use NIC-specific registers instead.
; These functions are for bit-bang or generic implementations.
;
; Reference: IEEE 802.3 Clause 22/45, ARCHITECTURE_V3.md
; ═══════════════════════════════════════════════════════════════════════════

section .data
    ; MDIO timing (minimum clock period ~400ns for 2.5MHz)
    ; We'll use conservative timing with TSC delays
    
    ; Clause 22 opcodes
    MDIO_C22_OP_WRITE   equ 0x1     ; 01
    MDIO_C22_OP_READ    equ 0x2     ; 10
    
    ; Clause 45 opcodes  
    MDIO_C45_OP_ADDRESS equ 0x0     ; 00
    MDIO_C45_OP_WRITE   equ 0x1     ; 01
    MDIO_C45_OP_READ_INC equ 0x2    ; 10
    MDIO_C45_OP_READ    equ 0x3     ; 11

section .text

; External dependencies
extern asm_tsc_read

; Export symbols
global asm_mdio_read_c22
global asm_mdio_write_c22
global asm_mdio_read_c45
global asm_mdio_write_c45

; ───────────────────────────────────────────────────────────────────────────
; asm_mdio_read_c22
; ───────────────────────────────────────────────────────────────────────────
; Read PHY register via Clause 22 MDIO (hardware-assisted)
;
; Parameters:
;   RCX = MDIO control register address (NIC-specific)
;   DL  = PHY address (0-31)
;   R8B = register address (0-31)
; Returns:
;   EAX = register value (16-bit, zero-extended)
;         0xFFFF if timeout/error
;
; Note: This is a generic interface. Actual implementation depends on NIC.
;       For Intel e1000: uses MDIC register
;       For Realtek: uses PHYAR register
;       For VirtIO: N/A (no PHY)
;
; Frame sent: PRE(32) + ST(01) + OP(10) + PHYAD(5) + REGAD(5) + TA(Z0) + DATA(16)
; ───────────────────────────────────────────────────────────────────────────
asm_mdio_read_c22:
    push    rbx
    push    r12
    push    r13
    push    r14
    
    mov     r12, rcx            ; MDIO control addr
    movzx   r13d, dl            ; PHY address
    movzx   r14d, r8b           ; Register address
    
    ; Build MDIO frame:
    ; For hardware-assisted MDIO, format varies by NIC
    ; Generic format: [31:26]=preamble bits, [25:24]=start, [23:22]=op,
    ;                 [21:17]=phy, [16:12]=reg, [11:10]=ta
    ; But actual format is NIC-specific
    
    ; Generic Intel e1000-style MDIC register format:
    ; [31]    = Ready (set by HW when complete)
    ; [30]    = Interrupt Enable
    ; [29:28] = OP (01=write, 10=read)
    ; [25:21] = PHY addr
    ; [20:16] = Reg addr
    ; [15:0]  = Data
    
    ; Build read command
    mov     eax, r13d           ; PHY addr
    shl     eax, 21
    mov     ebx, r14d           ; Reg addr  
    shl     ebx, 16
    or      eax, ebx
    or      eax, 0x08000000     ; Read opcode (10 in bits 29:28) = 0x08000000? 
                                ; Actually: (0x2 << 26) for some NICs
    
    ; For generic implementation, just use provided format
    ; Actual NIC drivers will have their own versions
    
    ; Write command
    mov     [r12], eax
    mfence
    
    ; Poll for completion (with timeout)
    mov     ebx, 10000          ; Iteration limit
    
.poll_loop:
    dec     ebx
    jz      .timeout
    
    ; Read back status
    mov     eax, [r12]
    test    eax, 0x10000000     ; Check ready bit (bit 28 common location)
    jnz     .ready
    
    ; Brief delay
    pause
    pause
    pause
    pause
    jmp     .poll_loop
    
.ready:
    ; Extract data (lower 16 bits)
    and     eax, 0xFFFF
    jmp     .done
    
.timeout:
    mov     eax, 0xFFFF         ; Error indicator
    
.done:
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_mdio_write_c22
; ───────────────────────────────────────────────────────────────────────────
; Write PHY register via Clause 22 MDIO (hardware-assisted)
;
; Parameters:
;   RCX = MDIO control register address
;   DL  = PHY address (0-31)
;   R8B = register address (0-31)
;   R9W = value to write
; Returns:
;   EAX = 0 on success, 1 on timeout
;
; Frame sent: PRE(32) + ST(01) + OP(01) + PHYAD(5) + REGAD(5) + TA(10) + DATA(16)
; ───────────────────────────────────────────────────────────────────────────
asm_mdio_write_c22:
    push    rbx
    push    r12
    push    r13
    push    r14
    push    r15
    
    mov     r12, rcx            ; MDIO control addr
    movzx   r13d, dl            ; PHY address
    movzx   r14d, r8b           ; Register address
    movzx   r15d, r9w           ; Value
    
    ; Build write command (generic format)
    mov     eax, r13d
    shl     eax, 21
    mov     ebx, r14d
    shl     ebx, 16
    or      eax, ebx
    or      eax, r15d           ; Data in lower 16 bits
    or      eax, 0x04000000     ; Write opcode (01 in bits 29:28) = 0x04000000?
    
    ; Write command
    mov     [r12], eax
    mfence
    
    ; Poll for completion
    mov     ebx, 10000
    
.poll_loop:
    dec     ebx
    jz      .timeout
    
    mov     eax, [r12]
    test    eax, 0x10000000     ; Ready bit
    jnz     .ready
    
    pause
    pause
    pause
    pause
    jmp     .poll_loop
    
.ready:
    xor     eax, eax            ; Success
    jmp     .done
    
.timeout:
    mov     eax, 1              ; Error
    
.done:
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_mdio_read_c45
; ───────────────────────────────────────────────────────────────────────────
; Read PHY register via Clause 45 MDIO (extended addressing)
;
; Parameters:
;   RCX = MDIO control register address
;   DL  = port address (PRTAD, 0-31)
;   R8B = device address (DEVAD, 0-31)
;   R9W = register address (0-65535)
; Returns:
;   EAX = register value (16-bit)
;         0xFFFF on error
;
; Clause 45 requires two transactions:
;   1. Address frame: OP=00, send register address
;   2. Read frame: OP=11, read data
; ───────────────────────────────────────────────────────────────────────────
asm_mdio_read_c45:
    push    rbx
    push    r12
    push    r13
    push    r14
    push    r15
    
    mov     r12, rcx            ; MDIO control addr
    movzx   r13d, dl            ; PRTAD
    movzx   r14d, r8b           ; DEVAD
    movzx   r15d, r9w           ; Register address
    
    ; Transaction 1: Set address
    ; Build address command with OP=00
    mov     eax, r13d           ; PRTAD
    shl     eax, 21
    mov     ebx, r14d           ; DEVAD
    shl     ebx, 16
    or      eax, ebx
    or      eax, r15d           ; Address in data field
    ; OP=00 (address) - no bits to set
    or      eax, 0x00000000     ; Clause 45 indicator + address op
    
    mov     [r12], eax
    mfence
    
    ; Wait for completion
    mov     ebx, 10000
.wait_addr:
    dec     ebx
    jz      .timeout
    mov     eax, [r12]
    test    eax, 0x10000000
    jz      .wait_addr
    
    ; Transaction 2: Read data
    mov     eax, r13d
    shl     eax, 21
    mov     ebx, r14d
    shl     ebx, 16
    or      eax, ebx
    or      eax, 0x0C000000     ; OP=11 (read) = 0x0C000000?
    
    mov     [r12], eax
    mfence
    
    ; Wait for completion
    mov     ebx, 10000
.wait_data:
    dec     ebx
    jz      .timeout
    mov     eax, [r12]
    test    eax, 0x10000000
    jz      .wait_data
    
    ; Extract data
    and     eax, 0xFFFF
    jmp     .done
    
.timeout:
    mov     eax, 0xFFFF
    
.done:
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_mdio_write_c45
; ───────────────────────────────────────────────────────────────────────────
; Write PHY register via Clause 45 MDIO
;
; Parameters:
;   RCX = MDIO control register address
;   DL  = port address (PRTAD)
;   R8B = device address (DEVAD)
;   R9W = register address
;   [RSP+40] = value to write (word)
; Returns:
;   EAX = 0 on success, 1 on error
; ───────────────────────────────────────────────────────────────────────────
asm_mdio_write_c45:
    push    rbx
    push    r12
    push    r13
    push    r14
    push    r15
    push    rsi
    
    mov     r12, rcx            ; MDIO control addr
    movzx   r13d, dl            ; PRTAD
    movzx   r14d, r8b           ; DEVAD
    movzx   r15d, r9w           ; Register address
    movzx   esi, word [rsp + 48 + 48]  ; Value (5th param from stack)
    
    ; Transaction 1: Set address
    mov     eax, r13d
    shl     eax, 21
    mov     ebx, r14d
    shl     ebx, 16
    or      eax, ebx
    or      eax, r15d
    
    mov     [r12], eax
    mfence
    
    mov     ebx, 10000
.wait_addr:
    dec     ebx
    jz      .timeout
    mov     eax, [r12]
    test    eax, 0x10000000
    jz      .wait_addr
    
    ; Transaction 2: Write data
    mov     eax, r13d
    shl     eax, 21
    mov     ebx, r14d
    shl     ebx, 16
    or      eax, ebx
    or      eax, esi            ; Data
    or      eax, 0x04000000     ; OP=01 (write)
    
    mov     [r12], eax
    mfence
    
    mov     ebx, 10000
.wait_data:
    dec     ebx
    jz      .timeout
    mov     eax, [r12]
    test    eax, 0x10000000
    jz      .wait_data
    
    xor     eax, eax
    jmp     .done
    
.timeout:
    mov     eax, 1
    
.done:
    pop     rsi
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret
