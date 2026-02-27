; ═══════════════════════════════════════════════════════════════════════════
; Framebuffer double-buffer delta presenter
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   asm_fb_present_delta — diff back/shadow, write changed spans to VRAM
;
; Design:
;   The kernel maintains three same-layout (same stride) pixel buffers:
;     back   — user-mapped back buffer written by the application (cacheable RAM)
;     shadow — kernel-private mirror of what is currently on screen (cacheable RAM)
;     vram   — real hardware framebuffer (uncacheable / write-combining VRAM)
;
;   For each scanline, we scan for contiguous runs of pixels where
;   back[x] != shadow[x] (changed spans).  Each changed span is flushed
;   to VRAM and shadow in a single `rep movsd`, minimising uncacheable
;   writes.  Unchanged pixels are skipped entirely.
;
;   All three buffers share the same stride (pixels per row >= width) so
;   advancing row pointers is uniform: ptr += stride * 4.
;
;   rep movsd only clobbers RSI, RDI, RCX.  RAX, R8-R15, RBX, RBP are
;   preserved across the instruction, so span state (start, end, row
;   pointers) survives without any extra saves.
;
; ═══════════════════════════════════════════════════════════════════════════

section .text

global asm_fb_present_delta

; ───────────────────────────────────────────────────────────────────────────
; asm_fb_present_delta
; ───────────────────────────────────────────────────────────────────────────
;
; void asm_fb_present_delta(
;     u64 back,    // RCX  — back buffer base (cacheable RAM, same layout as VRAM)
;     u64 shadow,  // RDX  — shadow buffer base (cacheable RAM, same layout as VRAM)
;     u64 vram,    // R8   — real framebuffer base (uncacheable / WC)
;     u64 width,   // R9   — active pixels per scanline
;     u64 height,  // [rsp+40] at call site — number of scanlines
;     u64 stride   // [rsp+48] at call site — pixels per row (>= width, HW alignment)
; )
;
; All three buffers use the same pixel layout (u32, packed 4 bytes/pixel)
; and the same stride.  back and shadow are allocated as normal cacheable
; RAM; vram is the uncacheable hardware framebuffer.
;
; Non-volatile registers saved/restored: RBP, RBX, RSI, RDI, R12-R15
; Volatile registers trashed:            RAX, RCX, RDX, R8-R11
;
; Register map (within function after prologue):
;   R12 = back base address
;   R13 = shadow base address
;   R14 = vram base address
;   R15 = width (pixels per active row)
;   RBX = height (number of rows)
;   RBP = stride (pixels per row in all three buffers)
;   R8  = current row index
;   R9  = back row pointer  (= R12 + row * stride * 4, advanced incrementally)
;   R10 = shadow row pointer (= R13 + row * stride * 4, advanced incrementally)
;   R11 = vram row pointer   (= R14 + row * stride * 4, advanced incrementally)
;   RAX = current x (column) within scanline
;   RDX = span_start (x at which the current differing run began)
;   RCX = span_len / comparison scratch (recomputed before each rep movsd)
;   RSI = rep movsd source (set just before instruction)
;   RDI = rep movsd destination (set just before instruction)
; ───────────────────────────────────────────────────────────────────────────
asm_fb_present_delta:
    ; ── Prologue: save all non-volatile registers we use ──────────────────
    ; 8 pushes = 64 bytes.  Stack args (Win64 ABI) at entry are:
    ;   [rsp+40] = height,  [rsp+48] = stride
    ; After 8 pushes:
    ;   [rsp+104] = height, [rsp+112] = stride
    push    rbp
    push    rbx
    push    rsi
    push    rdi
    push    r12
    push    r13
    push    r14
    push    r15

    ; ── Load parameters into preserved registers ───────────────────────────
    mov     r12, rcx                ; back base
    mov     r13, rdx                ; shadow base
    mov     r14, r8                 ; vram base
    mov     r15, r9                 ; width
    mov     rbx, [rsp + 104]        ; height
    mov     rbp, [rsp + 112]        ; stride (pixels)

    ; ── Early-out guards ──────────────────────────────────────────────────
    test    rbx, rbx
    jz      .epilog
    test    r15, r15
    jz      .epilog

    ; ── Initialise row pointers to start of row 0 ─────────────────────────
    mov     r9,  r12                ; back_row_ptr   = back base
    mov     r10, r13                ; shadow_row_ptr = shadow base
    mov     r11, r14                ; vram_row_ptr   = vram base
    xor     r8,  r8                 ; row = 0

; ── Outer loop: one iteration per scanline ────────────────────────────────
.row_loop:
    cmp     r8, rbx
    jge     .epilog

    xor     rax, rax                ; x = 0

; ── Inner loop: scan for changed pixel spans ──────────────────────────────
.scan:
    cmp     rax, r15                ; x >= width?
    jge     .row_done

    mov     ecx, [r9  + rax*4]     ; back[x]
    cmp     ecx, [r10 + rax*4]     ; == shadow[x]?
    jne     .span_start
    inc     rax
    jmp     .scan

; ── Found first pixel of a changed span ───────────────────────────────────
.span_start:
    mov     rdx, rax                ; span_start = x
    inc     rax

; ── Extend span while pixels still differ ─────────────────────────────────
.span_extend:
    cmp     rax, r15
    jge     .span_flush
    mov     ecx, [r9  + rax*4]
    cmp     ecx, [r10 + rax*4]
    je      .span_flush             ; equal pixel ends the span
    inc     rax
    jmp     .span_extend

; ── Flush span [rdx, rax) to VRAM and shadow ──────────────────────────────
.span_flush:
    ; Span length in dwords
    mov     rcx, rax
    sub     rcx, rdx                ; rcx = span_len

    ; 1. Copy back[span] → vram[span]
    lea     rsi, [r9  + rdx*4]     ; src = back_row[span_start]
    lea     rdi, [r11 + rdx*4]     ; dst = vram_row[span_start]
    rep     movsd

    ; rep movsd clobbered RCX, RSI, RDI.
    ; RAX (x/end), RDX (span_start), R8-R15, RBX, RBP all intact.

    ; 2. Copy back[span] → shadow[span]  (bring shadow up-to-date)
    mov     rcx, rax
    sub     rcx, rdx                ; rcx = span_len (recomputed)
    lea     rsi, [r9  + rdx*4]     ; src = back_row[span_start]
    lea     rdi, [r10 + rdx*4]     ; dst = shadow_row[span_start]
    rep     movsd

    jmp     .scan                   ; continue scanning from rax = end of span

; ── Advance all row pointers by one stride row ────────────────────────────
.row_done:
    lea     r9,  [r9  + rbp*4]     ; back_row_ptr   += stride * 4
    lea     r10, [r10 + rbp*4]     ; shadow_row_ptr += stride * 4
    lea     r11, [r11 + rbp*4]     ; vram_row_ptr   += stride * 4
    inc     r8
    jmp     .row_loop

; ── Epilogue: restore non-volatile registers ──────────────────────────────
.epilog:
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rdi
    pop     rsi
    pop     rbx
    pop     rbp
    ret
