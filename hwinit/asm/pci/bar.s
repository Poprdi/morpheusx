; ═══════════════════════════════════════════════════════════════════════════
; PCI BAR (Base Address Register) helpers
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_pci_bar_read: Read BAR value
;   - asm_pci_bar_size: Determine BAR size via sizing algorithm
;   - asm_pci_bar_type: Determine BAR type flags
;   - asm_pci_bar_is_io: Check if BAR is I/O type
;   - asm_pci_bar_is_64bit: Check if BAR is 64-bit memory type
;   - asm_pci_bar_base: Extract base address from BAR value
;
; BAR Format:
;   Bit 0: 0=Memory, 1=I/O
;   For Memory BARs:
;     Bits 2-1: Type (00=32-bit, 10=64-bit, 01=reserved, 11=reserved)
;     Bit 3: Prefetchable
;     Bits 31-4: Base address (16-byte aligned minimum)
;   For I/O BARs:
;     Bit 1: Reserved
;     Bits 31-2: Base address (4-byte aligned)
;
; BAR Sizing Algorithm:
;   1. Save original BAR value
;   2. Write 0xFFFFFFFF to BAR
;   3. Read back BAR (returns size mask)
;   4. Restore original BAR value
;   5. Invert mask and add 1 to get size
;
; Note: These functions use Legacy PCI access (CF8/CFC).
;       For ECAM, caller should use ecam.s functions directly.
;
; Reference: PCI Local Bus Spec 3.0, ARCHITECTURE_V3.md
; ═══════════════════════════════════════════════════════════════════════════

section .text

; External dependencies (from legacy.s)
extern asm_pci_cfg_read32
extern asm_pci_cfg_write32

; Export symbols
global asm_pci_bar_read
global asm_pci_bar_read64
global asm_pci_bar_size
global asm_pci_bar_size64
global asm_pci_bar_type
global asm_pci_bar_is_io
global asm_pci_bar_is_64bit
global asm_pci_bar_base
global asm_pci_bar_base64

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_bar_read
; ───────────────────────────────────────────────────────────────────────────
; Read a single BAR value (32-bit)
;
; Parameters:
;   CL  = bus number
;   DL  = device number (bits 4:0)
;   R8B = function number (bits 2:0)
;   R9B = BAR index (0-5)
; Returns:
;   EAX = BAR value
; ───────────────────────────────────────────────────────────────────────────
asm_pci_bar_read:
    ; BAR registers start at offset 0x10
    ; BAR[n] is at offset 0x10 + (n * 4)
    movzx   r9d, r9b
    shl     r9d, 2              ; n * 4
    add     r9d, 0x10           ; + 0x10
    
    ; Call legacy PCI read
    jmp     asm_pci_cfg_read32

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_bar_read64
; ───────────────────────────────────────────────────────────────────────────
; Read a 64-bit BAR (for 64-bit memory BARs that span two BAR slots)
;
; Parameters:
;   CL  = bus number
;   DL  = device number
;   R8B = function number
;   R9B = BAR index (must be even: 0, 2, or 4)
; Returns:
;   RAX = 64-bit BAR value (low 32 from BAR[n], high 32 from BAR[n+1])
; ───────────────────────────────────────────────────────────────────────────
asm_pci_bar_read64:
    push    rbx
    push    r12
    push    r13
    push    r14
    
    ; Save parameters
    mov     r12b, cl            ; bus
    mov     r13b, dl            ; device
    mov     r14b, r8b           ; function
    mov     bl, r9b             ; bar index
    
    ; Read low 32 bits (BAR[n])
    mov     cl, r12b
    mov     dl, r13b
    mov     r8b, r14b
    mov     r9b, bl
    call    asm_pci_bar_read
    mov     r12d, eax           ; Save low 32 bits
    
    ; Read high 32 bits (BAR[n+1])
    mov     cl, r12b
    mov     dl, r13b
    mov     r8b, r14b
    mov     r9b, bl
    inc     r9b                 ; Next BAR slot
    call    asm_pci_bar_read
    
    ; Combine into 64-bit value
    shl     rax, 32
    or      rax, r12            ; RAX = high:low
    
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_bar_size
; ───────────────────────────────────────────────────────────────────────────
; Determine size of a BAR region using PCI sizing algorithm
;
; Parameters:
;   CL  = bus number
;   DL  = device number
;   R8B = function number
;   R9B = BAR index (0-5)
; Returns:
;   RAX = BAR size in bytes (0 if unimplemented)
; Clobbers: R10, R11
;
; Algorithm:
;   1. Save original value
;   2. Write all 1s
;   3. Read back (writable bits = size bits)
;   4. Restore original
;   5. Calculate size from mask
; ───────────────────────────────────────────────────────────────────────────
asm_pci_bar_size:
    push    rbx
    push    r12
    push    r13
    push    r14
    push    r15
    
    ; Save parameters
    mov     r12b, cl            ; bus
    mov     r13b, dl            ; device
    mov     r14b, r8b           ; function
    mov     r15b, r9b           ; bar index
    
    ; Calculate register offset
    movzx   r9d, r15b
    shl     r9d, 2
    add     r9d, 0x10           ; BAR offset
    mov     ebx, r9d            ; Save offset
    
    ; Step 1: Read and save original value
    mov     cl, r12b
    mov     dl, r13b
    mov     r8b, r14b
    call    asm_pci_cfg_read32
    mov     r10d, eax           ; R10 = original value
    
    ; Determine if I/O or Memory BAR
    test    eax, 1
    jnz     .io_bar
    
.memory_bar:
    ; Step 2: Write all 1s
    mov     cl, r12b
    mov     dl, r13b
    mov     r8b, r14b
    mov     r9d, ebx
    mov     r11d, 0xFFFFFFFF
    call    asm_pci_cfg_write32
    
    ; Step 3: Read back
    mov     cl, r12b
    mov     dl, r13b
    mov     r8b, r14b
    mov     r9d, ebx
    call    asm_pci_cfg_read32
    mov     r11d, eax           ; R11 = size mask
    
    ; Step 4: Restore original
    mov     cl, r12b
    mov     dl, r13b
    mov     r8b, r14b
    mov     r9d, ebx
    mov     r11d, r10d
    call    asm_pci_cfg_write32
    
    ; Step 5: Calculate size
    ; For memory BAR: mask = base | size_bits (inverted writable bits)
    ; Size = ~(mask & ~0xF) + 1
    mov     eax, r11d
    and     eax, 0xFFFFFFF0     ; Clear type bits
    cmp     eax, 0
    je      .unimplemented
    not     eax
    inc     eax                 ; Size = ~mask + 1
    jmp     .done
    
.io_bar:
    ; I/O BAR sizing
    mov     cl, r12b
    mov     dl, r13b
    mov     r8b, r14b
    mov     r9d, ebx
    mov     r11d, 0xFFFFFFFF
    call    asm_pci_cfg_write32
    
    mov     cl, r12b
    mov     dl, r13b
    mov     r8b, r14b
    mov     r9d, ebx
    call    asm_pci_cfg_read32
    mov     r11d, eax
    
    ; Restore
    mov     cl, r12b
    mov     dl, r13b
    mov     r8b, r14b
    mov     r9d, ebx
    mov     r11d, r10d
    call    asm_pci_cfg_write32
    
    ; Calculate I/O size (mask bits 31:2)
    mov     eax, r11d
    and     eax, 0xFFFFFFFC     ; Clear I/O indicator bits
    cmp     eax, 0
    je      .unimplemented
    not     eax
    inc     eax
    jmp     .done
    
.unimplemented:
    xor     eax, eax
    
.done:
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_bar_size64
; ───────────────────────────────────────────────────────────────────────────
; Determine size of a 64-bit BAR region
;
; Parameters:
;   CL  = bus number
;   DL  = device number
;   R8B = function number
;   R9B = BAR index (must be even)
; Returns:
;   RAX = 64-bit BAR size in bytes
; ───────────────────────────────────────────────────────────────────────────
asm_pci_bar_size64:
    ; TODO: Implement 64-bit BAR sizing
    ; Similar to 32-bit but operates on two consecutive BARs
    ; For now, just use 32-bit sizing (works for regions < 4GB)
    jmp     asm_pci_bar_size

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_bar_type
; ───────────────────────────────────────────────────────────────────────────
; Get BAR type flags
;
; Parameters:
;   ECX = BAR value
; Returns:
;   AL = type flags:
;        Bit 0: 1=I/O, 0=Memory
;        Bit 1: 1=64-bit, 0=32-bit (memory only)
;        Bit 2: 1=Prefetchable (memory only)
; ───────────────────────────────────────────────────────────────────────────
asm_pci_bar_type:
    xor     eax, eax
    
    ; Check I/O vs Memory
    test    ecx, 1
    jz      .check_memory
    
    ; I/O BAR
    mov     al, 1
    ret
    
.check_memory:
    ; Check 64-bit (bits 2:1 = 10b)
    mov     edx, ecx
    shr     edx, 1
    and     edx, 3
    cmp     edx, 2
    jne     .check_prefetch
    or      al, 2               ; Set 64-bit flag
    
.check_prefetch:
    ; Check prefetchable (bit 3)
    test    ecx, 8
    jz      .done
    or      al, 4               ; Set prefetchable flag
    
.done:
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_bar_is_io
; ───────────────────────────────────────────────────────────────────────────
; Check if BAR is I/O type
;
; Parameters:
;   ECX = BAR value
; Returns:
;   EAX = 1 if I/O BAR, 0 if memory BAR
; ───────────────────────────────────────────────────────────────────────────
asm_pci_bar_is_io:
    mov     eax, ecx
    and     eax, 1
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_bar_is_64bit
; ───────────────────────────────────────────────────────────────────────────
; Check if memory BAR is 64-bit type
;
; Parameters:
;   ECX = BAR value
; Returns:
;   EAX = 1 if 64-bit memory BAR, 0 otherwise
; ───────────────────────────────────────────────────────────────────────────
asm_pci_bar_is_64bit:
    ; Must be memory BAR first
    test    ecx, 1
    jnz     .not_64bit
    
    ; Check type field (bits 2:1 = 10b for 64-bit)
    mov     eax, ecx
    shr     eax, 1
    and     eax, 3
    cmp     eax, 2
    jne     .not_64bit
    
    mov     eax, 1
    ret
    
.not_64bit:
    xor     eax, eax
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_bar_base
; ───────────────────────────────────────────────────────────────────────────
; Extract base address from 32-bit BAR value
;
; Parameters:
;   ECX = BAR value
; Returns:
;   EAX = base address (type bits masked off)
; ───────────────────────────────────────────────────────────────────────────
asm_pci_bar_base:
    mov     eax, ecx
    test    eax, 1
    jnz     .io_bar
    
    ; Memory BAR: mask off bits 3:0
    and     eax, 0xFFFFFFF0
    ret
    
.io_bar:
    ; I/O BAR: mask off bits 1:0
    and     eax, 0xFFFFFFFC
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_pci_bar_base64
; ───────────────────────────────────────────────────────────────────────────
; Extract base address from 64-bit BAR value
;
; Parameters:
;   RCX = 64-bit BAR value (low BAR in low 32, high BAR in high 32)
; Returns:
;   RAX = 64-bit base address
; ───────────────────────────────────────────────────────────────────────────
asm_pci_bar_base64:
    mov     rax, rcx
    ; Mask off type bits in low 32 bits
    and     rax, 0xFFFFFFFFFFFFFFF0
    ret
