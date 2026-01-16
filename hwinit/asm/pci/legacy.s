; ═══════════════════════════════════════════════════════════════════════════
; PCI Legacy Configuration Space Access (CF8/CFC)
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_pci_cfg_read32: Read 32-bit from PCI config space
;   - asm_pci_cfg_write32: Write 32-bit to PCI config space
;   - asm_pci_cfg_read16: Read 16-bit from PCI config space
;   - asm_pci_cfg_write16: Write 16-bit to PCI config space
;   - asm_pci_cfg_read8: Read 8-bit from PCI config space
;   - asm_pci_cfg_write8: Write 8-bit to PCI config space
;
; PCI Config Address Format (port CF8h):
;   Bit 31:    Enable bit (must be 1)
;   Bits 30-24: Reserved (0)
;   Bits 23-16: Bus number (0-255)
;   Bits 15-11: Device number (0-31)
;   Bits 10-8:  Function number (0-7)
;   Bits 7-2:   Register number (dword aligned)
;   Bits 1-0:   Must be 00 (dword alignment)
;
; Port CFCh: Data port (read/write config data)
;
; Reference: PCI Local Bus Spec 3.0, ARCHITECTURE_V3.md
; ═══════════════════════════════════════════════════════════════════════════

section .text

; Export symbols
global asm_pci_cfg_read32
global asm_pci_cfg_write32
global asm_pci_cfg_read16
global asm_pci_cfg_write16
global asm_pci_cfg_read8
global asm_pci_cfg_write8
global asm_pci_make_addr

; PCI configuration ports
%define PCI_CONFIG_ADDR 0x0CF8
%define PCI_CONFIG_DATA 0x0CFC

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_make_addr
; ───────────────────────────────────────────────────────────────────────────
; Build PCI configuration address from bus/device/function/register
;
; Parameters:
;   CL  = bus number (0-255)
;   DL  = device number (0-31) 
;   R8B = function number (0-7)
;   R9B = register offset (0-255, will be aligned to dword)
; Returns:
;   EAX = PCI config address with enable bit set
; Clobbers: None
; ───────────────────────────────────────────────────────────────────────────
asm_pci_make_addr:
    ; Build address: 0x80000000 | (bus << 16) | (dev << 11) | (func << 8) | (reg & 0xFC)
    movzx   eax, cl             ; EAX = bus
    shl     eax, 16             ; EAX = bus << 16
    
    movzx   r10d, dl            ; R10D = device
    shl     r10d, 11            ; R10D = dev << 11
    or      eax, r10d
    
    movzx   r10d, r8b           ; R10D = function
    shl     r10d, 8             ; R10D = func << 8
    or      eax, r10d
    
    movzx   r10d, r9b           ; R10D = register
    and     r10d, 0xFC          ; Align to dword boundary
    or      eax, r10d
    
    or      eax, 0x80000000     ; Set enable bit
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_cfg_read32
; ───────────────────────────────────────────────────────────────────────────
; Read 32-bit value from PCI configuration space
;
; Parameters:
;   CL  = bus number (0-255)
;   DL  = device number (0-31)
;   R8B = function number (0-7)
;   R9B = register offset (must be dword aligned: 0, 4, 8, ...)
; Returns:
;   EAX = 32-bit config value
; Clobbers: RDX
; ───────────────────────────────────────────────────────────────────────────
asm_pci_cfg_read32:
    ; Save parameters before building address
    push    rcx
    push    r8
    push    r9
    
    ; Build PCI address in EAX
    call    asm_pci_make_addr
    
    ; Write address to CF8
    mov     dx, PCI_CONFIG_ADDR
    out     dx, eax
    
    ; Read data from CFC
    mov     dx, PCI_CONFIG_DATA
    in      eax, dx
    
    pop     r9
    pop     r8
    pop     rcx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_cfg_write32
; ───────────────────────────────────────────────────────────────────────────
; Write 32-bit value to PCI configuration space
;
; Parameters:
;   CL  = bus number (0-255)
;   DL  = device number (0-31)
;   R8B = function number (0-7)
;   R9B = register offset (must be dword aligned)
;   [RSP+40] = value to write (5th parameter, on stack per MS x64 ABI)
; Returns: None
; Clobbers: RAX, RDX
;
; Note: MS x64 ABI passes first 4 params in RCX,RDX,R8,R9
;       5th+ params go on stack at RSP+32 (after shadow space)
;       But we have pushed 3 regs, so adjust offset
; ───────────────────────────────────────────────────────────────────────────
asm_pci_cfg_write32:
    push    rcx
    push    r8
    push    r9
    push    rbx                 ; Save for value
    
    ; Get value from stack (5th param)
    ; Stack: ret_addr, rcx, r8, r9, rbx = 5*8 = 40 bytes
    ; Caller's shadow space + 5th param at RSP+40+32 = RSP+72 before our pushes
    ; After 4 pushes (32 bytes): RSP+72+32 = RSP+104... 
    ; Actually in MS x64 ABI, 5th param is at RSP+40 relative to entry
    ; After 4 pushes, it's at RSP+40+32 = RSP+72
    mov     ebx, [rsp + 40 + 32] ; Value to write
    
    ; Build PCI address in EAX
    call    asm_pci_make_addr
    
    ; Write address to CF8
    mov     dx, PCI_CONFIG_ADDR
    out     dx, eax
    
    ; Write data to CFC
    mov     dx, PCI_CONFIG_DATA
    mov     eax, ebx
    out     dx, eax
    
    pop     rbx
    pop     r9
    pop     r8
    pop     rcx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_cfg_read16
; ───────────────────────────────────────────────────────────────────────────
; Read 16-bit value from PCI configuration space
;
; Parameters:
;   CL  = bus number
;   DL  = device number
;   R8B = function number
;   R9B = register offset (word aligned: 0, 2, 4, ...)
; Returns:
;   AX = 16-bit config value (zero-extended to EAX)
; Clobbers: RDX
; ───────────────────────────────────────────────────────────────────────────
asm_pci_cfg_read16:
    push    rcx
    push    r8
    push    r9
    
    ; Save byte offset within dword
    movzx   r10d, r9b
    and     r10d, 2             ; Offset 0 or 2 within dword
    push    r10
    
    ; Build address (aligned to dword)
    call    asm_pci_make_addr
    
    ; Write address to CF8
    mov     dx, PCI_CONFIG_ADDR
    out     dx, eax
    
    ; Read dword from CFC
    mov     dx, PCI_CONFIG_DATA
    in      eax, dx
    
    ; Extract correct 16-bit portion
    pop     r10
    test    r10d, r10d
    jz      .low_word
    shr     eax, 16             ; Get high word
.low_word:
    movzx   eax, ax             ; Zero-extend result
    
    pop     r9
    pop     r8
    pop     rcx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_cfg_write16
; ───────────────────────────────────────────────────────────────────────────
; Write 16-bit value to PCI configuration space
;
; Uses read-modify-write to preserve other 16 bits of dword
;
; Parameters:
;   CL  = bus number
;   DL  = device number
;   R8B = function number
;   R9B = register offset (word aligned)
;   R10W = value to write (passed in R10 since 5th param on stack is awkward)
;
; Note: Caller must put value in R10 before calling
; Returns: None
; Clobbers: RAX, RDX
; ───────────────────────────────────────────────────────────────────────────
asm_pci_cfg_write16:
    push    rcx
    push    r8
    push    r9
    push    rbx
    push    r11
    
    ; Save value and byte offset
    movzx   r11d, r10w          ; Value to write
    movzx   ebx, r9b
    and     ebx, 2              ; Offset within dword (0 or 2)
    
    ; First read the current dword
    call    asm_pci_make_addr
    push    rax                 ; Save address
    
    mov     dx, PCI_CONFIG_ADDR
    out     dx, eax
    mov     dx, PCI_CONFIG_DATA
    in      eax, dx             ; EAX = current dword
    
    ; Modify the appropriate 16 bits
    test    ebx, ebx
    jz      .write_low
    ; Write high word: clear high 16, shift value up
    and     eax, 0x0000FFFF
    shl     r11d, 16
    or      eax, r11d
    jmp     .do_write
.write_low:
    ; Write low word: clear low 16, OR in value
    and     eax, 0xFFFF0000
    or      eax, r11d
    
.do_write:
    ; Write back modified dword
    pop     rbx                 ; Restore address to RBX
    push    rax                 ; Save modified value
    
    mov     eax, ebx
    mov     dx, PCI_CONFIG_ADDR
    out     dx, eax
    
    pop     rax
    mov     dx, PCI_CONFIG_DATA
    out     dx, eax
    
    pop     r11
    pop     rbx
    pop     r9
    pop     r8
    pop     rcx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_cfg_read8
; ───────────────────────────────────────────────────────────────────────────
; Read 8-bit value from PCI configuration space
;
; Parameters:
;   CL  = bus number
;   DL  = device number
;   R8B = function number
;   R9B = register offset (any alignment)
; Returns:
;   AL = 8-bit config value (zero-extended to EAX)
; ───────────────────────────────────────────────────────────────────────────
asm_pci_cfg_read8:
    push    rcx
    push    r8
    push    r9
    
    ; Save byte offset within dword
    movzx   r10d, r9b
    and     r10d, 3             ; Offset 0-3 within dword
    push    r10
    
    ; Build address (aligned to dword)
    call    asm_pci_make_addr
    
    mov     dx, PCI_CONFIG_ADDR
    out     dx, eax
    
    mov     dx, PCI_CONFIG_DATA
    in      eax, dx
    
    ; Shift to get correct byte
    pop     rcx                 ; Byte offset
    shl     ecx, 3              ; Multiply by 8 for bit offset
    shr     eax, cl
    movzx   eax, al
    
    pop     r9
    pop     r8
    pop     rcx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_cfg_write8
; ───────────────────────────────────────────────────────────────────────────
; Write 8-bit value to PCI configuration space (read-modify-write)
;
; Parameters:
;   CL  = bus number
;   DL  = device number
;   R8B = function number
;   R9B = register offset
;   R10B = value to write
; Returns: None
; ───────────────────────────────────────────────────────────────────────────
asm_pci_cfg_write8:
    push    rcx
    push    r8
    push    r9
    push    rbx
    push    r11
    push    r12
    
    ; Save value and byte offset
    movzx   r11d, r10b          ; Value to write
    movzx   r12d, r9b
    and     r12d, 3             ; Byte offset within dword
    
    ; Read current dword
    call    asm_pci_make_addr
    push    rax                 ; Save address
    
    mov     dx, PCI_CONFIG_ADDR
    out     dx, eax
    mov     dx, PCI_CONFIG_DATA
    in      eax, dx             ; EAX = current dword
    
    ; Create mask and modify
    mov     ebx, 0xFF
    mov     ecx, r12d
    shl     ecx, 3              ; Bit offset
    shl     ebx, cl             ; Mask for byte position
    not     ebx                 ; Invert mask
    and     eax, ebx            ; Clear target byte
    shl     r11d, cl            ; Position new value
    or      eax, r11d           ; Insert new byte
    
    ; Write back
    pop     rbx                 ; Address
    push    rax
    mov     eax, ebx
    mov     dx, PCI_CONFIG_ADDR
    out     dx, eax
    pop     rax
    mov     dx, PCI_CONFIG_DATA
    out     dx, eax
    
    pop     r12
    pop     r11
    pop     rbx
    pop     r9
    pop     r8
    pop     rcx
    ret
