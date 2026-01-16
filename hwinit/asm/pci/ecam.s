; ═══════════════════════════════════════════════════════════════════════════
; PCIe ECAM (Enhanced Configuration Access Mechanism)
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_pci_ecam_addr: Calculate ECAM address
;   - asm_pci_ecam_read32: Read 32-bit from PCIe config space
;   - asm_pci_ecam_write32: Write 32-bit to PCIe config space
;   - asm_pci_ecam_read16: Read 16-bit from PCIe config space
;   - asm_pci_ecam_write16: Write 16-bit to PCIe config space
;   - asm_pci_ecam_read8: Read 8-bit from PCIe config space
;   - asm_pci_ecam_write8: Write 8-bit to PCIe config space
;
; ECAM Address Format (memory-mapped, 4KB per function):
;   Address = ECAM_Base + (Bus << 20) + (Device << 15) + (Function << 12) + Register
;
; ECAM provides access to full 4KB config space per function (vs 256 bytes legacy)
; Requires knowing ECAM base from ACPI MCFG table or UEFI.
;
; Reference: PCIe Base Spec 4.0, ARCHITECTURE_V3.md
; ═══════════════════════════════════════════════════════════════════════════

section .text

; Export symbols
global asm_pci_ecam_addr
global asm_pci_ecam_read32
global asm_pci_ecam_write32
global asm_pci_ecam_read16
global asm_pci_ecam_write16
global asm_pci_ecam_read8
global asm_pci_ecam_write8

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_ecam_addr
; ───────────────────────────────────────────────────────────────────────────
; Calculate ECAM memory address for a config space access
;
; Parameters:
;   RCX = ECAM base address (from MCFG table)
;   DL  = bus number (0-255)
;   R8B = device number (0-31)
;   R9B = function number (0-7)
;   [RSP+40] = register offset (0-4095) - but for simplicity we'll use R10
; Returns:
;   RAX = memory address for config access
; Clobbers: R10, R11
;
; Simplified signature for ease of use:
;   RCX = ECAM base
;   RDX = bus (low 8 bits)
;   R8  = device (low 5 bits)  
;   R9  = function (low 3 bits)
;   R10 = register offset (low 12 bits)
; ───────────────────────────────────────────────────────────────────────────
asm_pci_ecam_addr:
    ; Start with ECAM base
    mov     rax, rcx
    
    ; Add (bus << 20)
    movzx   r11, dl             ; Bus number
    shl     r11, 20
    add     rax, r11
    
    ; Add (device << 15)
    movzx   r11, r8b            ; Device number
    and     r11, 0x1F           ; Mask to 5 bits
    shl     r11, 15
    add     rax, r11
    
    ; Add (function << 12)
    movzx   r11, r9b            ; Function number
    and     r11, 0x07           ; Mask to 3 bits
    shl     r11, 12
    add     rax, r11
    
    ; Add register offset
    movzx   r11, r10w           ; Register offset (12 bits max)
    and     r11, 0x0FFF         ; Mask to 12 bits
    add     rax, r11
    
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_ecam_read32
; ───────────────────────────────────────────────────────────────────────────
; Read 32-bit value from PCIe config space via ECAM
;
; Parameters:
;   RCX = ECAM base address
;   DL  = bus number
;   R8B = device number
;   R9B = function number
;   R10W = register offset (must be dword aligned)
; Returns:
;   EAX = 32-bit config value
; ───────────────────────────────────────────────────────────────────────────
asm_pci_ecam_read32:
    ; Calculate address
    call    asm_pci_ecam_addr   ; RAX = memory address
    
    ; MMIO read
    mov     eax, [rax]
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_ecam_write32
; ───────────────────────────────────────────────────────────────────────────
; Write 32-bit value to PCIe config space via ECAM
;
; Parameters:
;   RCX = ECAM base address
;   DL  = bus number
;   R8B = device number
;   R9B = function number
;   R10W = register offset (must be dword aligned)
;   R11D = value to write
; Returns: None
; ───────────────────────────────────────────────────────────────────────────
asm_pci_ecam_write32:
    push    r11                 ; Save value
    
    call    asm_pci_ecam_addr   ; RAX = memory address
    
    pop     r11
    mov     [rax], r11d         ; MMIO write
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_ecam_read16
; ───────────────────────────────────────────────────────────────────────────
; Read 16-bit value from PCIe config space via ECAM
;
; Parameters: Same as read32, R10W = register offset (word aligned)
; Returns: AX = 16-bit value (zero-extended)
; ───────────────────────────────────────────────────────────────────────────
asm_pci_ecam_read16:
    call    asm_pci_ecam_addr
    movzx   eax, word [rax]
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_ecam_write16
; ───────────────────────────────────────────────────────────────────────────
; Write 16-bit value to PCIe config space via ECAM
;
; Parameters: Same as write32, R11W = value
; Returns: None
; ───────────────────────────────────────────────────────────────────────────
asm_pci_ecam_write16:
    push    r11
    call    asm_pci_ecam_addr
    pop     r11
    mov     [rax], r11w
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_ecam_read8
; ───────────────────────────────────────────────────────────────────────────
; Read 8-bit value from PCIe config space via ECAM
;
; Parameters: Same as read32, R10W = register offset (any alignment)
; Returns: AL = 8-bit value (zero-extended)
; ───────────────────────────────────────────────────────────────────────────
asm_pci_ecam_read8:
    call    asm_pci_ecam_addr
    movzx   eax, byte [rax]
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_ecam_write8
; ───────────────────────────────────────────────────────────────────────────
; Write 8-bit value to PCIe config space via ECAM
;
; Parameters: Same as write32, R11B = value
; Returns: None
; ───────────────────────────────────────────────────────────────────────────
asm_pci_ecam_write8:
    push    r11
    call    asm_pci_ecam_addr
    pop     r11
    mov     [rax], r11b
    ret
