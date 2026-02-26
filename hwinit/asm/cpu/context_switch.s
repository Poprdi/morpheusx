; ═══════════════════════════════════════════════════════════════════════════
; context_switch.s — Timer ISR + preemptive context switch
;
; ABI:  Microsoft x64 (RCX, RDX, R8, R9, xmm0-3, shadow space)
; Format: PE/COFF (win64)
;
; Exports:
;   irq_timer_isr   — installed in IDT vector 0x20 (PIT IRQ 0).
;                     Saves the current CpuContext + FPU/SSE state,
;                     calls scheduler_tick() (Rust, MS x64), restores
;                     the next process's state, and resumes via iretq.
;
; CpuContext field layout (must match hwinit/src/process/context.rs):
;   0x00  rax
;   0x08  rbx
;   0x10  rcx
;   0x18  rdx
;   0x20  rsi
;   0x28  rdi
;   0x30  rbp
;   0x38  r8
;   0x40  r9
;   0x48  r10
;   0x50  r11
;   0x58  r12
;   0x60  r13
;   0x68  r14
;   0x70  r15
;   0x78  rip
;   0x80  rflags
;   0x88  rsp
;   0x90  cs
;   0x98  ss
;   Total: 0xA0 (160) bytes
;
; FPU/SSE state (FpuState):
;   Saved/restored via FXSAVE/FXRSTOR through `current_fpu_ptr`.
;   512 bytes, 16-byte aligned, stored per-process in PROCESS_TABLE.
;   The pointer is updated by scheduler_tick() when it picks the next process.
;
; iretq frame layout pushed by CPU at ISR entry (all 8-byte slots):
;   [rsp+0x00]  RIP      (return address in interrupted code)
;   [rsp+0x08]  CS       (code segment selector, zero-extended)
;   [rsp+0x10]  RFLAGS
;   [rsp+0x18]  RSP      (stack pointer before interrupt)
;   [rsp+0x20]  SS       (stack segment selector, zero-extended)
;   Size: 0x28 (40) bytes
; ═══════════════════════════════════════════════════════════════════════════

bits 64
default rel

; ── Data ──────────────────────────────────────────────────────────────────
section .data

align 8
global next_cr3
next_cr3: dq 0

; Pointer to the FpuState of the currently-running process.
; Written by scheduler_tick() on every switch; read by this ISR for
; FXSAVE (outgoing) and FXRSTOR (incoming).  NULL during early boot
; before the scheduler is initialized — guarded by a null check.
align 16
global current_fpu_ptr
current_fpu_ptr: dq 0

section .text

global irq_timer_isr
extern scheduler_tick           ; Rust fn: unsafe extern "C" (MS x64 ABI)

; ───────────────────────────────────────────────────────────────────────────
; irq_timer_isr — PIT timer interrupt handler (vector 0x20)
; ───────────────────────────────────────────────────────────────────────────
; Stack layout at ISR entry (before any pushes):
;   [rsp+0x28]  SS
;   [rsp+0x20]  RSP (before IRQ)
;   [rsp+0x18]  RFLAGS
;   [rsp+0x10]  CS
;   [rsp+0x00]  RIP
;
; After `sub rsp, 0xA0` our CpuContext struct lives at [rsp]:
;   [rsp+0x00 .. 0x9F]  CpuContext
;   [rsp+0xA0 .. 0xC7]  CPU iretq frame (5 × 8 bytes)
; ───────────────────────────────────────────────────────────────────────────
irq_timer_isr:
    ; ── Allocate CpuContext on stack ──────────────────────────────────────
    sub     rsp, 0xA0

    ; ── Save general-purpose registers ───────────────────────────────────
    mov     [rsp + 0x00], rax
    mov     [rsp + 0x08], rbx
    mov     [rsp + 0x10], rcx
    mov     [rsp + 0x18], rdx
    mov     [rsp + 0x20], rsi
    mov     [rsp + 0x28], rdi
    mov     [rsp + 0x30], rbp
    mov     [rsp + 0x38], r8
    mov     [rsp + 0x40], r9
    mov     [rsp + 0x48], r10
    mov     [rsp + 0x50], r11
    mov     [rsp + 0x58], r12
    mov     [rsp + 0x60], r13
    mov     [rsp + 0x68], r14
    mov     [rsp + 0x70], r15

    ; ── Fill rip / cs / rflags / rsp / ss from the CPU iretq frame ───────
    mov     rax, [rsp + 0xA0]           ; RIP
    mov     [rsp + 0x78], rax
    mov     rax, [rsp + 0xA0 + 0x08]   ; CS
    mov     [rsp + 0x90], rax
    mov     rax, [rsp + 0xA0 + 0x10]   ; RFLAGS
    mov     [rsp + 0x80], rax
    mov     rax, [rsp + 0xA0 + 0x18]   ; RSP (pre-IRQ)
    mov     [rsp + 0x88], rax
    mov     rax, [rsp + 0xA0 + 0x20]   ; SS
    mov     [rsp + 0x98], rax

    ; ── Save outgoing process FPU/SSE state (FXSAVE) ─────────────────────
    ; current_fpu_ptr → &proc.fpu_state of the process being preempted.
    ; Must happen BEFORE calling Rust (scheduler_tick may use XMM regs).
    mov     rbx, [rel current_fpu_ptr]
    test    rbx, rbx
    jz      .skip_fxsave
    fxsave  [rbx]
.skip_fxsave:

    ; ── ACK PIT (send EOI to master PIC before calling Rust) ─────────────
    mov     al, 0x20
    out     0x20, al

    ; ── Call scheduler_tick(current_ctx: *const CpuContext) ──────────────
    ; MS x64 ABI: first arg in RCX.  Need 32-byte shadow space on stack.
    ; Current RSP = 8 mod 16 (verified in file header comment).
    ; After sub 32 still 8 mod 16; CALL pushes 8 → callee sees 0 mod 16. ✓
    sub     rsp, 32                     ; shadow space
    lea     rcx, [rsp + 32]             ; &current_ctx
    call    scheduler_tick              ; RAX = *const CpuContext (next proc)
    add     rsp, 32                     ; remove shadow space

    ; RAX = *const CpuContext of next process (points into PROCESS_TABLE).
    ; scheduler_tick has updated current_fpu_ptr to point to the incoming
    ; process's FpuState.

    ; ── Restore incoming process FPU/SSE state (FXRSTOR) ──────────────────
    ; Must happen AFTER scheduler_tick (which updated the pointer) and
    ; BEFORE restoring GPRs (FXRSTOR clobbers no GPRs, but we use RBX as
    ; scratch — RBX will be properly restored from the next context below).
    mov     rbx, [rel current_fpu_ptr]
    test    rbx, rbx
    jz      .skip_fxrstor
    fxrstor [rbx]
.skip_fxrstor:

    ; ── Switch CR3 if process address spaces differ ───────────────────────
    ; next_cr3 is written by scheduler_tick() before returning.
    mov     rbx, [rel next_cr3]
    test    rbx, rbx
    jz      .skip_cr3                   ; zero = unset, don't switch
    mov     rcx, cr3
    cmp     rbx, rcx
    je      .skip_cr3                   ; same address space — avoid TLB flush
    mov     cr3, rbx
.skip_cr3:

    ; ── Patch iretq frame with next-process values ────────────────────────
    mov     rbx, [rax + 0x78]           ; next RIP
    mov     [rsp + 0xA0], rbx
    mov     rbx, [rax + 0x90]           ; next CS
    mov     [rsp + 0xA0 + 0x08], rbx
    mov     rbx, [rax + 0x80]           ; next RFLAGS
    mov     [rsp + 0xA0 + 0x10], rbx
    mov     rbx, [rax + 0x88]           ; next RSP
    mov     [rsp + 0xA0 + 0x18], rbx
    mov     rbx, [rax + 0x98]           ; next SS
    mov     [rsp + 0xA0 + 0x20], rbx

    ; ── Restore GPRs from next-process context ────────────────────────────
    ; rbx is restored last (used as scratch above) except rax (used as ptr).
    mov     r15, [rax + 0x70]
    mov     r14, [rax + 0x68]
    mov     r13, [rax + 0x60]
    mov     r12, [rax + 0x58]
    mov     r11, [rax + 0x50]
    mov     r10, [rax + 0x48]
    mov     r9,  [rax + 0x40]
    mov     r8,  [rax + 0x38]
    mov     rbp, [rax + 0x30]
    mov     rdi, [rax + 0x28]
    mov     rsi, [rax + 0x20]
    mov     rdx, [rax + 0x18]
    mov     rcx, [rax + 0x10]
    mov     rbx, [rax + 0x08]           ; restore rbx (was used as scratch)
    mov     rax, [rax + 0x00]           ; restore rax last

    ; ── Return to next process ────────────────────────────────────────────
    add     rsp, 0xA0                   ; remove CpuContext frame
    iretq
