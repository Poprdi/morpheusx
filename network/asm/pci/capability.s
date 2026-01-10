; ═══════════════════════════════════════════════════════════════════════════
; PCI Capability Chain Walker
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_pci_has_capabilities: Check if device supports capability list
;   - asm_pci_get_cap_ptr: Get first capability pointer (offset 0x34)
;   - asm_pci_find_cap: Find capability by ID in chain
;   - asm_pci_find_cap_next: Find next capability after given offset
;   - asm_pci_get_cap_vndr: Get vendor-specific capability at offset
;
; PCI Capability List:
;   - Status register bit 4 (0x06.4): indicates capability list present
;   - Capability pointer at offset 0x34 (for type 0 headers)
;   - Each capability: ID (1 byte), Next (1 byte), data...
;
; VirtIO PCI Capabilities (ID = 0x09, vendor-specific):
;   - VIRTIO_PCI_CAP_COMMON_CFG  = 1
;   - VIRTIO_PCI_CAP_NOTIFY_CFG  = 2
;   - VIRTIO_PCI_CAP_ISR_CFG     = 3
;   - VIRTIO_PCI_CAP_DEVICE_CFG  = 4
;   - VIRTIO_PCI_CAP_PCI_CFG     = 5
;
; Reference: PCI Spec 3.0 §6.7, VirtIO Spec 1.2 §4.1.4
; ═══════════════════════════════════════════════════════════════════════════

section .data
    ; PCI config space offsets
    PCI_STATUS_REG          equ 0x06    ; Status register (16-bit)
    PCI_CAP_PTR             equ 0x34    ; Capability pointer (8-bit)
    PCI_STATUS_CAP_LIST     equ 0x10    ; Bit 4: capabilities list

    ; Standard PCI Capability IDs
    PCI_CAP_ID_PM           equ 0x01    ; Power Management
    PCI_CAP_ID_AGP          equ 0x02    ; AGP
    PCI_CAP_ID_VPD          equ 0x03    ; Vital Product Data
    PCI_CAP_ID_SLOTID       equ 0x04    ; Slot Identification
    PCI_CAP_ID_MSI          equ 0x05    ; Message Signalled Interrupts
    PCI_CAP_ID_CHSWP        equ 0x06    ; CompactPCI HotSwap
    PCI_CAP_ID_PCIX         equ 0x07    ; PCI-X
    PCI_CAP_ID_HT           equ 0x08    ; HyperTransport
    PCI_CAP_ID_VNDR         equ 0x09    ; Vendor-Specific (VirtIO uses this)
    PCI_CAP_ID_DBG          equ 0x0A    ; Debug port
    PCI_CAP_ID_CCRC         equ 0x0B    ; CompactPCI Central Resource Control
    PCI_CAP_ID_SHPC         equ 0x0C    ; PCI Standard Hot-Plug Controller
    PCI_CAP_ID_SSVID        equ 0x0D    ; Bridge subsystem vendor/device ID
    PCI_CAP_ID_AGP3         equ 0x0E    ; AGP Target PCI-PCI bridge
    PCI_CAP_ID_EXP          equ 0x10    ; PCI Express
    PCI_CAP_ID_MSIX         equ 0x11    ; MSI-X
    PCI_CAP_ID_AF           equ 0x13    ; PCI Advanced Features

    ; VirtIO PCI Capability types (within vendor-specific cap)
    VIRTIO_PCI_CAP_COMMON   equ 1       ; Common configuration
    VIRTIO_PCI_CAP_NOTIFY   equ 2       ; Notifications
    VIRTIO_PCI_CAP_ISR      equ 3       ; ISR access
    VIRTIO_PCI_CAP_DEVICE   equ 4       ; Device specific configuration
    VIRTIO_PCI_CAP_PCI_CFG  equ 5       ; PCI configuration access

section .text

; External: PCI legacy config access
extern asm_pci_cfg_read8
extern asm_pci_cfg_read16
extern asm_pci_cfg_read32

; Export symbols
global asm_pci_has_capabilities
global asm_pci_get_cap_ptr
global asm_pci_find_cap
global asm_pci_find_cap_next
global asm_pci_read_cap_header
global asm_pci_find_virtio_cap

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_has_capabilities
; ───────────────────────────────────────────────────────────────────────────
; Check if PCI device supports capability list
;
; Parameters:
;   CL  = bus number
;   DL  = device number
;   R8B = function number
; Returns:
;   EAX = 1 if capabilities supported, 0 otherwise
; ───────────────────────────────────────────────────────────────────────────
asm_pci_has_capabilities:
    push    rcx
    push    rdx
    push    r8
    push    r9
    
    ; Read status register at offset 0x06
    mov     r9b, PCI_STATUS_REG
    call    asm_pci_cfg_read16
    
    ; Check bit 4 (capabilities list)
    test    ax, PCI_STATUS_CAP_LIST
    jz      .no_caps
    
    mov     eax, 1
    jmp     .done
    
.no_caps:
    xor     eax, eax
    
.done:
    pop     r9
    pop     r8
    pop     rdx
    pop     rcx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_get_cap_ptr
; ───────────────────────────────────────────────────────────────────────────
; Get the first capability pointer from PCI config space
;
; Parameters:
;   CL  = bus number
;   DL  = device number
;   R8B = function number
; Returns:
;   EAX = capability pointer (offset), or 0 if no capabilities
; ───────────────────────────────────────────────────────────────────────────
asm_pci_get_cap_ptr:
    push    rcx
    push    rdx
    push    r8
    push    r9
    
    ; First check if capabilities are supported
    call    asm_pci_has_capabilities
    test    eax, eax
    jz      .no_caps
    
    ; Restore registers and read capability pointer
    pop     r9
    pop     r8
    pop     rdx
    pop     rcx
    
    push    rcx
    push    rdx
    push    r8
    push    r9
    
    mov     r9b, PCI_CAP_PTR
    call    asm_pci_cfg_read8
    
    ; Mask to dword alignment (bottom 2 bits must be 0)
    and     eax, 0xFC
    jmp     .done
    
.no_caps:
    xor     eax, eax
    
.done:
    pop     r9
    pop     r8
    pop     rdx
    pop     rcx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_read_cap_header
; ───────────────────────────────────────────────────────────────────────────
; Read capability header (ID and next pointer) at given offset
;
; Parameters:
;   CL  = bus number
;   DL  = device number
;   R8B = function number
;   R9B = capability offset
; Returns:
;   AL = capability ID
;   AH = next capability offset (or 0 if end of list)
; ───────────────────────────────────────────────────────────────────────────
asm_pci_read_cap_header:
    ; Read 16 bits at capability offset (ID at +0, Next at +1)
    call    asm_pci_cfg_read16
    ; AX now has: AH = next pointer, AL = capability ID
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_find_cap
; ───────────────────────────────────────────────────────────────────────────
; Find a capability by ID in the capability chain
;
; Parameters:
;   CL  = bus number
;   DL  = device number
;   R8B = function number
;   R9B = capability ID to find
; Returns:
;   EAX = offset of capability, or 0 if not found
; ───────────────────────────────────────────────────────────────────────────
asm_pci_find_cap:
    push    rcx
    push    rdx
    push    r8
    push    r10
    push    r11
    push    r12
    
    ; Save the capability ID we're looking for
    movzx   r10d, r9b           ; R10 = target cap ID
    
    ; Get first capability pointer
    ; (We need to save/restore R9 since asm_pci_get_cap_ptr uses it)
    push    r9
    call    asm_pci_get_cap_ptr
    pop     r9
    
    test    eax, eax
    jz      .not_found
    
    mov     r11d, eax           ; R11 = current capability offset
    mov     r12d, 256           ; R12 = safety counter (max iterations)
    
.walk_loop:
    ; Safety check
    dec     r12d
    jz      .not_found
    
    ; Check valid offset
    test    r11d, r11d
    jz      .not_found
    cmp     r11d, 0xFF
    ja      .not_found
    
    ; Read capability header at current offset
    movzx   r9d, r11b           ; Move low byte to r9d (using movzx instead of mov r9b)
    call    asm_pci_read_cap_header
    
    ; Check if this is the capability we want
    movzx   ebx, al             ; EBX = cap ID
    cmp     ebx, r10d
    je      .found
    
    ; Move to next capability
    ; AH contains next pointer - extract via shift
    shr     eax, 8              ; Shift to get next byte
    and     eax, 0xFC           ; Align
    mov     r11d, eax           ; Store as new offset
    jmp     .walk_loop
    
.found:
    mov     eax, r11d           ; Return offset
    jmp     .done
    
.not_found:
    xor     eax, eax
    
.done:
    pop     r12
    pop     r11
    pop     r10
    pop     r8
    pop     rdx
    pop     rcx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_find_cap_next
; ───────────────────────────────────────────────────────────────────────────
; Find the next capability of a given ID after a starting offset
;
; Parameters:
;   CL  = bus number
;   DL  = device number
;   R8B = function number
;   R9B = starting capability offset (will search AFTER this)
;   R10B = capability ID to find
; Returns:
;   EAX = offset of next matching capability, or 0 if not found
; ───────────────────────────────────────────────────────────────────────────
asm_pci_find_cap_next:
    push    rcx
    push    rdx
    push    r8
    push    r11
    push    r12
    push    r13
    
    movzx   r13d, r10b          ; R13 = target cap ID
    movzx   r11d, r9b           ; R11 = current offset
    mov     r12d, 256           ; Safety counter
    
    ; First, read the next pointer from the starting capability
    call    asm_pci_read_cap_header
    shr     eax, 8              ; Get next pointer from AH
    and     eax, 0xFC
    mov     r11d, eax           ; Move to next cap
    
.walk_loop:
    dec     r12d
    jz      .not_found
    
    test    r11d, r11d
    jz      .not_found
    cmp     r11d, 0xFF
    ja      .not_found
    
    ; Read this capability
    movzx   r9d, r11b           ; Use movzx instead of mov r9b
    call    asm_pci_read_cap_header
    
    movzx   ebx, al
    cmp     ebx, r13d
    je      .found
    
    shr     eax, 8              ; Get next pointer from AH via shift
    and     eax, 0xFC
    mov     r11d, eax
    jmp     .walk_loop
    
.found:
    mov     eax, r11d
    jmp     .done
    
.not_found:
    xor     eax, eax
    
.done:
    pop     r13
    pop     r12
    pop     r11
    pop     r8
    pop     rdx
    pop     rcx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_find_virtio_cap
; ───────────────────────────────────────────────────────────────────────────
; Find a VirtIO-specific capability by cfg_type
;
; VirtIO caps are vendor-specific (ID=0x09) with additional cfg_type field
; Layout: [cap_vndr:8][cap_next:8][cap_len:8][cfg_type:8][bar:8][...]
;
; Parameters:
;   CL  = bus number
;   DL  = device number
;   R8B = function number
;   R9B = VirtIO cfg_type to find (1=common, 2=notify, 3=isr, 4=device)
; Returns:
;   EAX = offset of capability, or 0 if not found
; ───────────────────────────────────────────────────────────────────────────
asm_pci_find_virtio_cap:
    push    rcx
    push    rdx
    push    r8
    push    r10
    push    r11
    push    r12
    push    r13
    push    r14
    
    movzx   r13d, r9b           ; R13 = target cfg_type
    mov     r14d, 256           ; Safety counter
    
    ; Find first vendor-specific capability (ID=0x09)
    mov     r9b, PCI_CAP_ID_VNDR
    call    asm_pci_find_cap
    test    eax, eax
    jz      .not_found
    
    mov     r11d, eax           ; R11 = current vndr cap offset
    
.check_virtio_cap:
    ; Read cfg_type at offset+3 within this capability
    push    r9
    mov     r9b, r11b
    add     r9b, 3              ; cfg_type is at cap+3
    call    asm_pci_cfg_read8
    pop     r9
    
    cmp     al, r13b
    je      .found
    
    ; Find next vendor-specific capability
    dec     r14d
    jz      .not_found
    
    mov     r9b, r11b           ; Start from current cap
    mov     r10b, PCI_CAP_ID_VNDR
    call    asm_pci_find_cap_next
    test    eax, eax
    jz      .not_found
    
    mov     r11d, eax
    jmp     .check_virtio_cap
    
.found:
    mov     eax, r11d
    jmp     .done
    
.not_found:
    xor     eax, eax
    
.done:
    pop     r14
    pop     r13
    pop     r12
    pop     r11
    pop     r10
    pop     r8
    pop     rdx
    pop     rcx
    ret

; ═══════════════════════════════════════════════════════════════════════════
; END OF FILE
; ═══════════════════════════════════════════════════════════════════════════
