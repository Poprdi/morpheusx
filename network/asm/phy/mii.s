; ═══════════════════════════════════════════════════════════════════════════
; MII (Media Independent Interface) Register Definitions
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_mii_read_bmcr: Read Basic Mode Control Register
;   - asm_mii_write_bmcr: Write Basic Mode Control Register
;   - asm_mii_read_bmsr: Read Basic Mode Status Register
;   - asm_mii_read_phyid: Read PHY ID (combined reg 2+3)
;   - asm_mii_read_anar: Read Auto-Neg Advertisement
;   - asm_mii_read_anlpar: Read Auto-Neg Link Partner Ability
;   - asm_mii_reset_phy: Reset PHY and wait for completion
;   - asm_mii_restart_autoneg: Restart auto-negotiation
;
; Standard MII Registers (Clause 22):
;   0 - BMCR: Basic Mode Control Register
;       [15] Reset
;       [14] Loopback
;       [13] Speed Select (LSB)
;       [12] Auto-Neg Enable
;       [11] Power Down
;       [10] Isolate
;       [9]  Restart Auto-Neg
;       [8]  Duplex Mode
;       [6]  Speed Select (MSB) - for 1000Mbps
;
;   1 - BMSR: Basic Mode Status Register
;       [15] 100BASE-T4
;       [14] 100BASE-X Full
;       [13] 100BASE-X Half
;       [12] 10 Mbps Full
;       [11] 10 Mbps Half
;       [5]  Auto-Neg Complete
;       [4]  Remote Fault
;       [3]  Auto-Neg Ability
;       [2]  Link Status
;       [1]  Jabber Detect
;       [0]  Extended Capability
;
;   2 - PHYID1: OUI bits 3-18
;   3 - PHYID2: OUI bits 19-24, model, revision
;   4 - ANAR:  Auto-Neg Advertisement
;   5 - ANLPAR: Auto-Neg Link Partner Ability
;
; Reference: IEEE 802.3 Clause 22, ARCHITECTURE_V3.md
; ═══════════════════════════════════════════════════════════════════════════

section .data
    ; MII Register addresses
    MII_BMCR        equ 0x00    ; Basic Mode Control
    MII_BMSR        equ 0x01    ; Basic Mode Status
    MII_PHYID1      equ 0x02    ; PHY ID High
    MII_PHYID2      equ 0x03    ; PHY ID Low
    MII_ANAR        equ 0x04    ; Auto-Neg Advertisement
    MII_ANLPAR      equ 0x05    ; Auto-Neg Link Partner Ability
    MII_ANER        equ 0x06    ; Auto-Neg Expansion
    MII_ANNPTR      equ 0x07    ; Auto-Neg Next Page TX
    MII_ANNPRR      equ 0x08    ; Auto-Neg Next Page RX
    
    ; BMCR bit definitions
    BMCR_RESET      equ 0x8000  ; PHY Reset
    BMCR_LOOPBACK   equ 0x4000  ; Loopback mode
    BMCR_SPEED_100  equ 0x2000  ; 100 Mbps
    BMCR_ANEG_EN    equ 0x1000  ; Auto-Neg Enable
    BMCR_PDOWN      equ 0x0800  ; Power Down
    BMCR_ISOLATE    equ 0x0400  ; Isolate PHY
    BMCR_ANEG_RST   equ 0x0200  ; Restart Auto-Neg
    BMCR_DUPLEX     equ 0x0100  ; Full Duplex
    BMCR_SPEED_1000 equ 0x0040  ; 1000 Mbps (bit 6)
    
    ; BMSR bit definitions
    BMSR_100T4      equ 0x8000  ; 100BASE-T4 capable
    BMSR_100FD      equ 0x4000  ; 100BASE-X Full Duplex
    BMSR_100HD      equ 0x2000  ; 100BASE-X Half Duplex
    BMSR_10FD       equ 0x1000  ; 10 Mbps Full Duplex
    BMSR_10HD       equ 0x0800  ; 10 Mbps Half Duplex
    BMSR_ANEG_COMP  equ 0x0020  ; Auto-Neg Complete
    BMSR_RFAULT     equ 0x0010  ; Remote Fault
    BMSR_ANEG_CAP   equ 0x0008  ; Auto-Neg Capable
    BMSR_LINK       equ 0x0004  ; Link Status
    BMSR_JABBER     equ 0x0002  ; Jabber Detect
    BMSR_EXTCAP     equ 0x0001  ; Extended Capability
    
    ; ANAR/ANLPAR bit definitions
    ANAR_100FD      equ 0x0100  ; 100BASE-TX Full
    ANAR_100HD      equ 0x0080  ; 100BASE-TX Half
    ANAR_10FD       equ 0x0040  ; 10BASE-T Full
    ANAR_10HD       equ 0x0020  ; 10BASE-T Half
    ANAR_SELECTOR   equ 0x001F  ; IEEE 802.3 selector

section .text

; External: MDIO functions
extern asm_mdio_read_c22
extern asm_mdio_write_c22
extern asm_tsc_read

; Export symbols
global asm_mii_read_bmcr
global asm_mii_write_bmcr
global asm_mii_read_bmsr
global asm_mii_read_phyid
global asm_mii_read_anar
global asm_mii_read_anlpar
global asm_mii_reset_phy
global asm_mii_restart_autoneg
global asm_mii_get_link_status

; ───────────────────────────────────────────────────────────────────────────
; asm_mii_read_bmcr
; ───────────────────────────────────────────────────────────────────────────
; Read Basic Mode Control Register (reg 0)
;
; Parameters:
;   RCX = MDIO control register address
;   DL  = PHY address
; Returns:
;   EAX = BMCR value (16-bit)
; ───────────────────────────────────────────────────────────────────────────
asm_mii_read_bmcr:
    mov     r8b, MII_BMCR
    jmp     asm_mdio_read_c22

; ───────────────────────────────────────────────────────────────────────────
; asm_mii_write_bmcr
; ───────────────────────────────────────────────────────────────────────────
; Write Basic Mode Control Register
;
; Parameters:
;   RCX = MDIO control register address
;   DL  = PHY address
;   R8W = value to write
; Returns:
;   EAX = 0 on success, 1 on error
; ───────────────────────────────────────────────────────────────────────────
asm_mii_write_bmcr:
    mov     r9w, r8w            ; Move value to R9
    mov     r8b, MII_BMCR       ; Register
    jmp     asm_mdio_write_c22

; ───────────────────────────────────────────────────────────────────────────
; asm_mii_read_bmsr
; ───────────────────────────────────────────────────────────────────────────
; Read Basic Mode Status Register (reg 1)
;
; Parameters:
;   RCX = MDIO control register address
;   DL  = PHY address
; Returns:
;   EAX = BMSR value (16-bit)
; ───────────────────────────────────────────────────────────────────────────
asm_mii_read_bmsr:
    mov     r8b, MII_BMSR
    jmp     asm_mdio_read_c22

; ───────────────────────────────────────────────────────────────────────────
; asm_mii_read_phyid
; ───────────────────────────────────────────────────────────────────────────
; Read combined PHY ID from registers 2 and 3
;
; Parameters:
;   RCX = MDIO control register address
;   DL  = PHY address
; Returns:
;   EAX = PHY ID (PHYID1 << 16 | PHYID2)
; ───────────────────────────────────────────────────────────────────────────
asm_mii_read_phyid:
    push    rbx
    push    r12
    push    r13
    
    mov     r12, rcx
    mov     r13b, dl
    
    ; Read PHYID1
    mov     rcx, r12
    mov     dl, r13b
    mov     r8b, MII_PHYID1
    call    asm_mdio_read_c22
    mov     ebx, eax
    shl     ebx, 16             ; PHYID1 in high 16 bits
    
    ; Read PHYID2
    mov     rcx, r12
    mov     dl, r13b
    mov     r8b, MII_PHYID2
    call    asm_mdio_read_c22
    
    ; Combine
    or      eax, ebx
    
    pop     r13
    pop     r12
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_mii_read_anar
; ───────────────────────────────────────────────────────────────────────────
; Read Auto-Negotiation Advertisement Register
; ───────────────────────────────────────────────────────────────────────────
asm_mii_read_anar:
    mov     r8b, MII_ANAR
    jmp     asm_mdio_read_c22

; ───────────────────────────────────────────────────────────────────────────
; asm_mii_read_anlpar
; ───────────────────────────────────────────────────────────────────────────
; Read Auto-Negotiation Link Partner Ability Register
; ───────────────────────────────────────────────────────────────────────────
asm_mii_read_anlpar:
    mov     r8b, MII_ANLPAR
    jmp     asm_mdio_read_c22

; ───────────────────────────────────────────────────────────────────────────
; asm_mii_reset_phy
; ───────────────────────────────────────────────────────────────────────────
; Reset PHY and wait for reset to complete
;
; Parameters:
;   RCX = MDIO control register address
;   DL  = PHY address
;   R8  = TSC frequency (for timeout)
; Returns:
;   EAX = 0 on success, 1 on timeout
;
; PHY reset bit auto-clears when reset complete (typically <500ms)
; ───────────────────────────────────────────────────────────────────────────
asm_mii_reset_phy:
    push    rbx
    push    r12
    push    r13
    push    r14
    
    mov     r12, rcx            ; MDIO addr
    mov     r13b, dl            ; PHY addr
    mov     r14, r8             ; TSC freq
    
    ; Write BMCR with reset bit
    mov     rcx, r12
    mov     dl, r13b
    mov     r8w, BMCR_RESET
    mov     r9w, r8w
    mov     r8b, MII_BMCR
    call    asm_mdio_write_c22
    
    ; Get start time
    call    asm_tsc_read
    mov     rbx, rax
    
    ; Calculate timeout (500ms)
    mov     rax, r14
    shr     rax, 1              ; / 2 = 500ms
    mov     r14, rax
    
.poll_loop:
    ; Check timeout
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r14
    ja      .timeout
    
    ; Read BMCR
    mov     rcx, r12
    mov     dl, r13b
    mov     r8b, MII_BMCR
    call    asm_mdio_read_c22
    
    ; Check if reset bit cleared
    test    ax, BMCR_RESET
    jnz     .poll_loop
    
    xor     eax, eax
    jmp     .done
    
.timeout:
    mov     eax, 1
    
.done:
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_mii_restart_autoneg
; ───────────────────────────────────────────────────────────────────────────
; Restart auto-negotiation
;
; Parameters:
;   RCX = MDIO control register address
;   DL  = PHY address
; Returns:
;   EAX = 0 on success, 1 on error
; ───────────────────────────────────────────────────────────────────────────
asm_mii_restart_autoneg:
    push    rbx
    push    r12
    push    r13
    
    mov     r12, rcx
    mov     r13b, dl
    
    ; Read current BMCR
    mov     rcx, r12
    mov     dl, r13b
    mov     r8b, MII_BMCR
    call    asm_mdio_read_c22
    mov     ebx, eax
    
    ; Set ANEG_EN and ANEG_RST bits
    or      bx, BMCR_ANEG_EN | BMCR_ANEG_RST
    
    ; Write back
    mov     rcx, r12
    mov     dl, r13b
    mov     r9w, bx
    mov     r8b, MII_BMCR
    call    asm_mdio_write_c22
    
    pop     r13
    pop     r12
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_mii_get_link_status
; ───────────────────────────────────────────────────────────────────────────
; Get link status from BMSR
;
; Parameters:
;   RCX = MDIO control register address
;   DL  = PHY address
; Returns:
;   EAX = 1 if link up, 0 if link down, -1 on error
;
; Note: BMSR link bit is latching-low. Read twice for current status.
; ───────────────────────────────────────────────────────────────────────────
asm_mii_get_link_status:
    push    r12
    push    r13
    
    mov     r12, rcx
    mov     r13b, dl
    
    ; First read clears latched status
    mov     rcx, r12
    mov     dl, r13b
    mov     r8b, MII_BMSR
    call    asm_mdio_read_c22
    
    ; Second read gets current status
    mov     rcx, r12
    mov     dl, r13b
    mov     r8b, MII_BMSR
    call    asm_mdio_read_c22
    
    ; Check for read error
    cmp     ax, 0xFFFF
    je      .error
    
    ; Extract link bit
    test    ax, BMSR_LINK
    jz      .link_down
    
    mov     eax, 1
    jmp     .done
    
.link_down:
    xor     eax, eax
    jmp     .done
    
.error:
    mov     eax, -1
    
.done:
    pop     r13
    pop     r12
    ret
