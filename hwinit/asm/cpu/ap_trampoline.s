; ═══════════════════════════════════════════════════════════════════════════
; ap_trampoline.s — AP bootstrap: real mode → protected → long mode
;
; Assembled as flat binary (nasm -f bin).  The BSP copies this blob to
; physical address 0x8000 and sends SIPI with vector = 0x08.
;
; At SIPI delivery the AP starts executing at CS:IP = 0x0800:0x0000
; in 16-bit real mode.  We need to get to 64-bit long mode and jump
; into ap_rust_entry(core_idx, lapic_id).
;
; DATA AREA at offset 0xF00 within this page is filled by ap_boot.rs
; before each AP is woken.  Layout must match the TD_* constants there.
;
; DEBUG markers are intentionally disabled in normal builds to keep
; early-boot logs readable on SMP.
;
; ═══════════════════════════════════════════════════════════════════════════

; ── COM1 debug macro ─────────────────────────────────────────────────────
; direct port I/O, no LSR check — QEMU's virtual UART is always ready.
%macro SERIAL_MARKER 1
    push    ax
    push    dx
    mov     dx, 0x3F8
    mov     al, %1
    out     dx, al
    pop     dx
    pop     ax
%endmacro

bits 16
org 0x8000          ; physical load address

; ───────────────────────────────────────────────────────────────────────────
; 16-bit real mode entry
; ───────────────────────────────────────────────────────────────────────────
ap_start:
    cli
    cld

    ; cs = 0x0800 from SIPI, set ds/es/ss to match
    mov     ax, cs
    mov     ds, ax
    mov     es, ax
    mov     ss, ax
    xor     sp, sp          ; stack at top of segment (wraps to 0xFFFF)

    ; SERIAL_MARKER '1'       ; marker 1: real mode entry alive

    ; ── load a TEMPORARY GDT that lives inside this trampoline page ──────
    ; The BSP's GDT is above 4 GB (PE BSS at 0x140xxxxxx).  In 16-bit
    ; mode, lgdt only reads a 4-byte base → truncated → wrong GDT → #GP.
    ; We use a temp GDT at a known sub-1MB address for the mode transition,
    ; then reload the real 64-bit GDT once we're in long mode.
    lgdt    [0xE38]         ; segment offset → physical 0x8E38 (temp_gdt_ptr_16)

    ; ── enter protected mode ──────────────────────────────────────────────
    mov     eax, cr0
    or      eax, 1          ; PE bit
    mov     cr0, eax

    ; far jump to 32-bit protected mode code
    ; selector 0x08 = index 1 = kernel code descriptor in BSP's GDT
    jmp     dword 0x08:ap_pm32

; ───────────────────────────────────────────────────────────────────────────
; 32-bit protected mode
; ───────────────────────────────────────────────────────────────────────────
bits 32
ap_pm32:
    ; marker disabled

    ; load data segments with kernel data selector (0x10)
    mov     ax, 0x10
    mov     ds, ax
    mov     es, ax
    mov     fs, ax
    mov     gs, ax
    mov     ss, ax

    ; ── enable PAE (required for long mode) ───────────────────────────────
    mov     eax, cr4
    or      eax, (1 << 5)   ; CR4.PAE
    mov     cr4, eax

    ; ── load kernel CR3 from data area ────────────────────────────────────
    mov     eax, dword [0x8F00]     ; TD_CR3 low 32 bits (phys address < 4GB)
    mov     cr3, eax

    ; marker disabled

    ; ── enable long mode via IA32_EFER.LME + NXE ──────────────────────────
    ; NXE (bit 11) is MANDATORY: the kernel page tables have NX bits (bit 63)
    ; set on data pages. with NXE=0, bit 63 is reserved → #PF on every
    ; TLB miss → triple fault → machine reset. ask me how i know.
    mov     ecx, 0xC0000080         ; IA32_EFER
    rdmsr
    or      eax, (1 << 8) | (1 << 11) ; LME + NXE
    wrmsr

    ; ── enable paging → activates long mode ───────────────────────────────
    mov     eax, cr0
    or      eax, (1 << 31)          ; CR0.PG
    mov     cr0, eax

    ; marker disabled

    ; far jump to 64-bit long mode code.
    ; selector 0x18 = temp GDT's 64-bit code descriptor (L=1, D=0).
    ; NOT 0x08 — that's the 32-bit code descriptor we used to get here.
    jmp     dword 0x18:ap_lm64

; ───────────────────────────────────────────────────────────────────────────
; 64-bit long mode
; ───────────────────────────────────────────────────────────────────────────
bits 64
ap_lm64:
    ; marker disabled

    ; reload data segments for 64-bit mode
    mov     ax, 0x10
    mov     ds, ax
    mov     es, ax
    mov     fs, ax
    mov     ss, ax
    ; intentionally skip gs — Rust will set GS base via MSR

    ; ── reload the BSP's ACTUAL 64-bit GDT from the data area ────────────
    ; In 64-bit mode lgdt reads the full 10-byte descriptor (2+8 base).
    ; The BSP's GDT is in PE BSS above 4 GB — now we can load it properly.
    lgdt    [0x8F20]        ; TD_GDT_PTR (flat 64-bit address)

    ; reload data segments again with the real GDT's selectors
    mov     ax, 0x10
    mov     ds, ax
    mov     es, ax
    mov     fs, ax
    mov     ss, ax

    ; ── load the per-AP stack BEFORE the retfq trick ────────────────────
    ; RSP is 0 from the real-mode `xor sp, sp`. push/retfq would write
    ; to 0xFFFFFFFF_FFFFFFF8 which is unmapped → #PF → triple fault.
    ; load the real stack first so the retfq has somewhere to push.
    mov     rsp, qword [0x8F10]     ; TD_STACK

    ; reload CS via a far return trick — push selector:offset, retfq
    lea     rax, [rel .cs_reloaded]
    push    0x08            ; kernel code selector from BSP's GDT
    push    rax
    retfq
.cs_reloaded:

    ; ── read core_idx and lapic_id from data area ─────────────────────────
    ; SysV ABI: arg1=RDI, arg2=RSI. target is x86_64-unknown-none-elf, not windows.
    mov     edi, dword [0x8F18]     ; TD_CORE_IDX → RDI (arg1, SysV x64)
    mov     esi, dword [0x8F1C]     ; TD_LAPIC_ID → RSI (arg2, SysV x64)

    ; ── jump to Rust entry point ──────────────────────────────────────────
    ; marker disabled

    mov     rax, qword [0x8F08]     ; TD_ENTRY64
    jmp     rax                     ; ap_rust_entry(core_idx, lapic_id) — never returns

; ───────────────────────────────────────────────────────────────────────────
; TEMP GDT (offset 0xE00) — used for the real→prot→long mode transition.
;
; The BSP's GDT lives in PE BSS above 4 GB.  A 16-bit lgdt can only read
; a 4-byte base, truncating 0x140xxxxxx → garbage.  This temp GDT at
; physical 0x8E00 is safely below 1 MB.
;
; We need TWO code descriptors:
;   0x08 — 32-bit code (D=1, L=0): for protected mode transition.
;          D=1 gives 32-bit default operand size so `bits 32` instructions
;          decode correctly.  If D=0 the CPU interprets `or eax, imm32`
;          as `or ax, imm16` and the instruction stream desynchronizes.
;          yes this is what was causing the triple fault. yes it was that dumb.
;   0x18 — 64-bit code (L=1, D=0): for the far jump into long mode.
;          Intel requires L=1 D=0 for 64-bit mode.
;
; After reaching 64-bit mode we reload the BSP's real GDT (which has its
; own code64 at selector 0x08) and retfq to it.
; ───────────────────────────────────────────────────────────────────────────
times (0xE00 - ($ - $$)) db 0

; 0xE00: temp GDT entries
temp_gdt:
    ; 0x00: null descriptor
    dq 0

    ; 0x08: 32-bit code (D=1, L=0, P=1, DPL=0, type=exec+read)
    ;   used ONLY for the real→protected mode transition.
    dw 0xFFFF       ; limit low (4GB with G=1)
    dw 0x0000       ; base low
    db 0x00         ; base mid
    db 0x9A         ; P=1, DPL=0, S=1, type=0xA (exec/read)
    db 0xCF         ; G=1, D=1, L=0, AVL=0, limit_hi=0xF
    db 0x00         ; base high

    ; 0x10: flat data (P=1, DPL=0, writable, G=1, D=1, 4GB limit)
    dw 0xFFFF       ; limit low
    dw 0x0000       ; base low
    db 0x00         ; base mid
    db 0x92         ; P=1, DPL=0, S=1, type=0x2 (data/write)
    db 0xCF         ; G=1, D=1, L=0, AVL=0, limit_hi=0xF
    db 0x00         ; base high

    ; 0x18: 64-bit code (L=1, D=0, P=1, DPL=0, type=exec+read)
    ;   used for the far jump into long mode after LME+PG are set.
    dw 0x0000       ; limit low (ignored in 64-bit mode)
    dw 0x0000       ; base low
    db 0x00         ; base mid
    db 0x9A         ; P=1, DPL=0, S=1, type=0xA (exec/read)
    db 0x20         ; G=0, L=1, D=0, AVL=0, limit_hi=0x0
    db 0x00         ; base high
temp_gdt_end:

; padding to 0xE38
times (0xE38 - ($ - $$)) db 0

; 0xE38: temp GDT pointer for 16-bit lgdt (6 bytes: limit + 32-bit base)
;   segment offset 0xE38 → physical 0x8E38.  lgdt reads 6 bytes in 16-bit mode.
temp_gdt_ptr_16:
    dw (temp_gdt_end - temp_gdt - 1)    ; limit = 31 (4 entries × 8 - 1)
    dd 0x00008E00                        ; base = physical address of temp_gdt

; pad to 0xF00
times (0xF00 - ($ - $$)) db 0

; ───────────────────────────────────────────────────────────────────────────
; DATA AREA (offset 0xF00 within the 4K page)
;
; Filled by ap_boot.rs before each SIPI.  Layout must match TD_* constants.
; ───────────────────────────────────────────────────────────────────────────
; 0xF00: CR3 (8 bytes)
dd 0, 0

; 0xF08: ENTRY64 — 64-bit Rust entry point address (8 bytes)
dd 0, 0

; 0xF10: STACK — per-AP kernel stack top (8 bytes)
dd 0, 0

; 0xF18: CORE_IDX (4 bytes)
dd 0

; 0xF1C: LAPIC_ID (4 bytes)
dd 0

; 0xF20: GDT_PTR — 10 bytes (limit:2 + base:8), copied from BSP's SGDT
dw 0            ; limit
dd 0, 0         ; base (8 bytes, split as dd to avoid `dq` in 16-bit section)

; 0xF2A: padding to 0xF30
times (0xF30 - (0xF20 + 10)) db 0

; 0xF30: READY flag (4 bytes) — AP sets to 1 when Rust entry is reached
dd 0

; pad rest of page
times (0x1000 - ($ - $$)) db 0
