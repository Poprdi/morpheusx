; ═══════════════════════════════════════════════════════════════════════════
; TSC Calibration via PIT (Programmable Interval Timer)
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Self-contained TSC frequency calibration. No UEFI needed.
; Uses the 8254 PIT which runs at 1.193182 MHz.
;
; Method:
;   1. Program PIT channel 2 for one-shot countdown
;   2. Read TSC at start
;   3. Wait for PIT countdown to complete
;   4. Read TSC at end
;   5. Calculate: tsc_freq = (tsc_delta * PIT_FREQ) / pit_ticks
;
; Reference: Intel 8254 datasheet, OSDev wiki
; ═══════════════════════════════════════════════════════════════════════════

section .text

global asm_tsc_calibrate_pit

; PIT ports
PIT_CH0_DATA    equ 0x40
PIT_CH1_DATA    equ 0x41
PIT_CH2_DATA    equ 0x42
PIT_CMD         equ 0x43

; Port 0x61 - PC speaker / PIT gate control
PORT_61         equ 0x61

; PIT frequency: 1193182 Hz (1.193182 MHz)
; We'll use a ~10ms measurement (11932 ticks at 1.193182 MHz)
PIT_CALIBRATION_TICKS equ 11932     ; ~10ms

; ───────────────────────────────────────────────────────────────────────────
; asm_tsc_calibrate_pit
; ───────────────────────────────────────────────────────────────────────────
; Calibrate TSC frequency using PIT channel 2.
;
; Parameters: None
; Returns: RAX = TSC frequency in Hz (0 on failure)
; Clobbers: RCX, RDX, R8, R9, R10, R11
;
; No UEFI dependencies. Pure hardware access.
; ───────────────────────────────────────────────────────────────────────────
asm_tsc_calibrate_pit:
    push    rbx
    push    rdi
    push    rsi

    ; ─── Step 1: Set up PIT channel 2 for one-shot mode ───
    
    ; Read current port 0x61 value
    in      al, PORT_61
    mov     r8b, al                 ; Save original value
    
    ; Enable PIT channel 2 gate (bit 0) but disable speaker (bit 1)
    and     al, 0xFC                ; Clear bits 0-1
    or      al, 0x01                ; Set bit 0 (gate enable)
    out     PORT_61, al
    
    ; Program PIT channel 2: mode 0 (interrupt on terminal count)
    ; Bits 7-6: 10 = channel 2
    ; Bits 5-4: 11 = lobyte/hibyte access
    ; Bits 3-1: 000 = mode 0 (interrupt on terminal count)
    ; Bit 0: 0 = binary counting
    mov     al, 0xB0                ; 10110000b
    out     PIT_CMD, al
    
    ; Load countdown value (low byte first, then high byte)
    mov     ax, PIT_CALIBRATION_TICKS
    out     PIT_CH2_DATA, al        ; Low byte
    mov     al, ah
    out     PIT_CH2_DATA, al        ; High byte
    
    ; ─── Step 2: Read starting TSC ───
    
    ; Small delay for PIT to latch
    in      al, PORT_61
    in      al, PORT_61
    
    ; Read start TSC (serialized for accuracy)
    xor     eax, eax
    cpuid                           ; Serialize
    rdtsc
    shl     rdx, 32
    or      rax, rdx
    mov     rsi, rax                ; RSI = start TSC
    
    ; ─── Step 3: Wait for PIT countdown ───
    
    ; Poll port 0x61 bit 5 (OUT2) which goes high when countdown reaches 0
.wait_pit:
    in      al, PORT_61
    test    al, 0x20                ; Bit 5 = OUT2
    jz      .wait_pit
    
    ; ─── Step 4: Read ending TSC ───
    
    xor     eax, eax
    cpuid                           ; Serialize
    rdtsc
    shl     rdx, 32
    or      rax, rdx
    mov     rdi, rax                ; RDI = end TSC
    
    ; ─── Step 5: Restore port 0x61 ───
    
    mov     al, r8b
    out     PORT_61, al
    
    ; ─── Step 6: Calculate frequency ───
    
    ; tsc_delta = end - start
    sub     rdi, rsi                ; RDI = TSC delta
    
    ; tsc_freq = (tsc_delta * 1193182) / PIT_CALIBRATION_TICKS
    ; To avoid overflow: freq = (delta / ticks) * pit_freq + ((delta % ticks) * pit_freq) / ticks
    
    mov     rax, rdi                ; RAX = delta
    xor     rdx, rdx
    mov     rcx, PIT_CALIBRATION_TICKS
    div     rcx                     ; RAX = delta / ticks, RDX = remainder
    
    mov     r9, rax                 ; R9 = quotient
    mov     rax, rdx                ; RAX = remainder
    mov     rcx, 1193182            ; PIT frequency
    mul     rcx                     ; RDX:RAX = remainder * pit_freq
    mov     rcx, PIT_CALIBRATION_TICKS
    div     rcx                     ; RAX = (remainder * pit_freq) / ticks
    
    mov     rcx, 1193182
    imul    r9, rcx                 ; R9 = quotient * pit_freq
    add     rax, r9                 ; RAX = final frequency
    
    pop     rsi
    pop     rdi
    pop     rbx
    ret
