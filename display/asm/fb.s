; ═══════════════════════════════════════════════════════════════════════════
; Framebuffer MMIO primitives
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_fb_write32: Write 32-bit pixel to framebuffer address
;   - asm_fb_read32: Read 32-bit pixel from framebuffer address
;   - asm_fb_memset32: Fill memory with 32-bit value (for clear/fill_rect)
;   - asm_fb_memcpy: Copy memory (for scroll)
;
; CRITICAL: These are the ONLY code paths that touch framebuffer memory.
; Standalone ASM call acts as compiler barrier - compiler cannot reorder across.
; This matches network crate's asm/core/mmio.s pattern.
;
; ═══════════════════════════════════════════════════════════════════════════

section .text

; Export symbols
global asm_fb_write32
global asm_fb_read32
global asm_fb_memset32
global asm_fb_memcpy

; ───────────────────────────────────────────────────────────────────────────
; asm_fb_write32
; ───────────────────────────────────────────────────────────────────────────
; Write 32-bit pixel value to framebuffer address
;
; Parameters:
;   RCX = framebuffer address (must be 4-byte aligned)
;   RDX = 32-bit pixel value (BGRX or RGBX format)
; Returns: None
; Clobbers: None
;
; Safety: Address must be within valid framebuffer region
; ───────────────────────────────────────────────────────────────────────────
asm_fb_write32:
    mov     [rcx], edx          ; 32-bit store to framebuffer
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_fb_read32
; ───────────────────────────────────────────────────────────────────────────
; Read 32-bit pixel value from framebuffer address
;
; Parameters:
;   RCX = framebuffer address (must be 4-byte aligned)
; Returns:
;   RAX = 32-bit pixel value (zero-extended to 64-bit)
; Clobbers: None
; ───────────────────────────────────────────────────────────────────────────
asm_fb_read32:
    mov     eax, [rcx]          ; 32-bit load from framebuffer
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_fb_memset32
; ───────────────────────────────────────────────────────────────────────────
; Fill framebuffer region with 32-bit value (for clear screen, fill rect)
;
; Parameters:
;   RCX = destination address (must be 4-byte aligned)
;   RDX = 32-bit value to fill with
;   R8  = count of 32-bit values to write (NOT bytes)
; Returns: None
; Clobbers: RCX, R8, RAX
;
; PERF: Uses REP STOSD which is hardware-optimized on modern Intel/AMD CPUs
; (ERMS/FSRM microcode). Up to 256-bit internal stores vs scalar 32-bit loop.
; For a 1920x1080 screen clear (~8 MB), this is ~10-20x faster.
; ───────────────────────────────────────────────────────────────────────────
asm_fb_memset32:
    test    r8, r8              ; Check if count is 0
    jz      .done
    push    rdi
    mov     rdi, rcx            ; Destination in RDI (REP STOSD target)
    mov     eax, edx            ; Value in EAX (REP STOSD source)
    mov     rcx, r8             ; Count in RCX (REP STOSD count)
    rep     stosd               ; Fill dwords - hardware optimized
    pop     rdi
.done:
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_fb_memcpy
; ───────────────────────────────────────────────────────────────────────────
; Copy memory within framebuffer (for scrolling)
;
; Parameters:
;   RCX = destination address
;   RDX = source address
;   R8  = number of bytes to copy
; Returns: None
; Clobbers: RCX, RDX, R8, RAX
;
; Note: Handles overlapping regions correctly (copies forward)
; For scroll up: dst < src, so forward copy is safe
; ───────────────────────────────────────────────────────────────────────────
asm_fb_memcpy:
    test    r8, r8              ; Check if count is 0
    jz      .done
    ; Use REP MOVSB for simplicity - hardware optimized on modern CPUs
    push    rdi
    push    rsi
    mov     rdi, rcx            ; Destination in RDI
    mov     rsi, rdx            ; Source in RSI
    mov     rcx, r8             ; Count in RCX
    rep     movsb               ; Copy bytes
    pop     rsi
    pop     rdi
.done:
    ret
