; PCI Configuration Space I/O Port Access
; Standalone assembly for reliable I/O port operations
;
; These functions use the System V AMD64 ABI:
;   - Arguments: RDI, RSI, RDX, RCX, R8, R9
;   - Return: RAX
;   - Caller-saved: RAX, RCX, RDX, RSI, RDI, R8-R11
;   - Callee-saved: RBX, RBP, R12-R15

section .text

; =============================================================================
; pci_config_read32 - Read 32-bit value from PCI config space
;
; Arguments:
;   RDI: bus (0-255)
;   RSI: device (0-31)  
;   RDX: function (0-7)
;   RCX: offset (0-255, must be 4-byte aligned)
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
    
    ; Add bus number (RDI << 16)
    mov ebx, edi
    shl ebx, 16
    or eax, ebx
    
    ; Add device number (RSI << 11)
    mov ebx, esi
    shl ebx, 11
    or eax, ebx
    
    ; Add function number (RDX << 8)
    mov ebx, edx
    shl ebx, 8
    or eax, ebx
    
    ; Add offset (RCX & 0xFC)
    mov ebx, ecx
    and ebx, 0xFC
    or eax, ebx
    
    ; Write CONFIG_ADDRESS to port 0xCF8
    mov dx, 0x0CF8
    out dx, eax
    
    ; Small delay for hardware (some chipsets need this)
    ; Just a few NOPs is usually enough
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
; Arguments:
;   RDI: bus (0-255)
;   RSI: device (0-31)
;   RDX: function (0-7)
;   RCX: offset (0-255, must be 4-byte aligned)
;   R8:  value to write
;
; Returns:
;   Nothing
; =============================================================================
global pci_config_write32
pci_config_write32:
    push rbx                    ; Save callee-saved register
    
    ; Build CONFIG_ADDRESS in EAX (same as read32)
    mov eax, 0x80000000
    
    mov ebx, edi
    shl ebx, 16
    or eax, ebx
    
    mov ebx, esi
    shl ebx, 11
    or eax, ebx
    
    mov ebx, edx
    shl ebx, 8
    or eax, ebx
    
    mov ebx, ecx
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
    
    ; Write value (R8D) to CONFIG_DATA port 0xCFC
    mov dx, 0x0CFC
    mov eax, r8d
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
; Arguments:
;   RDI: port number (16-bit)
;   RSI: value to write (32-bit)
; =============================================================================
global io_outl
io_outl:
    mov dx, di
    mov eax, esi
    out dx, eax
    ret

; =============================================================================
; io_inl - Raw 32-bit input from I/O port
;
; Arguments:
;   RDI: port number (16-bit)
;
; Returns:
;   EAX: value read
; =============================================================================
global io_inl
io_inl:
    mov dx, di
    in eax, dx
    ret
