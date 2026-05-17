//! xHCI register definitions — spec 1.2

// Capability registers (offset from BAR0)
pub const CAP_CAPLENGTH: u64 = 0x00;
pub const CAP_HCIVERSION: u64 = 0x02;
pub const CAP_HCSPARAMS1: u64 = 0x04;
pub const CAP_HCSPARAMS2: u64 = 0x08;
pub const CAP_HCCPARAMS1: u64 = 0x10;
pub const CAP_DBOFF: u64 = 0x14;
pub const CAP_RTSOFF: u64 = 0x18;

// Operational registers (offset from op_base = BAR0 + CAPLENGTH)
pub const OP_USBCMD: u64 = 0x00;
pub const OP_USBSTS: u64 = 0x04;
pub const OP_CRCR: u64 = 0x18;
pub const OP_DCBAAP: u64 = 0x30;
pub const OP_CONFIG: u64 = 0x38;

pub const PORT_REG_BASE: u64 = 0x400;
pub const PORT_REG_STRIDE: u64 = 0x10;

// Runtime register offsets (from rt_base)
pub const RT_IR0_IMAN: u64 = 0x20;
pub const RT_IR0_IMOD: u64 = 0x24;
pub const RT_IR0_ERSTSZ: u64 = 0x28;
pub const RT_IR0_ERSTBA: u64 = 0x30;
pub const RT_IR0_ERDP: u64 = 0x38;

// USBCMD bits
pub const CMD_RS: u32 = 1 << 0;
pub const CMD_HCRST: u32 = 1 << 1;
pub const CMD_INTE: u32 = 1 << 2;

// USBSTS bits
pub const STS_HCH: u32 = 1 << 0;
pub const STS_CNR: u32 = 1 << 11;

// PORTSC bits
pub const PORTSC_CCS: u32 = 1 << 0;
pub const PORTSC_PED: u32 = 1 << 1;
pub const PORTSC_PR: u32 = 1 << 4;
pub const PORTSC_PLS_MASK: u32 = 0xF << 5;
pub const PORTSC_PP: u32 = 1 << 9;
pub const PORTSC_LWS: u32 = 1 << 16;
pub const PORTSC_PRC: u32 = 1 << 21;
pub const PORTSC_CAS: u32 = 1 << 24;
pub const PORTSC_WPR: u32 = 1 << 31;
pub const PORTSC_RW1C: u32 = 0x00FE_0000;
pub const PORTSC_SPEED_SHIFT: u32 = 10;

pub const PLS_U0: u32 = 0x0 << 5;
pub const PLS_U3: u32 = 0x3 << 5;
pub const PLS_RECOVERY: u32 = 0x8 << 5;
pub const PLS_RESUME: u32 = 0xF << 5;
pub const PLS_INACTIVE: u32 = 0x6 << 5;
pub const PLS_POLLING: u32 = 0x7 << 5;
pub const PLS_COMPLIANCE: u32 = 0xA << 5;

// Extended capability IDs
pub const EXT_CAP_LEGACY: u8 = 1;

// USBLEGSUP bits
pub const LEGSUP_BIOS_OWNED: u32 = 1 << 16;
pub const LEGSUP_OS_OWNED: u32 = 1 << 24;

// TRB types (pre-shifted to bits 10:6)
pub const TRB_NORMAL: u32 = 1 << 10;
pub const TRB_SETUP: u32 = 2 << 10;
pub const TRB_DATA: u32 = 3 << 10;
pub const TRB_STATUS: u32 = 4 << 10;
pub const TRB_LINK: u32 = 6 << 10;
pub const TRB_ENABLE_SLOT: u32 = 9 << 10;
pub const TRB_ADDRESS_DEV: u32 = 11 << 10;
pub const TRB_CONFIGURE_EP: u32 = 12 << 10;
pub const TRB_TRANSFER_EVENT: u32 = 32 << 10;
pub const TRB_CMD_COMPLETE: u32 = 33 << 10;

// TRB control bits
pub const TRB_TC: u32 = 1 << 1;
pub const TRB_ISP: u32 = 1 << 2;
pub const TRB_IOC: u32 = 1 << 5;
pub const TRB_IDT: u32 = 1 << 6;
pub const TRB_DIR_IN: u32 = 1 << 16;
pub const TRB_TRT_IN: u32 = 3 << 16;
pub const TRB_TYPE_MASK: u32 = 0x3F << 10;