; ═══════════════════════════════════════════════════════════════════════════
; Delay/timing primitives
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_spin_hint: CPU hint for spin loop (PAUSE instruction)
;   - asm_delay_tsc: Delay for N TSC ticks
;   - asm_delay_us: Delay for N microseconds (requires TSC frequency)
;
; WARNING: Delay functions are BLOCKING! Use only during initialization
; where blocking is acceptable. Never use in the main poll loop.
;
; Reference: ARCHITECTURE_V3.md - delay primitives
; ═══════════════════════════════════════════════════════════════════════════

section .text

; Export symbols
global asm_spin_hint
global asm_delay_tsc
global asm_delay_us

; ───────────────────────────────────────────────────────────────────────────
; asm_spin_hint
; ───────────────────────────────────────────────────────────────────────────
; CPU hint for spin-wait loops
;
; The PAUSE instruction improves spin-wait loop performance by:
;   1. Reducing power consumption in the loop
;   2. Avoiding memory order violations when exiting the loop
;   3. Improving performance on hyperthreaded CPUs
;
; Parameters: None
; Returns: None
; Clobbers: None
; ───────────────────────────────────────────────────────────────────────────
asm_spin_hint:
    pause                       ; Spin-loop hint (~10-20 cycles)
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_delay_tsc
; ───────────────────────────────────────────────────────────────────────────
; Delay for a specified number of TSC ticks
;
; WARNING: This is a BLOCKING delay! Use only during initialization.
;
; Parameters:
;   RCX = number of TSC ticks to delay
; Returns: None
; Clobbers: RAX, RDX
; ───────────────────────────────────────────────────────────────────────────
asm_delay_tsc:
    ; Read starting TSC
    rdtsc                       ; EDX:EAX = start TSC
    shl     rdx, 32
    or      rax, rdx            ; RAX = start TSC (64-bit)
    
    add     rcx, rax            ; RCX = target TSC (start + delay)
    
.wait_loop:
    pause                       ; Spin hint
    rdtsc                       ; Read current TSC
    shl     rdx, 32
    or      rax, rdx            ; RAX = current TSC
    cmp     rax, rcx            ; Compare current with target
    jb      .wait_loop          ; Loop while current < target
    
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_delay_us
; ───────────────────────────────────────────────────────────────────────────
; Delay for a specified number of microseconds
;
; WARNING: This is a BLOCKING delay! Use only during initialization.
;
; Parameters:
;   RCX = number of microseconds to delay
;   RDX = TSC frequency in Hz (ticks per second)
; Returns: None
; Clobbers: RAX, RCX, RDX, R8
;
; Note: Calculates ticks = (us * tsc_freq) / 1,000,000
; ───────────────────────────────────────────────────────────────────────────
asm_delay_us:
    ; Calculate TSC ticks for this delay
    ; ticks = (us * freq) / 1,000,000
    mov     rax, rcx            ; RAX = microseconds
    mul     rdx                 ; RDX:RAX = us * freq
    
    ; Divide by 1,000,000
    mov     rcx, 1000000
    div     rcx                 ; RAX = ticks (quotient)
    
    mov     rcx, rax            ; RCX = ticks to delay
    
    ; Now do the TSC delay (inline version of asm_delay_tsc)
    rdtsc
    shl     rdx, 32
    or      rax, rdx            ; RAX = start TSC
    add     rcx, rax            ; RCX = target TSC
    
.wait_loop:
    pause
    rdtsc
    shl     rdx, 32
    or      rax, rdx
    cmp     rax, rcx
    jb      .wait_loop
    
    ret
