; ═══════════════════════════════════════════════════════════════════════════
; Cache management primitives
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_cache_clflush: Flush cache line (serializing)
;   - asm_cache_clflushopt: Optimized cache line flush (weakly ordered)
;   - asm_cache_flush_range: Flush a range of memory
;
; These are used for DMA coherency when memory is mapped as Write-Back (WB)
; instead of Uncached (UC) or Write-Combining (WC). In UC/WC mode, cache
; flush is not needed - hardware handles coherency.
;
; Reference: NETWORK_IMPL_GUIDE.md §3.6 - Cache coherency
; ═══════════════════════════════════════════════════════════════════════════

section .text

; Export symbols
global asm_cache_clflush
global asm_cache_clflushopt
global asm_cache_flush_range

; Cache line size (64 bytes on all modern x86-64)
%define CACHE_LINE_SIZE 64

; ───────────────────────────────────────────────────────────────────────────
; asm_cache_clflush
; ───────────────────────────────────────────────────────────────────────────
; Flush cache line containing address (serializing)
;
; CLFLUSH is strongly ordered with respect to other CLFLUSH, stores,
; and MFENCE/SFENCE. Use when ordering with respect to other stores matters.
;
; Parameters:
;   RCX = address (any byte in the cache line)
; Returns: None
; Clobbers: None
; ───────────────────────────────────────────────────────────────────────────
asm_cache_clflush:
    clflush [rcx]               ; Flush cache line containing address
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_cache_clflushopt
; ───────────────────────────────────────────────────────────────────────────
; Optimized cache line flush (weakly ordered)
;
; CLFLUSHOPT is more efficient but weakly ordered. Multiple CLFLUSHOPTs
; may execute in any order. Use SFENCE after a batch of CLFLUSHOPTs
; to ensure completion before proceeding.
;
; Parameters:
;   RCX = address (any byte in the cache line)
; Returns: None
; Clobbers: None
; ───────────────────────────────────────────────────────────────────────────
asm_cache_clflushopt:
    clflushopt [rcx]            ; Optimized flush (requires SFENCE after batch)
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_cache_flush_range
; ───────────────────────────────────────────────────────────────────────────
; Flush a range of memory from cache
;
; Flushes all cache lines covering [addr, addr+len). Uses CLFLUSHOPT
; for efficiency and issues SFENCE at the end.
;
; Parameters:
;   RCX = start address
;   RDX = length in bytes
; Returns: None
; Clobbers: RAX, RCX, RDX
; ───────────────────────────────────────────────────────────────────────────
asm_cache_flush_range:
    ; Handle zero-length case
    test    rdx, rdx
    jz      .done
    
    ; Calculate end address
    lea     rax, [rcx + rdx]    ; RAX = end address
    
    ; Align start address down to cache line boundary
    and     rcx, ~(CACHE_LINE_SIZE - 1)
    
.loop:
    clflushopt [rcx]            ; Flush this cache line
    add     rcx, CACHE_LINE_SIZE
    cmp     rcx, rax
    jb      .loop               ; Continue while RCX < end
    
    sfence                      ; Ensure all flushes complete
    
.done:
    ret
