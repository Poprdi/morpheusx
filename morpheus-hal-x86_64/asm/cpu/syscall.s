; ═══════════════════════════════════════════════════════════════════════════
; syscall.s — SYSCALL entry/exit trampoline
;
; ABI:  Microsoft x64 inside the kernel; custom at the SYSCALL boundary.
; Format: PE/COFF (win64)
;
; The SYSCALL instruction is the fast ring-3 → ring-0 transition:
;   - Saves RIP → RCX, RFLAGS → R11
;   - Loads RIP from IA32_LSTAR MSR (points here)
;   - Clears RFLAGS bits set in IA32_FMASK
;   - Does NOT switch stacks (must be done manually)
;
; Calling convention for user-space syscalls (future ring-3):
;   RAX = syscall number
;   RDI = arg1    RSI = arg2    RDX = arg3
;   R10 = arg4    R8  = arg5    R9  = arg6
;   Return value: RAX (u64, -errno on error)
;
; Exports:
;   syscall_entry  — installed as IA32_LSTAR handler by syscall_init().
;   syscall_init   — Rust-callable fn to configure the SYSCALL MSRs.
;
; ═══════════════════════════════════════════════════════════════════════════

bits 64
default rel

section .text

global syscall_entry
global syscall_init
extern syscall_dispatch         ; Rust: unsafe extern "C" fn(nr, a1, a2, a3, a4, a5) -> u64

; IA32 MSR addresses
%define IA32_EFER    0xC0000080
%define IA32_STAR    0xC0000081
%define IA32_LSTAR   0xC0000082
%define IA32_FMASK   0xC0000084

; GDT selectors (must match hwinit/src/cpu/gdt.rs)
%define KERNEL_CS    0x08
%define KERNEL_DS    0x10
; SYSRET computes:  CS = STAR[63:48]+16,  SS = STAR[63:48]+8
; With SYSRET_BASE = 0x10:  CS = 0x20 (user code),  SS = 0x18 (user data)
; RPL is forced to 3 by the CPU.
%define SYSRET_BASE  0x10

; ───────────────────────────────────────────────────────────────────────────
; syscall_init — Configure SYSCALL/SYSRET MSRs
;
; Sets:
;   IA32_EFER.SCE  = 1 (enable SYSCALL/SYSRET in 64-bit mode)
;   IA32_STAR      = [kernel_cs << 32 | (user_ds-8) << 48]
;   IA32_LSTAR     = address of syscall_entry
;   IA32_FMASK     = ~0x200 (clears IF on syscall, keeping other flags)
;
; Parameters: none
; Returns: void
; ───────────────────────────────────────────────────────────────────────────
syscall_init:
    ; Enable SYSCALL: set IA32_EFER.SCE (bit 0)
    mov     ecx, IA32_EFER
    rdmsr
    or      eax, 1              ; SCE = bit 0
    wrmsr

    ; IA32_STAR layout:
    ;   [63:48] = SS for SYSRET (user data sel);  CS = SS+8
    ;   [47:32] = CS for SYSCALL entry (kernel CS);  SS = CS+8
    ;   [31:0]  = reserved (zero)
    mov     ecx, IA32_STAR
    xor     eax, eax
    mov     edx, (SYSRET_BASE << 16) | KERNEL_CS  ; [63:48]=SYSRET_BASE, [47:32]=KERNEL_CS
    wrmsr

    ; IA32_LSTAR = address of our syscall entry point
    mov     ecx, IA32_LSTAR
    lea     rax, [syscall_entry]
    mov     rdx, rax
    shr     rdx, 32
    wrmsr

    ; IA32_FMASK: bits to CLEAR in RFLAGS at SYSCALL entry.
    ; Clear IF (0x200) so the kernel handler runs with interrupts disabled.
    ; Caller's IF is saved in R11 and restored by SYSRET.
    mov     ecx, IA32_FMASK
    mov     eax, 0x200          ; clear IF
    xor     edx, edx
    wrmsr

    ret

; ───────────────────────────────────────────────────────────────────────────
; syscall_entry — fast ring-3 → ring-0 entry (SMP-safe)
;
; At entry (set by hardware):
;   RCX  = user RIP to return to
;   R11  = user RFLAGS
;   RSP  = USER stack (must switch immediately)
;   RAX  = syscall number
;   RDI  = arg1, RSI = arg2, RDX = arg3, R10 = arg4, R8 = arg5, R9 = arg6
;   Interrupts are OFF (IF cleared by IA32_FMASK)
;
; Per-CPU data via GS segment (PerCpu offsets from per_cpu.rs):
;   gs:[0x20] = kernel_syscall_rsp
;   gs:[0x28] = user_rsp_scratch
; ───────────────────────────────────────────────────────────────────────────

section .text
syscall_entry:
    ; ── SWAPGS: switch from user GS to kernel PerCpu GS ──────────────────
    ; always needed — SYSCALL is always from ring 3
    swapgs

    ; ── Switch to kernel stack via per-CPU data ───────────────────────────
    mov     [gs:0x28], rsp              ; save user RSP → gs:user_rsp_scratch
    mov     rsp, [gs:0x20]              ; load kernel RSP from gs:kernel_syscall_rsp

    ; ── Build a frame for SYSRET restoration ──────────────────────────────
    push    qword [gs:0x28]             ; user RSP
    push    rcx                         ; user RIP
    push    r11                         ; user RFLAGS

    ; ── Save user argument registers ──────────────────────────────────────
    push    r8                          ; user a5
    push    r10                         ; user a4
    push    rdi                         ; user a1
    push    rsi                         ; user a2
    push    rdx                         ; user a3

    ; ── Save callee-saved (MS x64 ABI for the Rust call) ─────────────────
    push    rbp
    push    rbx
    push    r12
    push    r13
    push    r14
    push    r15
    mov     rbp, rsp

    ; ── Translate user ABI → MS x64 and call syscall_dispatch ─────────────
    sub     rsp, 48                 ; 32 shadow + 16 stack args
    mov     [rsp + 0x28], r8        ; a5
    mov     [rsp + 0x20], r10       ; a4
    mov     r9, rdx                 ; a3
    mov     r8, rsi                 ; a2
    mov     rdx, rdi                ; a1
    mov     rcx, rax                ; nr

    call    syscall_dispatch        ; returns in RAX

    ; ── Tear down and restore ─────────────────────────────────────────────
    mov     rsp, rbp
    pop     r15
    pop     r14
    pop     r13
    pop     r12
    pop     rbx
    pop     rbp

    ; ── Restore user argument registers ───────────────────────────────────
    pop     rdx                     ; user a3
    pop     rsi                     ; user a2
    pop     rdi                     ; user a1
    pop     r10                     ; user a4
    pop     r8                      ; user a5

    pop     r11                     ; user RFLAGS
    pop     rcx                     ; user RIP
    ; RAX = return value (preserved across all the above)
    mov     rsp, [rsp]              ; load user RSP (top of stack is user RSP)

    ; ── SWAPGS: switch from kernel PerCpu GS back to user GS ─────────────
    swapgs

    o64 sysret                      ; SYSRETQ — back to Ring 3
