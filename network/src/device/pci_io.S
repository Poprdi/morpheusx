; PCI Configuration Space I/O Port Access
; Standalone assembly for reliable I/O port operations
;
; UEFI uses Microsoft x64 ABI (NOT System V):
;   - Arguments: RCX, RDX, R8, R9 (then stack)
;   - Return: RAX
;   - Caller-saved: RAX, RCX, RDX, R8, R9, R10, R11
;   - Callee-saved: RBX, RBP, RDI, RSI, R12-R15

section .text

; =============================================================================
; pci_config_read32 - Read 32-bit value from PCI config space
;
; Microsoft x64 ABI Arguments:
;   RCX: bus (0-255)
;   RDX: device (0-31)  
;   R8:  function (0-7)
;   R9:  offset (0-255, must be 4-byte aligned)
;
; Returns:
;   EAX: 32-bit value read from config space (0xFFFFFFFF if no device)
;
; The CONFIG_ADDRESS format (port 0xCF8):
;   Bit 31:     Enable bit (must be 1)
;   Bits 30-24: Reserved
;   Bits 23-16: Bus number
;   Bits 15-11: Device number
;   Bits 10-8:  Function number
;   Bits 7-2:   Register offset (aligned)
;   Bits 1-0:   Always 0
; =============================================================================
global pci_config_read32
pci_config_read32:
    push rbx                    ; Save callee-saved register
    
    ; Build CONFIG_ADDRESS in EAX
    ; Start with enable bit
    mov eax, 0x80000000
    
    ; Add bus number (RCX << 16)
    mov ebx, ecx
    shl ebx, 16
    or eax, ebx
    
    ; Add device number (RDX << 11)
    mov ebx, edx
    shl ebx, 11
    or eax, ebx
    
    ; Add function number (R8 << 8)
    mov ebx, r8d
    shl ebx, 8
    or eax, ebx
    
    ; Add offset (R9 & 0xFC)
    mov ebx, r9d
    and ebx, 0xFC
    or eax, ebx
    
    ; Write CONFIG_ADDRESS to port 0xCF8
    mov dx, 0x0CF8
    out dx, eax
    
    ; Small delay for hardware (some chipsets need this)
    nop
    nop
    nop
    nop
    
    ; Read CONFIG_DATA from port 0xCFC
    mov dx, 0x0CFC
    in eax, dx
    
    pop rbx                     ; Restore callee-saved register
    ret

; =============================================================================
; pci_config_write32 - Write 32-bit value to PCI config space
;
; Microsoft x64 ABI Arguments:
;   RCX: bus (0-255)
;   RDX: device (0-31)
;   R8:  function (0-7)
;   R9:  offset (0-255, must be 4-byte aligned)
;   [RSP+40]: value to write (5th param on stack)
;
; Returns:
;   Nothing
; =============================================================================
global pci_config_write32
pci_config_write32:
    push rbx                    ; Save callee-saved register
    
    ; Build CONFIG_ADDRESS in EAX
    mov eax, 0x80000000
    
    mov ebx, ecx
    shl ebx, 16
    or eax, ebx
    
    mov ebx, edx
    shl ebx, 11
    or eax, ebx
    
    mov ebx, r8d
    shl ebx, 8
    or eax, ebx
    
    mov ebx, r9d
    and ebx, 0xFC
    or eax, ebx
    
    ; Write CONFIG_ADDRESS to port 0xCF8
    mov dx, 0x0CF8
    out dx, eax
    
    ; Small delay
    nop
    nop
    nop
    nop
    
    ; Get value from stack (5th parameter in Microsoft x64)
    ; Stack layout after push rbx: [RSP] = saved RBX, [RSP+8] = return addr
    ; 5th param is at [RSP + 8 + 32 + 8] = [RSP + 48]
    ; (32 bytes shadow space + 8 for return address)
    mov eax, [rsp + 48]
    
    ; Write to CONFIG_DATA port 0xCFC
    mov dx, 0x0CFC
    out dx, eax
    
    pop rbx
    ret

; =============================================================================
; pci_io_test - Test if PCI I/O ports are accessible
;
; Arguments: None
;
; Returns:
;   EAX: Value read back from 0xCF8 after writing test pattern
;        Should have bit 31 set if write succeeded
; =============================================================================
global pci_io_test
pci_io_test:
    ; Write a known pattern to 0xCF8 (bus 0, dev 0, func 0, reg 0, enable=1)
    mov eax, 0x80000000
    mov dx, 0x0CF8
    out dx, eax
    
    ; Delay
    nop
    nop
    nop
    nop
    
    ; Read it back
    in eax, dx
    ret

; =============================================================================
; io_outl - Raw 32-bit output to I/O port
;
; Microsoft x64 ABI Arguments:
;   RCX: port number (16-bit)
;   RDX: value to write (32-bit)
; =============================================================================
global io_outl
io_outl:
    push rdx                    ; Save value
    mov dx, cx                  ; Port to DX
    pop rax                     ; Value to EAX (from saved RDX)
    mov eax, edx                ; Actually get value from EDX
    mov dx, cx                  ; Port from RCX
    out dx, eax
    ret

; =============================================================================
; io_inl - Raw 32-bit input from I/O port
;
; Microsoft x64 ABI Arguments:
;   RCX: port number (16-bit)
;
; Returns:
;   EAX: value read
; =============================================================================
global io_inl
io_inl:
    mov dx, cx
    in eax, dx
    ret
