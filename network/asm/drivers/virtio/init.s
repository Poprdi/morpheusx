; ═══════════════════════════════════════════════════════════════════════════
; VirtIO Device Initialization
; ABI: Microsoft x64 (RCX, RDX, R8, R9, stack)
; ═══════════════════════════════════════════════════════════════════════════
;
; TODO: Implement the following functions:
;   - asm_nic_reset: Reset VirtIO device (write 0, wait for 0)
;   - asm_nic_set_status: Write VirtIO status register
;   - asm_nic_get_status: Read VirtIO status register
;   - asm_nic_read_features: Read device feature bits (64-bit)
;   - asm_nic_write_features: Write driver-accepted features
;   - asm_nic_read_mac: Read MAC address from config space
;
; VirtIO Status Bits:
;   0x01 - ACKNOWLEDGE
;   0x02 - DRIVER
;   0x04 - DRIVER_OK
;   0x08 - FEATURES_OK
;   0x40 - DEVICE_NEEDS_RESET
;   0x80 - FAILED
;
; Reference: NETWORK_IMPL_GUIDE.md §4.3, VirtIO Spec §3.1
; ═══════════════════════════════════════════════════════════════════════════

section .text
