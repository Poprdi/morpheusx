; crt0 for unmodified Rust std binaries on morpheus. The kernel jumps to _start
; with no TLS set up and std's generated C `main` expects to be called like a
; hosted process would. Mirrors the proven rung crt0 (variant-II TLS, FS = TCB).
bits 64
default rel
extern __tls_start, __tls_end, main
global _start

SYS_EXIT                equ 0
SYS_MMAP                equ 26
SYS_SET_THREAD_POINTER  equ 100
PROT_WRITE              equ 1
MAP_PRIVATE_ANON        equ 0x22
TCB_SIZE                equ 64

section .text
_start:
    lea  r12, [__tls_start]
    lea  r13, [__tls_end]
    sub  r13, r12                   ; r13 = block size (whole .tdata+.tbss, file-backed)
    jz   .run
    lea  rdi, [r13 + TCB_SIZE + 4095]
    shr  rdi, 12                    ; pages
    mov  rax, SYS_MMAP
    mov  rsi, PROT_WRITE
    mov  rdx, MAP_PRIVATE_ANON
    xor  r10, r10
    syscall
    mov  rcx, 0xFFFFFFFFFFFFF001    ; -errno range: run without TLS rather than fault
    cmp  rax, rcx
    jae  .run
    mov  rdi, rax                   ; dst = block base
    mov  rsi, r12                   ; src = template
    mov  rcx, r13
    lea  r14, [rax + r13]           ; tp = base + block
    rep  movsb
    mov  [r14], r14                 ; TCB self-pointer (local-exec reads %fs:0)
    mov  rax, SYS_SET_THREAD_POINTER
    mov  rdi, r14
    syscall
.run:
    and  rsp, -16                   ; SysV: rsp 16-aligned at call
    xor  edi, edi                   ; argc: std's args PAL reads SYS_GETARGS itself
    xor  esi, esi                   ; argv
    call main
    mov  edi, eax                   ; i32 exit code from lang_start
    mov  rax, SYS_EXIT
    syscall
    ud2
