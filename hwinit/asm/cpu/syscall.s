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
; User code selector (CS for SYSRET = STAR[63:48] + 16)
; User data selector (SS for SYSRET = STAR[63:48])
; For now user selectors = 0 (no userspace yet); will be updated in Phase 3+.
%define USER_DS      0x00
%define USER_CS      0x00

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
    mov     edx, (USER_DS << 16) | KERNEL_CS   ; [63:48]=USER_DS, [47:32]=KERNEL_CS
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
; syscall_entry — fast ring-3 → ring-0 entry
;
; At entry (set by hardware):
;   RCX  = user RIP to return to
;   R11  = user RFLAGS
;   RSP  = still the USER stack (DANGEROUS — must switch immediately)
;   RAX  = syscall number
;   RDI  = arg1, RSI = arg2, RDX = arg3, R10 = arg4, R8 = arg5, R9 = arg6
;   Interrupts are OFF (IF cleared by IA32_FMASK)
;
; Note: Until per-process kernel stacks are set up (Phase 3+), all kernel
;       threads share the same RSP. For safety we use IA32_KERNEL_GS_BASE  
;       (or a simple global) to find the kernel stack. For now, since all
;       code is Ring 0 and SYSCALL is called kernel→kernel, we keep the
;       current stack (it IS the kernel stack).
; ───────────────────────────────────────────────────────────────────────────
syscall_entry:
    ; ── Save user registers we'll clobber ─────────────────────────────────
    ; RCX = user RIP (saved by CPU), R11 = user RFLAGS (saved by CPU).
    ; We need to preserve these for SYSRET.  Also save the user RSP.
    push    rcx                 ; user RIP
    push    r11                 ; user RFLAGS
    push    rbp
    mov     rbp, rsp

    ; ── Align stack and set up shadow space for MS x64 call ───────────────
    ; Save RAX (syscall nr) before it's clobbered.
    push    rax                 ; syscall number (will go into RCX below)
    push    rdi
    push    rsi
    push    rdx
    push    r10
    push    r8
    push    r9

    ; Shadow space (32 bytes) for the Rust callee
    sub     rsp, 32

    ; ── Call syscall_dispatch(nr, a1, a2, a3, a4, a5) ────────────────────
    ; MS x64: RCX, RDX, R8, R9 + stack (shadow)
    ; Our layout: nr=rax, a1=rdi, a2=rsi, a3=rdx, a4=r10, a5=r8, a6=r9
    ; Translate to MS x64: RCX=nr, RDX=a1, R8=a2, R9=a3; a4/a5 → stack

    ; Retrieve saved args (above shadow space)
    mov     rcx, [rsp + 32 + 7*8]  ; syscall nr (rax saved)
    mov     rdx, [rsp + 32 + 6*8]  ; rdi (arg1)
    mov     r8,  [rsp + 32 + 5*8]  ; rsi (arg2)
    mov     r9,  [rsp + 32 + 4*8]  ; rdx (arg3)
    ; arg4 (r10) and arg5 (r8) go on stack above shadow
    mov     rax, [rsp + 32 + 3*8]  ; r10 (arg4)
    mov     [rsp + 0x20], rax       ; arg4 @ rsp+32 (first stack arg)
    mov     rax, [rsp + 32 + 2*8]  ; r8 (arg5)
    mov     [rsp + 0x28], rax       ; arg5 @ rsp+40

    call    syscall_dispatch        ; RAX = return value

    ; ── Restore and return to caller ──────────────────────────────────────
    add     rsp, 32                 ; remove shadow space
    add     rsp, 7*8                ; remove saved args (nr, rdi, rsi, rdx, r10, r8, r9)
    ; RAX = syscall return value (must be preserved through pops below)
    push    rax                     ; save return value

    pop     rax                     ; restore return value into temp
    ; Restore rbp, user RFLAGS (r11), user RIP (rcx) for SYSRET
    mov     rsp, rbp
    pop     rbp
    pop     r11                     ; user RFLAGS
    pop     rcx                     ; user RIP

    ; Return value is in RAX.  SYSRET restores RCX→RIP, R11→RFLAGS.
    ; For kernel-mode callers: just `ret` would also work (no SYSRET).
    sysretq
