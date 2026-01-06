//! Hardware register definitions for NIC drivers.
//!
//! This module contains register offsets, bit masks, and constants
//! for various NIC hardware. Organized by vendor/device family.

/// Intel e1000/e1000e register definitions.
pub mod intel {
    // Device Control Register
    pub const CTRL: u32 = 0x0000;
    pub const CTRL_SLU: u32 = 1 << 6; // Set Link Up
    pub const CTRL_RST: u32 = 1 << 26; // Device Reset

    // Device Status Register
    pub const STATUS: u32 = 0x0008;
    pub const STATUS_LU: u32 = 1 << 1; // Link Up

    // Interrupt registers
    pub const ICR: u32 = 0x00C0; // Interrupt Cause Read
    pub const IMS: u32 = 0x00D0; // Interrupt Mask Set
    pub const IMC: u32 = 0x00D8; // Interrupt Mask Clear

    // Receive registers
    pub const RCTL: u32 = 0x0100; // Receive Control
    pub const RCTL_EN: u32 = 1 << 1; // Receive Enable
    pub const RCTL_BAM: u32 = 1 << 15; // Broadcast Accept Mode

    pub const RDBAL: u32 = 0x2800; // RX Descriptor Base Low
    pub const RDBAH: u32 = 0x2804; // RX Descriptor Base High
    pub const RDLEN: u32 = 0x2808; // RX Descriptor Length
    pub const RDH: u32 = 0x2810; // RX Descriptor Head
    pub const RDT: u32 = 0x2818; // RX Descriptor Tail

    // Transmit registers
    pub const TCTL: u32 = 0x0400; // Transmit Control
    pub const TCTL_EN: u32 = 1 << 1; // Transmit Enable
    pub const TCTL_PSP: u32 = 1 << 3; // Pad Short Packets

    pub const TDBAL: u32 = 0x3800; // TX Descriptor Base Low
    pub const TDBAH: u32 = 0x3804; // TX Descriptor Base High
    pub const TDLEN: u32 = 0x3808; // TX Descriptor Length
    pub const TDH: u32 = 0x3810; // TX Descriptor Head
    pub const TDT: u32 = 0x3818; // TX Descriptor Tail

    // MAC address
    pub const RAL: u32 = 0x5400; // Receive Address Low
    pub const RAH: u32 = 0x5404; // Receive Address High
}

/// Realtek RTL8111/8168/8125 register definitions.
pub mod realtek {
    // MAC address registers
    pub const IDR0: u32 = 0x00; // MAC address byte 0-3
    pub const IDR4: u32 = 0x04; // MAC address byte 4-5

    // Command register
    pub const CR: u32 = 0x37;
    pub const CR_RST: u8 = 1 << 4; // Reset
    pub const CR_RE: u8 = 1 << 3; // Receive Enable
    pub const CR_TE: u8 = 1 << 2; // Transmit Enable

    // Transmit Configuration
    pub const TCR: u32 = 0x40;

    // Receive Configuration
    pub const RCR: u32 = 0x44;
    pub const RCR_AAP: u32 = 1 << 0; // Accept All Packets
    pub const RCR_APM: u32 = 1 << 1; // Accept Physical Match
    pub const RCR_AM: u32 = 1 << 2; // Accept Multicast
    pub const RCR_AB: u32 = 1 << 3; // Accept Broadcast

    // Interrupt registers
    pub const IMR: u32 = 0x3C; // Interrupt Mask Register
    pub const ISR: u32 = 0x3E; // Interrupt Status Register

    // Descriptor addresses (8111/8168)
    pub const TNPDS: u32 = 0x20; // TX Normal Priority Descriptor Start
    pub const RDSAR: u32 = 0xE4; // RX Descriptor Start Address
}

/// Broadcom register definitions (tg3).
pub mod broadcom {
    // TODO: Add Broadcom register definitions
    // These are more complex and vary by chip revision
}

/// VirtIO-net constants.
pub mod virtio {
    // PCI IDs
    pub const VENDOR_ID: u16 = 0x1AF4;
    pub const DEVICE_ID_LEGACY: u16 = 0x1000;
    pub const DEVICE_ID_MODERN: u16 = 0x1041;

    // Device status bits
    pub const STATUS_ACK: u8 = 1;
    pub const STATUS_DRIVER: u8 = 2;
    pub const STATUS_DRIVER_OK: u8 = 4;
    pub const STATUS_FEATURES_OK: u8 = 8;

    // Feature bits for net device
    pub const NET_F_MAC: u64 = 1 << 5; // Device has given MAC address
    pub const NET_F_STATUS: u64 = 1 << 16; // Configuration status available
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intel_register_offsets() {
        assert_eq!(intel::CTRL, 0x0000);
        assert_eq!(intel::STATUS, 0x0008);
        assert_eq!(intel::RDBAL, 0x2800);
        assert_eq!(intel::TDBAL, 0x3800);
    }

    #[test]
    fn test_realtek_register_offsets() {
        assert_eq!(realtek::IDR0, 0x00);
        assert_eq!(realtek::CR, 0x37);
    }

    #[test]
    fn test_virtio_ids() {
        assert_eq!(virtio::VENDOR_ID, 0x1AF4);
    }
}
