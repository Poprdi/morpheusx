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
; ═══════════════════════════════════════════════════════════════════════════

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

    ; ── load the GDT pointer from the data area ──────────────────────────
    ; data area is at this_page + 0xF00.  Our segment base = 0x8000.
    ; so offset within segment = 0xF00 + 0x20 (TD_GDT_PTR within data area)
    lgdt    [0xF20]         ; 0x8000 + 0xF20 in linear addressing = offset 0xF20 from segment base

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

    ; ── enable long mode via IA32_EFER.LME ────────────────────────────────
    mov     ecx, 0xC0000080         ; IA32_EFER
    rdmsr
    or      eax, (1 << 8)           ; LME = bit 8
    wrmsr

    ; ── enable paging → activates long mode ───────────────────────────────
    mov     eax, cr0
    or      eax, (1 << 31)          ; CR0.PG
    mov     cr0, eax

    ; far jump to 64-bit long mode code
    jmp     dword 0x08:ap_lm64

; ───────────────────────────────────────────────────────────────────────────
; 64-bit long mode
; ───────────────────────────────────────────────────────────────────────────
bits 64
ap_lm64:
    ; reload data segments for 64-bit mode
    mov     ax, 0x10
    mov     ds, ax
    mov     es, ax
    mov     fs, ax
    mov     ss, ax
    ; intentionally skip gs — Rust will set GS base via MSR

    ; ── load the per-AP stack from data area ──────────────────────────────
    mov     rsp, qword [0x8F10]     ; TD_STACK

    ; ── read core_idx and lapic_id from data area ─────────────────────────
    mov     ecx, dword [0x8F18]     ; TD_CORE_IDX → RCX (arg1, MS x64)
    mov     edx, dword [0x8F1C]     ; TD_LAPIC_ID → RDX (arg2, MS x64)

    ; ── jump to Rust entry point ──────────────────────────────────────────
    mov     rax, qword [0x8F08]     ; TD_ENTRY64
    jmp     rax                     ; ap_rust_entry(core_idx, lapic_id) — never returns

; ───────────────────────────────────────────────────────────────────────────
; Pad to keep total code well under 0xF00 (data area starts there)
; ───────────────────────────────────────────────────────────────────────────
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
