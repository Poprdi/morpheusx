; ═══════════════════════════════════════════════════════════════════════════
; Link Status Detection
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_link_poll_up: Poll for link up with timeout
;   - asm_link_get_speed: Get negotiated speed
;   - asm_link_get_duplex: Get duplex mode
;   - asm_link_get_info: Get combined link info
;
; Link Info struct layout:
;   +0x00: status    (u8)  - 0=down, 1=up
;   +0x01: speed     (u8)  - 0=10, 1=100, 2=1000, 3=10000
;   +0x02: duplex    (u8)  - 0=half, 1=full
;   +0x03: autoneg   (u8)  - 0=forced, 1=autoneg complete
;
; Reference: IEEE 802.3, ARCHITECTURE_V3.md - PHY layer
; ═══════════════════════════════════════════════════════════════════════════

section .data
    ; Speed constants
    SPEED_10        equ 0
    SPEED_100       equ 1
    SPEED_1000      equ 2
    SPEED_10000     equ 3
    
    ; BMSR bits (from mii.s)
    BMSR_LINK       equ 0x0004
    BMSR_ANEG_COMP  equ 0x0020
    
    ; ANLPAR bits
    ANLPAR_100FD    equ 0x0100
    ANLPAR_100HD    equ 0x0080
    ANLPAR_10FD     equ 0x0040
    ANLPAR_10HD     equ 0x0020
    
    ; MII register addresses
    MII_BMSR        equ 0x01
    MII_ANAR        equ 0x04
    MII_ANLPAR      equ 0x05
    MII_GBCR        equ 0x09    ; 1000BASE-T Control
    MII_GBSR        equ 0x0A    ; 1000BASE-T Status

section .text

; External: MDIO and MII functions
extern asm_mdio_read_c22
extern asm_tsc_read
extern asm_mii_read_bmsr

; Export symbols
global asm_link_poll_up
global asm_link_get_speed
global asm_link_get_duplex
global asm_link_get_info

; ───────────────────────────────────────────────────────────────────────────
; asm_link_poll_up
; ───────────────────────────────────────────────────────────────────────────
; Poll for link to come up with timeout (state machine friendly)
;
; Parameters:
;   RCX = MDIO control register address
;   DL  = PHY address
;   R8  = TSC frequency
;   R9  = timeout in seconds
; Returns:
;   EAX = 0 if link up
;         1 if timeout (link still down)
;         -1 on error
;
; Note: This is a bounded wait suitable for init sequences.
;       For runtime, use asm_link_get_info in poll loop.
; ───────────────────────────────────────────────────────────────────────────
asm_link_poll_up:
    push    rbx
    push    r12
    push    r13
    push    r14
    push    r15
    
    mov     r12, rcx            ; MDIO addr
    mov     r13b, dl            ; PHY addr
    mov     r14, r8             ; TSC freq
    mov     r15, r9             ; Timeout seconds
    
    ; Calculate timeout ticks
    imul    r15, r14            ; timeout_ticks = seconds * freq
    
    ; Get start time
    call    asm_tsc_read
    mov     rbx, rax
    
.poll_loop:
    ; Check timeout
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r15
    ja      .timeout
    
    ; Read BMSR (twice - first clears latched bits)
    mov     rcx, r12
    mov     dl, r13b
    mov     r8b, MII_BMSR
    call    asm_mdio_read_c22
    
    mov     rcx, r12
    mov     dl, r13b
    mov     r8b, MII_BMSR
    call    asm_mdio_read_c22
    
    ; Check for error
    cmp     ax, 0xFFFF
    je      .error
    
    ; Check link bit
    test    ax, BMSR_LINK
    jz      .poll_loop          ; Link down, keep polling
    
    ; Link up!
    xor     eax, eax
    jmp     .done
    
.timeout:
    mov     eax, 1
    jmp     .done
    
.error:
    mov     eax, -1
    
.done:
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_link_get_speed
; ───────────────────────────────────────────────────────────────────────────
; Get negotiated link speed
;
; Parameters:
;   RCX = MDIO control register address
;   DL  = PHY address
; Returns:
;   EAX = speed code (0=10, 1=100, 2=1000, 3=10000)
;         -1 on error or link down
;
; Determines speed from auto-negotiation result (ANAR & ANLPAR)
; ───────────────────────────────────────────────────────────────────────────
asm_link_get_speed:
    push    rbx
    push    r12
    push    r13
    
    mov     r12, rcx
    mov     r13b, dl
    
    ; Read ANAR (our capabilities)
    mov     rcx, r12
    mov     dl, r13b
    mov     r8b, MII_ANAR
    call    asm_mdio_read_c22
    mov     ebx, eax
    
    ; Read ANLPAR (partner capabilities)
    mov     rcx, r12
    mov     dl, r13b
    mov     r8b, MII_ANLPAR
    call    asm_mdio_read_c22
    
    ; Check for error
    cmp     ax, 0xFFFF
    je      .error
    
    ; Common capabilities = ANAR & ANLPAR
    and     eax, ebx
    
    ; Check 1000 Mbps (would need GBSR register)
    ; For now, check 100/10 only
    
    ; Check 100FD
    test    eax, ANLPAR_100FD
    jnz     .speed_100
    
    ; Check 100HD
    test    eax, ANLPAR_100HD
    jnz     .speed_100
    
    ; Check 10FD
    test    eax, ANLPAR_10FD
    jnz     .speed_10
    
    ; Check 10HD
    test    eax, ANLPAR_10HD
    jnz     .speed_10
    
    ; No common speed (shouldn't happen)
    jmp     .error
    
.speed_100:
    mov     eax, SPEED_100
    jmp     .done
    
.speed_10:
    mov     eax, SPEED_10
    jmp     .done
    
.error:
    mov     eax, -1
    
.done:
    pop     r13
    pop     r12
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_link_get_duplex
; ───────────────────────────────────────────────────────────────────────────
; Get negotiated duplex mode
;
; Parameters:
;   RCX = MDIO control register address
;   DL  = PHY address
; Returns:
;   EAX = 0 for half duplex, 1 for full duplex, -1 on error
; ───────────────────────────────────────────────────────────────────────────
asm_link_get_duplex:
    push    rbx
    push    r12
    push    r13
    
    mov     r12, rcx
    mov     r13b, dl
    
    ; Read ANAR
    mov     rcx, r12
    mov     dl, r13b
    mov     r8b, MII_ANAR
    call    asm_mdio_read_c22
    mov     ebx, eax
    
    ; Read ANLPAR
    mov     rcx, r12
    mov     dl, r13b
    mov     r8b, MII_ANLPAR
    call    asm_mdio_read_c22
    
    cmp     ax, 0xFFFF
    je      .error
    
    ; Common capabilities
    and     eax, ebx
    
    ; Check for full duplex (100FD or 10FD)
    test    eax, ANLPAR_100FD | ANLPAR_10FD
    jnz     .full_duplex
    
    ; Half duplex
    xor     eax, eax
    jmp     .done
    
.full_duplex:
    mov     eax, 1
    jmp     .done
    
.error:
    mov     eax, -1
    
.done:
    pop     r13
    pop     r12
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_link_get_info
; ───────────────────────────────────────────────────────────────────────────
; Get combined link information
;
; Parameters:
;   RCX = MDIO control register address
;   DL  = PHY address
;   R8  = pointer to LinkInfo struct (4 bytes)
; Returns:
;   EAX = 0 on success, -1 on error
;
; LinkInfo struct:
;   [0] status:  0=down, 1=up
;   [1] speed:   0=10, 1=100, 2=1000, 3=10000
;   [2] duplex:  0=half, 1=full
;   [3] autoneg: 0=forced, 1=autoneg complete
; ───────────────────────────────────────────────────────────────────────────
asm_link_get_info:
    push    rbx
    push    r12
    push    r13
    push    r14
    
    mov     r12, rcx            ; MDIO addr
    mov     r13b, dl            ; PHY addr
    mov     r14, r8             ; LinkInfo ptr
    
    ; Clear struct
    mov     dword [r14], 0
    
    ; Read BMSR (twice for current status)
    mov     rcx, r12
    mov     dl, r13b
    mov     r8b, MII_BMSR
    call    asm_mdio_read_c22
    
    mov     rcx, r12
    mov     dl, r13b
    mov     r8b, MII_BMSR
    call    asm_mdio_read_c22
    mov     ebx, eax
    
    cmp     ax, 0xFFFF
    je      .error
    
    ; Check link
    test    bx, BMSR_LINK
    jz      .link_down
    mov     byte [r14 + 0], 1   ; Link up
    
    ; Check autoneg complete
    test    bx, BMSR_ANEG_COMP
    jz      .no_autoneg
    mov     byte [r14 + 3], 1   ; Autoneg complete
    
    ; Get speed
    mov     rcx, r12
    mov     dl, r13b
    call    asm_link_get_speed
    cmp     eax, 0
    jl      .error
    mov     [r14 + 1], al
    
    ; Get duplex
    mov     rcx, r12
    mov     dl, r13b
    call    asm_link_get_duplex
    cmp     eax, 0
    jl      .error
    mov     [r14 + 2], al
    
.no_autoneg:
.link_down:
    xor     eax, eax
    jmp     .done
    
.error:
    mov     eax, -1
    
.done:
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret
