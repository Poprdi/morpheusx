; ═══════════════════════════════════════════════════════════════════════════
; VirtIO Device Initialization
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; Functions:
;   - asm_virtio_reset: Reset VirtIO device (write 0, wait for 0)
;   - asm_virtio_set_status: Write VirtIO status register
;   - asm_virtio_get_status: Read VirtIO status register
;   - asm_virtio_read_features: Read device feature bits (64-bit)
;   - asm_virtio_write_features: Write driver-accepted features
;   - asm_virtio_read_mac: Read MAC address from config space
;   - asm_virtio_init_device: Full init sequence
;
; VirtIO Status Bits:
;   0x01 - ACKNOWLEDGE     Driver found device
;   0x02 - DRIVER          Driver knows how to drive device
;   0x04 - DRIVER_OK       Driver ready, device may operate
;   0x08 - FEATURES_OK     Feature negotiation complete
;   0x40 - DEVICE_NEEDS_RESET  Device error
;   0x80 - FAILED          Driver gave up
;
; VirtIO MMIO Register Offsets (Modern Device):
;   0x000 - MagicValue (0x74726976 = "virt")
;   0x004 - Version (2 for modern)
;   0x008 - DeviceID
;   0x00C - VendorID
;   0x010 - DeviceFeatures
;   0x014 - DeviceFeaturesSel
;   0x020 - DriverFeatures
;   0x024 - DriverFeaturesSel
;   0x030 - QueueSel
;   0x034 - QueueNumMax
;   0x038 - QueueNum
;   0x044 - QueueReady
;   0x050 - QueueNotify
;   0x060 - InterruptStatus
;   0x064 - InterruptACK
;   0x070 - Status
;   0x080 - QueueDescLow
;   0x084 - QueueDescHigh
;   0x090 - QueueDriverLow
;   0x094 - QueueDriverHigh
;   0x0A0 - QueueDeviceLow
;   0x0A4 - QueueDeviceHigh
;   0x100+ - Config space
;
; Reference: VirtIO Spec 1.2, NETWORK_IMPL_GUIDE.md §4.3-4.5
; ═══════════════════════════════════════════════════════════════════════════

section .data
    ; VirtIO MMIO register offsets
    VIRTIO_MMIO_MAGIC           equ 0x000
    VIRTIO_MMIO_VERSION         equ 0x004
    VIRTIO_MMIO_DEVICE_ID       equ 0x008
    VIRTIO_MMIO_VENDOR_ID       equ 0x00C
    VIRTIO_MMIO_DEVICE_FEATURES equ 0x010
    VIRTIO_MMIO_DEVICE_FEATURES_SEL equ 0x014
    VIRTIO_MMIO_DRIVER_FEATURES equ 0x020
    VIRTIO_MMIO_DRIVER_FEATURES_SEL equ 0x024
    VIRTIO_MMIO_QUEUE_SEL       equ 0x030
    VIRTIO_MMIO_QUEUE_NUM_MAX   equ 0x034
    VIRTIO_MMIO_QUEUE_NUM       equ 0x038
    VIRTIO_MMIO_QUEUE_READY     equ 0x044
    VIRTIO_MMIO_QUEUE_NOTIFY    equ 0x050
    VIRTIO_MMIO_INTERRUPT_STATUS equ 0x060
    VIRTIO_MMIO_INTERRUPT_ACK   equ 0x064
    VIRTIO_MMIO_STATUS          equ 0x070
    VIRTIO_MMIO_QUEUE_DESC_LOW  equ 0x080
    VIRTIO_MMIO_QUEUE_DESC_HIGH equ 0x084
    VIRTIO_MMIO_QUEUE_DRIVER_LOW equ 0x090
    VIRTIO_MMIO_QUEUE_DRIVER_HIGH equ 0x094
    VIRTIO_MMIO_QUEUE_DEVICE_LOW equ 0x0A0
    VIRTIO_MMIO_QUEUE_DEVICE_HIGH equ 0x0A4
    VIRTIO_MMIO_CONFIG          equ 0x100
    
    ; VirtIO status bits
    VIRTIO_STATUS_ACKNOWLEDGE   equ 0x01
    VIRTIO_STATUS_DRIVER        equ 0x02
    VIRTIO_STATUS_DRIVER_OK     equ 0x04
    VIRTIO_STATUS_FEATURES_OK   equ 0x08
    VIRTIO_STATUS_DEVICE_NEEDS_RESET equ 0x40
    VIRTIO_STATUS_FAILED        equ 0x80
    
    ; VirtIO net feature bits (low 32)
    VIRTIO_NET_F_MAC            equ (1 << 5)
    VIRTIO_NET_F_STATUS         equ (1 << 16)
    
    ; VirtIO feature bits (high 32, when feature_sel=1)
    VIRTIO_F_VERSION_1          equ (1 << 0)    ; Bit 32 = high[0]
    
    ; Magic value
    VIRTIO_MAGIC                equ 0x74726976  ; "virt"

section .text

; External: Core primitives
extern asm_tsc_read
extern asm_bar_mfence

; Export symbols
global asm_virtio_reset
global asm_virtio_set_status
global asm_virtio_get_status
global asm_virtio_read_features
global asm_virtio_write_features
global asm_virtio_read_mac
global asm_virtio_verify_magic
global asm_virtio_get_version
global asm_virtio_get_device_id

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_verify_magic
; ───────────────────────────────────────────────────────────────────────────
; Verify VirtIO magic value at MMIO base
;
; Parameters:
;   RCX = MMIO base address
; Returns:
;   EAX = 1 if valid VirtIO device, 0 otherwise
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_verify_magic:
    mov     eax, [rcx + VIRTIO_MMIO_MAGIC]
    cmp     eax, VIRTIO_MAGIC
    jne     .invalid
    mov     eax, 1
    ret
.invalid:
    xor     eax, eax
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_get_version
; ───────────────────────────────────────────────────────────────────────────
; Get VirtIO device version
;
; Parameters:
;   RCX = MMIO base address
; Returns:
;   EAX = version (2 = modern)
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_get_version:
    mov     eax, [rcx + VIRTIO_MMIO_VERSION]
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_get_device_id
; ───────────────────────────────────────────────────────────────────────────
; Get VirtIO device ID (1 = network, 2 = block, etc.)
;
; Parameters:
;   RCX = MMIO base address
; Returns:
;   EAX = device ID
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_get_device_id:
    mov     eax, [rcx + VIRTIO_MMIO_DEVICE_ID]
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_get_status
; ───────────────────────────────────────────────────────────────────────────
; Read VirtIO device status register
;
; Parameters:
;   RCX = MMIO base address
; Returns:
;   EAX = status byte (zero-extended)
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_get_status:
    mov     eax, [rcx + VIRTIO_MMIO_STATUS]
    and     eax, 0xFF
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_set_status
; ───────────────────────────────────────────────────────────────────────────
; Write VirtIO device status register
;
; Parameters:
;   RCX = MMIO base address
;   DL  = status value
; Returns: None
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_set_status:
    movzx   edx, dl
    mov     [rcx + VIRTIO_MMIO_STATUS], edx
    mfence                      ; Ensure write completes before return
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_reset
; ───────────────────────────────────────────────────────────────────────────
; Reset VirtIO device and wait for reset to complete
;
; Parameters:
;   RCX = MMIO base address
;   RDX = TSC frequency (for timeout calculation)
; Returns:
;   EAX = 0 on success, 1 on timeout
;
; The reset is complete when the status register reads 0.
; Timeout: 100ms (bounded wait per AUDIT §7.2.3)
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_reset:
    push    rbx
    push    r12
    push    r13
    
    mov     r12, rcx            ; Save MMIO base
    mov     r13, rdx            ; Save TSC frequency
    
    ; Write 0 to status register to reset
    xor     edx, edx
    mov     [rcx + VIRTIO_MMIO_STATUS], edx
    mfence
    
    ; Get start time
    call    asm_tsc_read
    mov     rbx, rax            ; RBX = start TSC
    
    ; Calculate timeout: 100ms = tsc_freq / 10
    mov     rax, r13
    xor     edx, edx
    mov     rcx, 10
    div     rcx                 ; RAX = tsc_freq / 10
    mov     r13, rax            ; R13 = timeout ticks
    
.wait_loop:
    ; Check if status is 0
    mov     eax, [r12 + VIRTIO_MMIO_STATUS]
    and     eax, 0xFF
    test    eax, eax
    jz      .success
    
    ; Check timeout
    call    asm_tsc_read
    sub     rax, rbx            ; Elapsed ticks
    cmp     rax, r13
    ja      .timeout
    
    ; Brief pause before retry
    pause
    jmp     .wait_loop
    
.success:
    xor     eax, eax            ; Return 0 (success)
    jmp     .done
    
.timeout:
    mov     eax, 1              ; Return 1 (timeout)
    
.done:
    pop     r13
    pop     r12
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_read_features
; ───────────────────────────────────────────────────────────────────────────
; Read 64-bit device feature bits
;
; Parameters:
;   RCX = MMIO base address
; Returns:
;   RAX = 64-bit feature bits
;
; Modern VirtIO uses feature selection:
;   - Write 0 to FeaturesSel, read low 32 bits
;   - Write 1 to FeaturesSel, read high 32 bits
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_read_features:
    push    rbx
    
    ; Select low 32 bits (feature_sel = 0)
    xor     eax, eax
    mov     [rcx + VIRTIO_MMIO_DEVICE_FEATURES_SEL], eax
    mfence
    
    ; Read low 32 bits
    mov     ebx, [rcx + VIRTIO_MMIO_DEVICE_FEATURES]
    
    ; Select high 32 bits (feature_sel = 1)
    mov     eax, 1
    mov     [rcx + VIRTIO_MMIO_DEVICE_FEATURES_SEL], eax
    mfence
    
    ; Read high 32 bits
    mov     eax, [rcx + VIRTIO_MMIO_DEVICE_FEATURES]
    
    ; Combine: RAX = (high << 32) | low
    shl     rax, 32
    or      rax, rbx
    
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_write_features
; ───────────────────────────────────────────────────────────────────────────
; Write 64-bit driver-accepted feature bits
;
; Parameters:
;   RCX = MMIO base address
;   RDX = 64-bit feature bits to write
; Returns: None
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_write_features:
    push    rbx
    mov     rbx, rdx            ; Save features
    
    ; Write low 32 bits (feature_sel = 0)
    xor     eax, eax
    mov     [rcx + VIRTIO_MMIO_DRIVER_FEATURES_SEL], eax
    mfence
    mov     eax, ebx            ; Low 32 bits
    mov     [rcx + VIRTIO_MMIO_DRIVER_FEATURES], eax
    mfence
    
    ; Write high 32 bits (feature_sel = 1)
    mov     eax, 1
    mov     [rcx + VIRTIO_MMIO_DRIVER_FEATURES_SEL], eax
    mfence
    shr     rbx, 32             ; High 32 bits
    mov     [rcx + VIRTIO_MMIO_DRIVER_FEATURES], ebx
    mfence
    
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_read_mac
; ───────────────────────────────────────────────────────────────────────────
; Read MAC address from VirtIO-net config space
;
; Parameters:
;   RCX = MMIO base address
;   RDX = pointer to 6-byte buffer for MAC address
; Returns:
;   EAX = 0 on success
;
; MAC address is at config space offset 0 (MMIO base + 0x100)
; Note: Only valid if VIRTIO_NET_F_MAC feature was negotiated
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_read_mac:
    ; Config space starts at offset 0x100
    ; MAC address is first 6 bytes
    mov     al, [rcx + VIRTIO_MMIO_CONFIG + 0]
    mov     [rdx + 0], al
    mov     al, [rcx + VIRTIO_MMIO_CONFIG + 1]
    mov     [rdx + 1], al
    mov     al, [rcx + VIRTIO_MMIO_CONFIG + 2]
    mov     [rdx + 2], al
    mov     al, [rcx + VIRTIO_MMIO_CONFIG + 3]
    mov     [rdx + 3], al
    mov     al, [rcx + VIRTIO_MMIO_CONFIG + 4]
    mov     [rdx + 4], al
    mov     al, [rcx + VIRTIO_MMIO_CONFIG + 5]
    mov     [rdx + 5], al
    
    xor     eax, eax
    ret
