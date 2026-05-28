//! NIC register/bit constants. See vendor datasheets.

/// Intel e1000/e1000e (PCH datasheet 8.x).
pub mod intel {
    pub const CTRL: u32 = 0x0000;
    pub const CTRL_SLU: u32 = 1 << 6;
    pub const CTRL_RST: u32 = 1 << 26;

    pub const STATUS: u32 = 0x0008;
    pub const STATUS_LU: u32 = 1 << 1;

    pub const ICR: u32 = 0x00C0;
    pub const IMS: u32 = 0x00D0;
    pub const IMC: u32 = 0x00D8;

    pub const RCTL: u32 = 0x0100;
    pub const RCTL_EN: u32 = 1 << 1;
    pub const RCTL_BAM: u32 = 1 << 15;

    pub const RDBAL: u32 = 0x2800;
    pub const RDBAH: u32 = 0x2804;
    pub const RDLEN: u32 = 0x2808;
    pub const RDH: u32 = 0x2810;
    pub const RDT: u32 = 0x2818;

    pub const TCTL: u32 = 0x0400;
    pub const TCTL_EN: u32 = 1 << 1;
    pub const TCTL_PSP: u32 = 1 << 3;

    pub const TDBAL: u32 = 0x3800;
    pub const TDBAH: u32 = 0x3804;
    pub const TDLEN: u32 = 0x3808;
    pub const TDH: u32 = 0x3810;
    pub const TDT: u32 = 0x3818;

    pub const RAL: u32 = 0x5400;
    pub const RAH: u32 = 0x5404;
}

/// Realtek RTL8111/8168/8125.
pub mod realtek {
    pub const IDR0: u32 = 0x00;
    pub const IDR4: u32 = 0x04;

    pub const CR: u32 = 0x37;
    pub const CR_RST: u8 = 1 << 4;
    pub const CR_RE: u8 = 1 << 3;
    pub const CR_TE: u8 = 1 << 2;

    pub const TCR: u32 = 0x40;

    pub const RCR: u32 = 0x44;
    pub const RCR_AAP: u32 = 1 << 0;
    pub const RCR_APM: u32 = 1 << 1;
    pub const RCR_AM: u32 = 1 << 2;
    pub const RCR_AB: u32 = 1 << 3;

    pub const IMR: u32 = 0x3C;
    pub const ISR: u32 = 0x3E;

    pub const TNPDS: u32 = 0x20;
    pub const RDSAR: u32 = 0xE4;
}

/// Broadcom tg3. Per-chip-revision; unimplemented.
pub mod broadcom {}

/// VirtIO 1.1 §5.1 (net).
pub mod virtio {
    pub const VENDOR_ID: u16 = 0x1AF4;
    pub const DEVICE_ID_LEGACY: u16 = 0x1000;
    pub const DEVICE_ID_MODERN: u16 = 0x1041;

    pub const STATUS_ACK: u8 = 1;
    pub const STATUS_DRIVER: u8 = 2;
    pub const STATUS_DRIVER_OK: u8 = 4;
    pub const STATUS_FEATURES_OK: u8 = 8;

    pub const NET_F_MAC: u64 = 1 << 5;
    pub const NET_F_STATUS: u64 = 1 << 16;
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
