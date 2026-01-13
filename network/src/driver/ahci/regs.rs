//! AHCI register definitions and constants.
//!
//! These are Rust-side constants matching the ASM definitions.

/// AHCI HBA registers (offset from ABAR)
pub mod hba {
    /// Host Capabilities
    pub const CAP: u64 = 0x00;
    /// Global Host Control
    pub const GHC: u64 = 0x04;
    /// Interrupt Status
    pub const IS: u64 = 0x08;
    /// Ports Implemented
    pub const PI: u64 = 0x0C;
    /// Version
    pub const VS: u64 = 0x10;
}

/// Port registers (offset from ABAR + 0x100 + port * 0x80)
pub mod port {
    /// Command List Base Address Low
    pub const CLB: u64 = 0x00;
    /// Command List Base Address High
    pub const CLBU: u64 = 0x04;
    /// FIS Base Address Low
    pub const FB: u64 = 0x08;
    /// FIS Base Address High
    pub const FBU: u64 = 0x0C;
    /// Interrupt Status
    pub const IS: u64 = 0x10;
    /// Interrupt Enable
    pub const IE: u64 = 0x14;
    /// Command and Status
    pub const CMD: u64 = 0x18;
    /// Task File Data
    pub const TFD: u64 = 0x20;
    /// Signature
    pub const SIG: u64 = 0x24;
    /// SATA Status
    pub const SSTS: u64 = 0x28;
    /// SATA Control
    pub const SCTL: u64 = 0x2C;
    /// SATA Error
    pub const SERR: u64 = 0x30;
    /// SATA Active
    pub const SACT: u64 = 0x34;
    /// Command Issue
    pub const CI: u64 = 0x38;
}

/// Global Host Control bits
pub mod ghc {
    pub const HR: u32 = 1 << 0;  // HBA Reset
    pub const IE: u32 = 1 << 1;  // Interrupt Enable
    pub const AE: u32 = 1 << 31; // AHCI Enable
}

/// Port Command bits
pub mod cmd {
    pub const ST: u32 = 1 << 0;   // Start
    pub const SUD: u32 = 1 << 1;  // Spin-Up Device
    pub const POD: u32 = 1 << 2;  // Power On Device
    pub const CLO: u32 = 1 << 3;  // Command List Override
    pub const FRE: u32 = 1 << 4;  // FIS Receive Enable
    pub const FR: u32 = 1 << 14;  // FIS Receive Running
    pub const CR: u32 = 1 << 15;  // Command List Running
}

/// Task File Data bits
pub mod tfd {
    pub const STS_ERR: u32 = 1 << 0;
    pub const STS_DRQ: u32 = 1 << 3;
    pub const STS_BSY: u32 = 1 << 7;
}

/// Port Interrupt Status bits
pub mod pxis {
    pub const DHRS: u32 = 1 << 0;  // Device to Host Register FIS
    pub const PSS: u32 = 1 << 1;   // PIO Setup FIS
    pub const DSS: u32 = 1 << 2;   // DMA Setup FIS
    pub const SDBS: u32 = 1 << 3;  // Set Device Bits FIS
    pub const TFES: u32 = 1 << 30; // Task File Error
}

/// ATA commands
pub mod ata {
    pub const READ_DMA_EXT: u8 = 0x25;
    pub const WRITE_DMA_EXT: u8 = 0x35;
    pub const IDENTIFY: u8 = 0xEC;
    pub const FLUSH_CACHE_EXT: u8 = 0xEA;
}

/// FIS types
pub mod fis {
    pub const REG_H2D: u8 = 0x27;
    pub const REG_D2H: u8 = 0x34;
    pub const DMA_ACTIVATE: u8 = 0x39;
    pub const DMA_SETUP: u8 = 0x41;
    pub const DATA: u8 = 0x46;
    pub const PIO_SETUP: u8 = 0x5F;
    pub const DEV_BITS: u8 = 0xA1;
}

/// DMA structure alignment requirements
pub mod align {
    /// Command List alignment (1KB)
    pub const CMD_LIST: usize = 1024;
    /// FIS Receive buffer alignment (256 bytes)
    pub const FIS: usize = 256;
    /// Command Table alignment (128 bytes)
    pub const CMD_TABLE: usize = 128;
}

/// DMA structure sizes
pub mod size {
    /// Command List size (32 headers × 32 bytes)
    pub const CMD_LIST: usize = 1024;
    /// FIS Receive buffer size
    pub const FIS: usize = 256;
    /// Command Table size (header + PRDTs)
    pub const CMD_TABLE: usize = 256;
    /// Command Header size
    pub const CMD_HEADER: usize = 32;
    /// PRDT Entry size
    pub const PRDT_ENTRY: usize = 16;
}

/// Device signatures
pub mod signature {
    pub const ATA: u32 = 0x00000101;
    pub const ATAPI: u32 = 0xEB140101;
    pub const SEMB: u32 = 0xC33C0101;
    pub const PM: u32 = 0x96690101;
}
