; ═══════════════════════════════════════════════════════════════════════════
; VirtIO PCI Capability Parser
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_virtio_pci_parse_cap: Parse a VirtIO PCI capability
;   - asm_virtio_pci_find_all_caps: Find and parse all VirtIO caps
;   - asm_virtio_pci_read_bar: Read BAR value (with memory/IO detection)
;
; VirtIO PCI Capability Structure (at cap_offset):
;   +0x00: cap_vndr (u8)     = 0x09 (vendor-specific)
;   +0x01: cap_next (u8)     = offset to next capability
;   +0x02: cap_len (u8)      = length of this capability
;   +0x03: cfg_type (u8)     = 1=common, 2=notify, 3=isr, 4=device, 5=pci_cfg
;   +0x04: bar (u8)          = which BAR (0-5)
;   +0x05: padding[3]        = reserved
;   +0x08: offset (u32)      = offset within BAR
;   +0x0C: length (u32)      = length of region
;
; For NOTIFY capability (cfg_type=2), additional field:
;   +0x10: notify_off_multiplier (u32)
;
; Reference: VirtIO Spec 1.2 §4.1.4
; ═══════════════════════════════════════════════════════════════════════════

section .data
    ; VirtIO PCI capability offsets (within capability)
    VIRTIO_CAP_VNDR         equ 0x00
    VIRTIO_CAP_NEXT         equ 0x01
    VIRTIO_CAP_LEN          equ 0x02
    VIRTIO_CAP_CFG_TYPE     equ 0x03
    VIRTIO_CAP_BAR          equ 0x04
    VIRTIO_CAP_OFFSET       equ 0x08
    VIRTIO_CAP_LENGTH       equ 0x0C
    VIRTIO_CAP_NOTIFY_MULT  equ 0x10    ; Only for notify cap
    
    ; PCI BAR offsets in config space
    PCI_BAR0                equ 0x10
    PCI_BAR1                equ 0x14
    PCI_BAR2                equ 0x18
    PCI_BAR3                equ 0x1C
    PCI_BAR4                equ 0x20
    PCI_BAR5                equ 0x24
    
    ; BAR type bits
    BAR_TYPE_MEM            equ 0       ; Memory BAR (bit 0 = 0)
    BAR_TYPE_IO             equ 1       ; I/O BAR (bit 0 = 1)
    BAR_MEM_TYPE_64         equ 0x04    ; 64-bit memory BAR (bits 2:1 = 10)

section .text

; External functions
extern asm_pci_cfg_read8
extern asm_pci_cfg_read16
extern asm_pci_cfg_read32
extern asm_pci_find_virtio_cap

; Export symbols
global asm_virtio_pci_parse_cap
global asm_virtio_pci_read_bar
global asm_virtio_pci_get_bar_addr
global asm_virtio_pci_probe_caps

; ═══════════════════════════════════════════════════════════════════════════
; VirtioCapInfo structure (output from asm_virtio_pci_parse_cap)
; Size: 24 bytes, must match Rust #[repr(C)] struct
; ═══════════════════════════════════════════════════════════════════════════
; struct VirtioCapInfo {
;     cfg_type: u8,      // +0x00
;     bar: u8,           // +0x01
;     _pad: [u8; 2],     // +0x02
;     offset: u32,       // +0x04
;     length: u32,       // +0x08
;     notify_mult: u32,  // +0x0C (only valid for notify cap)
;     cap_offset: u8,    // +0x10 (PCI config space offset of this cap)
;     _pad2: [u8; 7],    // +0x11
; }

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_parse_cap
; ───────────────────────────────────────────────────────────────────────────
; Parse a VirtIO PCI capability at given config space offset
;
; Parameters:
;   CL  = bus number
;   DL  = device number
;   R8B = function number
;   R9B = capability offset in PCI config space
;   [RSP+40] = pointer to VirtioCapInfo output struct (5th param)
; Returns:
;   EAX = 1 on success, 0 on error
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_pci_parse_cap:
    push    rbp
    mov     rbp, rsp
    push    rbx
    push    r12
    push    r13
    push    r14
    push    r15
    
    ; Save parameters
    movzx   r12d, cl            ; bus
    movzx   r13d, dl            ; device
    movzx   r14d, r8b           ; function
    movzx   r15d, r9b           ; cap_offset
    
    ; Get output pointer from stack (5th param at RBP+16+32=RBP+48)
    mov     rbx, [rbp + 48]     ; RBX = output struct pointer
    test    rbx, rbx
    jz      .error
    
    ; Store cap_offset in output (+0x10)
    mov     byte [rbx + 0x10], r15b
    
    ; Read cfg_type (+3 from cap base)
    mov     cl, r12b
    mov     dl, r13b
    mov     r8b, r14b
    mov     r9b, r15b
    add     r9b, VIRTIO_CAP_CFG_TYPE
    call    asm_pci_cfg_read8
    mov     byte [rbx + 0x00], al   ; Store cfg_type
    
    ; Read bar (+4 from cap base)
    mov     cl, r12b
    mov     dl, r13b
    mov     r8b, r14b
    mov     r9b, r15b
    add     r9b, VIRTIO_CAP_BAR
    call    asm_pci_cfg_read8
    mov     byte [rbx + 0x01], al   ; Store bar
    
    ; Zero padding
    mov     word [rbx + 0x02], 0
    
    ; Read offset (+8 from cap base, 32-bit)
    mov     cl, r12b
    mov     dl, r13b
    mov     r8b, r14b
    mov     r9b, r15b
    add     r9b, VIRTIO_CAP_OFFSET
    call    asm_pci_cfg_read32
    mov     dword [rbx + 0x04], eax ; Store offset
    
    ; Read length (+12 from cap base, 32-bit)
    mov     cl, r12b
    mov     dl, r13b
    mov     r8b, r14b
    mov     r9b, r15b
    add     r9b, VIRTIO_CAP_LENGTH
    call    asm_pci_cfg_read32
    mov     dword [rbx + 0x08], eax ; Store length
    
    ; Check if this is a notify capability (cfg_type == 2)
    cmp     byte [rbx + 0x00], 2
    jne     .skip_notify_mult
    
    ; Read notify_off_multiplier (+16 from cap base)
    mov     cl, r12b
    mov     dl, r13b
    mov     r8b, r14b
    mov     r9b, r15b
    add     r9b, VIRTIO_CAP_NOTIFY_MULT
    call    asm_pci_cfg_read32
    mov     dword [rbx + 0x0C], eax ; Store notify_mult
    jmp     .done_mult
    
.skip_notify_mult:
    mov     dword [rbx + 0x0C], 0   ; Zero notify_mult for non-notify caps
    
.done_mult:
    ; Zero remaining padding
    mov     qword [rbx + 0x11], 0
    and     byte [rbx + 0x17], 0    ; Clear last byte of padding
    
    mov     eax, 1
    jmp     .done
    
.error:
    xor     eax, eax
    
.done:
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    pop     rbp
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_read_bar
; ───────────────────────────────────────────────────────────────────────────
; Read a PCI BAR value (handles 32-bit and 64-bit BARs)
;
; Parameters:
;   CL  = bus number
;   DL  = device number
;   R8B = function number
;   R9B = BAR index (0-5)
; Returns:
;   RAX = BAR base address (masked, without type bits)
;   RDX = 1 if memory BAR, 0 if I/O BAR
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_pci_read_bar:
    push    rbx
    push    r12
    push    r13
    push    r14
    push    r15
    
    ; Save parameters
    movzx   r12d, cl            ; bus
    movzx   r13d, dl            ; device  
    movzx   r14d, r8b           ; function
    movzx   r15d, r9b           ; bar_index
    
    ; Calculate BAR register offset: 0x10 + (bar_index * 4)
    mov     eax, r15d
    shl     eax, 2
    add     eax, PCI_BAR0
    movzx   r9d, al
    
    ; Read lower 32 bits of BAR
    mov     cl, r12b
    mov     dl, r13b
    mov     r8b, r14b
    mov     r9b, r9b
    call    asm_pci_cfg_read32
    mov     ebx, eax            ; EBX = low 32 bits
    
    ; Check if I/O BAR (bit 0 = 1)
    test    ebx, BAR_TYPE_IO
    jnz     .io_bar
    
    ; Memory BAR - check if 64-bit (bits 2:1 = 10)
    mov     eax, ebx
    and     eax, 0x06           ; Bits 2:1
    cmp     eax, BAR_MEM_TYPE_64
    jne     .mem32_bar
    
    ; 64-bit memory BAR - read high 32 bits from next BAR
    mov     eax, r15d
    inc     eax                 ; Next BAR index
    cmp     eax, 5              ; Check bounds
    ja      .mem32_bar          ; Treat as 32-bit if at end
    
    shl     eax, 2
    add     eax, PCI_BAR0
    
    mov     cl, r12b
    mov     dl, r13b
    mov     r8b, r14b
    mov     r9b, al
    call    asm_pci_cfg_read32
    
    ; Combine: RAX = (high << 32) | (low & ~0xF)
    shl     rax, 32
    mov     ecx, ebx
    and     ecx, 0xFFFFFFF0     ; Mask low 4 bits (type bits)
    or      rax, rcx
    mov     rdx, 1              ; Memory BAR
    jmp     .done
    
.mem32_bar:
    ; 32-bit memory BAR
    mov     eax, ebx
    and     eax, 0xFFFFFFF0     ; Mask low 4 bits
    mov     rdx, 1              ; Memory BAR
    jmp     .done
    
.io_bar:
    ; I/O BAR - mask low 2 bits
    mov     eax, ebx
    and     eax, 0xFFFFFFFC
    xor     rdx, rdx            ; I/O BAR (0)
    
.done:
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_get_bar_addr
; ───────────────────────────────────────────────────────────────────────────
; Get the actual address for a VirtIO region (BAR base + offset)
;
; Parameters:
;   CL  = bus number
;   DL  = device number
;   R8B = function number
;   R9B = BAR index
;   R10 = offset within BAR
; Returns:
;   RAX = computed address (BAR base + offset)
;   RDX = 1 if memory-mapped, 0 if port I/O
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_pci_get_bar_addr:
    push    r10
    
    ; Get BAR base address
    call    asm_virtio_pci_read_bar
    
    ; Add offset
    pop     r10
    add     rax, r10
    ; RDX already has memory/IO flag
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_probe_caps
; ───────────────────────────────────────────────────────────────────────────
; Probe all VirtIO capabilities for a device and fill output array
;
; Parameters:
;   CL  = bus number
;   DL  = device number
;   R8B = function number
;   R9  = pointer to array of 5 VirtioCapInfo structs (24 bytes each)
;         Index: [0]=common, [1]=notify, [2]=isr, [3]=device, [4]=pci_cfg
; Returns:
;   EAX = bitmask of found capabilities (bit 0=common, 1=notify, etc.)
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_pci_probe_caps:
    push    rbp
    mov     rbp, rsp
    sub     rsp, 32             ; Shadow space
    push    rbx
    push    r12
    push    r13
    push    r14
    push    r15
    
    ; Save parameters
    movzx   r12d, cl            ; bus
    movzx   r13d, dl            ; device
    movzx   r14d, r8b           ; function
    mov     r15, r9             ; output array
    xor     ebx, ebx            ; found bitmask
    
    ; Probe each capability type (1-5)
    mov     ecx, 1              ; Start with cfg_type 1 (common)
    
.probe_loop:
    cmp     ecx, 6
    jge     .done
    
    push    rcx                 ; Save cfg_type
    
    ; Find VirtIO cap of this type
    mov     cl, r12b
    mov     dl, r13b
    mov     r8b, r14b
    pop     r9                  ; cfg_type to find
    push    r9                  ; Save again
    call    asm_pci_find_virtio_cap
    
    pop     rcx                 ; Restore cfg_type
    test    eax, eax
    jz      .next_type          ; Not found
    
    ; Found capability at offset EAX
    ; Calculate output struct pointer: r15 + (cfg_type-1) * 24
    push    rcx
    push    rax                 ; Save cap offset
    
    mov     eax, ecx
    dec     eax                 ; cfg_type - 1
    imul    eax, 24
    add     rax, r15            ; Output struct address
    mov     r10, rax            ; Save output ptr
    
    pop     rax                 ; Restore cap offset
    
    ; Parse the capability
    push    rcx
    mov     [rsp - 8], r10      ; 5th param on stack
    sub     rsp, 48             ; Shadow space + 5th param
    mov     [rsp + 40], r10
    mov     cl, r12b
    mov     dl, r13b
    mov     r8b, r14b
    mov     r9b, al             ; cap offset
    call    asm_virtio_pci_parse_cap
    add     rsp, 48
    pop     rcx
    
    ; Set bit in found mask
    mov     eax, 1
    push    rcx
    dec     cl
    shl     eax, cl
    pop     rcx
    or      ebx, eax
    
    pop     rcx
    
.next_type:
    inc     ecx
    jmp     .probe_loop
    
.done:
    mov     eax, ebx            ; Return bitmask
    
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    add     rsp, 32
    pop     rbp
    ret

; ═══════════════════════════════════════════════════════════════════════════
; END OF FILE
; ═══════════════════════════════════════════════════════════════════════════
