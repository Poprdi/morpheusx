; ═══════════════════════════════════════════════════════════════════════════
; VirtIO PCI Modern Transport Operations
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; VirtIO PCI Modern uses capability structures to locate device registers
; in PCI BARs. This is different from VirtIO MMIO which has fixed offsets.
;
; PCI Modern common_cfg layout (per VirtIO 1.2 spec §4.1.4.3):
;   Offset  Size  Name
;   0x00    4     device_feature_select
;   0x04    4     device_feature
;   0x08    4     driver_feature_select
;   0x0C    4     driver_feature
;   0x10    2     msix_config
;   0x12    2     num_queues
;   0x14    1     device_status
;   0x15    1     config_generation
;   0x16    2     queue_select
;   0x18    2     queue_size
;   0x1A    2     queue_msix_vector
;   0x1C    2     queue_enable
;   0x1E    2     queue_notify_off
;   0x20    8     queue_desc
;   0x28    8     queue_driver (avail)
;   0x30    8     queue_device (used)
;
; ═══════════════════════════════════════════════════════════════════════════

section .data
    ; PCI Modern common_cfg offsets
    VIRTIO_PCI_COMMON_DFSELECT      equ 0x00    ; device_feature_select
    VIRTIO_PCI_COMMON_DF            equ 0x04    ; device_feature
    VIRTIO_PCI_COMMON_GFSELECT      equ 0x08    ; driver_feature_select
    VIRTIO_PCI_COMMON_GF            equ 0x0C    ; driver_feature
    VIRTIO_PCI_COMMON_MSIX_CONFIG   equ 0x10
    VIRTIO_PCI_COMMON_NUMQUEUES     equ 0x12
    VIRTIO_PCI_COMMON_STATUS        equ 0x14    ; device_status (1 byte!)
    VIRTIO_PCI_COMMON_CFGGEN        equ 0x15    ; config_generation
    VIRTIO_PCI_COMMON_Q_SELECT      equ 0x16    ; queue_select
    VIRTIO_PCI_COMMON_Q_SIZE        equ 0x18    ; queue_size
    VIRTIO_PCI_COMMON_Q_MSIX        equ 0x1A    ; queue_msix_vector
    VIRTIO_PCI_COMMON_Q_ENABLE      equ 0x1C    ; queue_enable
    VIRTIO_PCI_COMMON_Q_NOFF        equ 0x1E    ; queue_notify_off
    VIRTIO_PCI_COMMON_Q_DESCLO      equ 0x20    ; queue_desc low
    VIRTIO_PCI_COMMON_Q_DESCHI      equ 0x24    ; queue_desc high
    VIRTIO_PCI_COMMON_Q_AVAILLO     equ 0x28    ; queue_driver low
    VIRTIO_PCI_COMMON_Q_AVAILHI     equ 0x2C    ; queue_driver high
    VIRTIO_PCI_COMMON_Q_USEDLO      equ 0x30    ; queue_device low
    VIRTIO_PCI_COMMON_Q_USEDHI      equ 0x34    ; queue_device high

    ; VirtIO status bits
    VIRTIO_STATUS_ACKNOWLEDGE       equ 0x01
    VIRTIO_STATUS_DRIVER            equ 0x02
    VIRTIO_STATUS_DRIVER_OK         equ 0x04
    VIRTIO_STATUS_FEATURES_OK       equ 0x08
    VIRTIO_STATUS_DEVICE_NEEDS_RESET equ 0x40
    VIRTIO_STATUS_FAILED            equ 0x80

section .text

; External: Core primitives
extern asm_tsc_read
extern asm_bar_mfence

; Export symbols
global asm_virtio_pci_get_status
global asm_virtio_pci_set_status
global asm_virtio_pci_reset
global asm_virtio_pci_read_features
global asm_virtio_pci_write_features
global asm_virtio_pci_get_num_queues
global asm_virtio_pci_select_queue
global asm_virtio_pci_get_queue_size
global asm_virtio_pci_set_queue_size
global asm_virtio_pci_enable_queue
global asm_virtio_pci_set_queue_desc
global asm_virtio_pci_set_queue_avail
global asm_virtio_pci_set_queue_used
global asm_virtio_pci_get_queue_notify_off
global asm_virtio_pci_notify_queue

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_get_status
; ───────────────────────────────────────────────────────────────────────────
; Read VirtIO device status from PCI Modern common_cfg
;
; Parameters:
;   RCX = common_cfg base address (BAR + offset from capability)
; Returns:
;   EAX = status byte (zero-extended)
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_pci_get_status:
    movzx   eax, byte [rcx + VIRTIO_PCI_COMMON_STATUS]
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_set_status
; ───────────────────────────────────────────────────────────────────────────
; Write VirtIO device status to PCI Modern common_cfg
;
; Parameters:
;   RCX = common_cfg base address
;   DL  = status value
; Returns: None
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_pci_set_status:
    mov     byte [rcx + VIRTIO_PCI_COMMON_STATUS], dl
    mfence                          ; Ensure write completes
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_reset
; ───────────────────────────────────────────────────────────────────────────
; Reset VirtIO PCI device and wait for reset to complete
;
; Parameters:
;   RCX = common_cfg base address
;   RDX = TSC frequency (for timeout calculation)
; Returns:
;   EAX = 0 on success, 1 on timeout
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_pci_reset:
    push    rbx
    push    r12
    push    r13
    
    mov     r12, rcx                ; Save common_cfg base
    mov     r13, rdx                ; Save TSC frequency
    
    ; Write 0 to status to reset
    mov     byte [rcx + VIRTIO_PCI_COMMON_STATUS], 0
    mfence
    
    ; Get start time
    call    asm_tsc_read
    mov     rbx, rax                ; RBX = start TSC
    
    ; Calculate timeout: 100ms = tsc_freq / 10
    mov     rax, r13
    xor     edx, edx
    mov     rcx, 10
    div     rcx
    mov     r13, rax                ; R13 = timeout ticks
    
.wait_loop:
    ; Check if status is 0
    movzx   eax, byte [r12 + VIRTIO_PCI_COMMON_STATUS]
    test    al, al
    jz      .success
    
    ; Check timeout
    call    asm_tsc_read
    sub     rax, rbx
    cmp     rax, r13
    ja      .timeout
    
    pause
    jmp     .wait_loop
    
.success:
    xor     eax, eax
    jmp     .done
    
.timeout:
    mov     eax, 1
    
.done:
    pop     r13
    pop     r12
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_read_features
; ───────────────────────────────────────────────────────────────────────────
; Read 64-bit device feature bits from PCI Modern common_cfg
;
; Parameters:
;   RCX = common_cfg base address
; Returns:
;   RAX = 64-bit feature bits
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_pci_read_features:
    push    rbx
    
    ; Select low 32 bits (feature_select = 0)
    mov     dword [rcx + VIRTIO_PCI_COMMON_DFSELECT], 0
    mfence
    
    ; Read low 32 bits
    mov     ebx, [rcx + VIRTIO_PCI_COMMON_DF]
    
    ; Select high 32 bits (feature_select = 1)
    mov     dword [rcx + VIRTIO_PCI_COMMON_DFSELECT], 1
    mfence
    
    ; Read high 32 bits
    mov     eax, [rcx + VIRTIO_PCI_COMMON_DF]
    
    ; Combine: RAX = (high << 32) | low
    shl     rax, 32
    or      rax, rbx
    
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_write_features
; ───────────────────────────────────────────────────────────────────────────
; Write 64-bit driver-accepted feature bits to PCI Modern common_cfg
;
; Parameters:
;   RCX = common_cfg base address
;   RDX = 64-bit feature bits
; Returns: None
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_pci_write_features:
    push    rbx
    mov     rbx, rdx                ; Save features
    
    ; Write low 32 bits (feature_select = 0)
    mov     dword [rcx + VIRTIO_PCI_COMMON_GFSELECT], 0
    mfence
    mov     eax, ebx                ; Low 32 bits
    mov     [rcx + VIRTIO_PCI_COMMON_GF], eax
    mfence
    
    ; Write high 32 bits (feature_select = 1)
    mov     dword [rcx + VIRTIO_PCI_COMMON_GFSELECT], 1
    mfence
    shr     rbx, 32
    mov     [rcx + VIRTIO_PCI_COMMON_GF], ebx
    mfence
    
    pop     rbx
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_get_num_queues
; ───────────────────────────────────────────────────────────────────────────
; Get number of virtqueues supported by device
;
; Parameters:
;   RCX = common_cfg base address
; Returns:
;   EAX = number of queues
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_pci_get_num_queues:
    movzx   eax, word [rcx + VIRTIO_PCI_COMMON_NUMQUEUES]
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_select_queue
; ───────────────────────────────────────────────────────────────────────────
; Select a virtqueue for configuration
;
; Parameters:
;   RCX = common_cfg base address
;   DX  = queue index (0 = RX, 1 = TX for net)
; Returns: None
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_pci_select_queue:
    mov     word [rcx + VIRTIO_PCI_COMMON_Q_SELECT], dx
    mfence
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_get_queue_size
; ───────────────────────────────────────────────────────────────────────────
; Get maximum queue size for selected queue
;
; Parameters:
;   RCX = common_cfg base address
; Returns:
;   EAX = max queue size (0 = queue not available)
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_pci_get_queue_size:
    movzx   eax, word [rcx + VIRTIO_PCI_COMMON_Q_SIZE]
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_set_queue_size
; ───────────────────────────────────────────────────────────────────────────
; Set queue size for selected queue (must be <= max)
;
; Parameters:
;   RCX = common_cfg base address
;   DX  = queue size
; Returns: None
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_pci_set_queue_size:
    mov     word [rcx + VIRTIO_PCI_COMMON_Q_SIZE], dx
    mfence
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_enable_queue
; ───────────────────────────────────────────────────────────────────────────
; Enable the selected queue
;
; Parameters:
;   RCX = common_cfg base address
;   DX  = 1 to enable, 0 to disable
; Returns: None
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_pci_enable_queue:
    mov     word [rcx + VIRTIO_PCI_COMMON_Q_ENABLE], dx
    mfence
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_set_queue_desc
; ───────────────────────────────────────────────────────────────────────────
; Set descriptor table address for selected queue
;
; Parameters:
;   RCX = common_cfg base address
;   RDX = 64-bit physical address of descriptor table
; Returns: None
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_pci_set_queue_desc:
    mov     eax, edx                ; Low 32 bits
    mov     [rcx + VIRTIO_PCI_COMMON_Q_DESCLO], eax
    shr     rdx, 32
    mov     [rcx + VIRTIO_PCI_COMMON_Q_DESCHI], edx
    mfence
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_set_queue_avail
; ───────────────────────────────────────────────────────────────────────────
; Set available ring address for selected queue
;
; Parameters:
;   RCX = common_cfg base address
;   RDX = 64-bit physical address of available ring
; Returns: None
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_pci_set_queue_avail:
    mov     eax, edx
    mov     [rcx + VIRTIO_PCI_COMMON_Q_AVAILLO], eax
    shr     rdx, 32
    mov     [rcx + VIRTIO_PCI_COMMON_Q_AVAILHI], edx
    mfence
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_set_queue_used
; ───────────────────────────────────────────────────────────────────────────
; Set used ring address for selected queue
;
; Parameters:
;   RCX = common_cfg base address
;   RDX = 64-bit physical address of used ring
; Returns: None
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_pci_set_queue_used:
    mov     eax, edx
    mov     [rcx + VIRTIO_PCI_COMMON_Q_USEDLO], eax
    shr     rdx, 32
    mov     [rcx + VIRTIO_PCI_COMMON_Q_USEDHI], edx
    mfence
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_get_queue_notify_off
; ───────────────────────────────────────────────────────────────────────────
; Get notification offset multiplier for selected queue
;
; Parameters:
;   RCX = common_cfg base address
; Returns:
;   EAX = queue_notify_off value
;
; Note: Actual notify address = notify_cfg_base + queue_notify_off * notify_off_multiplier
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_pci_get_queue_notify_off:
    movzx   eax, word [rcx + VIRTIO_PCI_COMMON_Q_NOFF]
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_notify_queue
; ───────────────────────────────────────────────────────────────────────────
; Notify device that queue has new buffers available
;
; Parameters:
;   RCX = notify address (notify_cfg_base + queue_notify_off * multiplier)
;   DX  = queue index
; Returns: None
;
; For PCI Modern, we write the queue index to the notify address
; ───────────────────────────────────────────────────────────────────────────
asm_virtio_pci_notify_queue:
    mfence                          ; Ensure all prior writes visible
    mov     word [rcx], dx          ; Write queue index to notify address
    mfence
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_read_device_cfg_u8
; ───────────────────────────────────────────────────────────────────────────
; Read byte from device-specific config space
;
; Parameters:
;   RCX = device_cfg base address (BAR + offset from capability)
;   RDX = offset within device config
; Returns:
;   EAX = byte value (zero-extended)
; ───────────────────────────────────────────────────────────────────────────
global asm_virtio_pci_read_device_cfg_u8
asm_virtio_pci_read_device_cfg_u8:
    add     rcx, rdx
    movzx   eax, byte [rcx]
    ret

; ───────────────────────────────────────────────────────────────────────────
; asm_virtio_pci_read_mac
; ───────────────────────────────────────────────────────────────────────────
; Read MAC address from VirtIO-net device config
;
; Parameters:
;   RCX = device_cfg base address
;   RDX = pointer to 6-byte buffer
; Returns:
;   EAX = 0 on success
;
; VirtIO-net device config layout (offset 0-5 = MAC address)
; ───────────────────────────────────────────────────────────────────────────
global asm_virtio_pci_read_mac
asm_virtio_pci_read_mac:
    mov     al, [rcx + 0]
    mov     [rdx + 0], al
    mov     al, [rcx + 1]
    mov     [rdx + 1], al
    mov     al, [rcx + 2]
    mov     [rdx + 2], al
    mov     al, [rcx + 3]
    mov     [rdx + 3], al
    mov     al, [rcx + 4]
    mov     [rdx + 4], al
    mov     al, [rcx + 5]
    mov     [rdx + 5], al
    xor     eax, eax
    ret
