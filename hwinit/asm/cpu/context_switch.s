; ═══════════════════════════════════════════════════════════════════════════
; context_switch.s — Timer ISR + preemptive context switch (SMP-safe)
;
; ABI:  Microsoft x64 (RCX, RDX, R8, R9, xmm0-3, shadow space)
; Format: PE/COFF (win64)
;
; Exports:
;   irq_timer_isr   — installed in IDT vector 0x20 (LAPIC timer).
;                     Saves the current CpuContext + FPU/SSE state,
;                     calls scheduler_tick() (Rust, MS x64), restores
;                     the next process's state, and resumes via iretq.
;
; Per-CPU data is accessed via GS segment register.  The kernel sets
; IA32_GS_BASE to point at the PerCpu struct for each core.  SWAPGS
; is used on transitions between ring 3 and ring 0.
;
; PerCpu field offsets (must match per_cpu.rs):
;   gs:[0x00]  self_ptr
;   gs:[0x08]  cpu_id (u32)
;   gs:[0x0C]  current_pid (u32)
;   gs:[0x10]  next_cr3 (u64)
;   gs:[0x18]  current_fpu_ptr (u64)
;   gs:[0x20]  kernel_syscall_rsp (u64)
;   gs:[0x28]  user_rsp_scratch (u64)
;   gs:[0x30]  tss_ptr (u64)
;   gs:[0x38]  lapic_base (u64)
;   gs:[0x40]  tick_count (u64)
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
; iretq frame layout pushed by CPU at ISR entry (all 8-byte slots):
;   [rsp+0x00]  RIP
;   [rsp+0x08]  CS
;   [rsp+0x10]  RFLAGS
;   [rsp+0x18]  RSP
;   [rsp+0x20]  SS
;   Size: 0x28 (40) bytes
; ═══════════════════════════════════════════════════════════════════════════

bits 64
default rel

; LAPIC EOI register (identity-mapped)
%define LAPIC_EOI_ADDR  0xFEE000B0

section .text

global irq_timer_isr
extern scheduler_tick           ; Rust fn: unsafe extern "C" (MS x64 ABI)

; ───────────────────────────────────────────────────────────────────────────
; irq_timer_isr — LAPIC timer interrupt handler (vector 0x20)
; ───────────────────────────────────────────────────────────────────────────
irq_timer_isr:
    ; ── SWAPGS if coming from user mode (ring 3) ─────────────────────────
    ; check CS RPL in the iretq frame pushed by CPU
    test    qword [rsp + 0x08], 3   ; CS is at rsp+8 (second qword)
    jz      .no_swapgs_entry
    swapgs
.no_swapgs_entry:

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
    ; per-CPU FPU pointer: gs:[0x18]
    mov     rbx, [gs:0x18]              ; current_fpu_ptr
    test    rbx, rbx
    jz      .skip_fxsave
    fxsave  [rbx]
.skip_fxsave:

    ; ── ACK LAPIC (write 0 to EOI register) ──────────────────────────────
    ; 0xFEE000B0 has bit 31 set. [imm32] in 64-bit mode sign-extends to
    ; 0xFFFFFFFFFEE000B0. load into r11d: zero-extends, gives correct ptr.
    ; r11 is already saved at [rsp+0x50] so clobbering it here is safe.
    mov     r11d, LAPIC_EOI_ADDR        ; r11 = 0x00000000FEE000B0
    mov     dword [r11], 0              ; EOI

    ; ── Call scheduler_tick(current_ctx: *const CpuContext) ──────────────
    ; MS x64 ABI: first arg in RCX.  Need 32-byte shadow space on stack.
    sub     rsp, 32                     ; shadow space
    lea     rcx, [rsp + 32]             ; &current_ctx
    call    scheduler_tick              ; RAX = *const CpuContext (next proc)
    add     rsp, 32                     ; remove shadow space

    ; RAX = *const CpuContext of next process.
    ; scheduler_tick has updated gs:[0x18] (current_fpu_ptr) and
    ; gs:[0x10] (next_cr3) for the incoming process.

    ; ── Restore incoming process FPU/SSE state (FXRSTOR) ──────────────────
    mov     rbx, [gs:0x18]              ; updated current_fpu_ptr
    test    rbx, rbx
    jz      .skip_fxrstor
    fxrstor [rbx]
.skip_fxrstor:

    ; ── Switch CR3 if process address spaces differ ───────────────────────
    mov     rbx, [gs:0x10]              ; next_cr3 from PerCpu
    test    rbx, rbx
    jz      .skip_cr3
    mov     rcx, cr3
    cmp     rbx, rcx
    je      .skip_cr3
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
    mov     rbx, [rax + 0x08]
    mov     rax, [rax + 0x00]

    ; ── Remove CpuContext frame ───────────────────────────────────────────
    add     rsp, 0xA0

    ; ── SWAPGS if returning to user mode (ring 3) ────────────────────────
    ; check next CS in the patched iretq frame
    test    qword [rsp + 0x08], 3
    jz      .no_swapgs_exit
    swapgs
.no_swapgs_exit:

    ; ── Return to next process ────────────────────────────────────────────
    iretq
