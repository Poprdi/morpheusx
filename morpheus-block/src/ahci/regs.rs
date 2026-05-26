//! AHCI 1.3.1 register map mirrored on the Rust side.

/// HBA generic registers (offset from ABAR).
pub mod hba {
    pub const CAP: u64 = 0x00;
    pub const GHC: u64 = 0x04;
    pub const IS: u64 = 0x08;
    pub const PI: u64 = 0x0C;
    pub const VS: u64 = 0x10;
}

/// Port register block at ABAR + 0x100 + port * 0x80.
pub mod port {
    pub const CLB: u64 = 0x00;
    pub const CLBU: u64 = 0x04;
    pub const FB: u64 = 0x08;
    pub const FBU: u64 = 0x0C;
    pub const IS: u64 = 0x10;
    pub const IE: u64 = 0x14;
    pub const CMD: u64 = 0x18;
    pub const TFD: u64 = 0x20;
    pub const SIG: u64 = 0x24;
    pub const SSTS: u64 = 0x28;
    pub const SCTL: u64 = 0x2C;
    pub const SERR: u64 = 0x30;
    pub const SACT: u64 = 0x34;
    pub const CI: u64 = 0x38;
}

pub mod ghc {
    pub const HR: u32 = 1 << 0;
    pub const IE: u32 = 1 << 1;
    pub const AE: u32 = 1 << 31;
}

pub mod cmd {
    pub const ST: u32 = 1 << 0;
    pub const SUD: u32 = 1 << 1;
    pub const POD: u32 = 1 << 2;
    pub const CLO: u32 = 1 << 3;
    pub const FRE: u32 = 1 << 4;
    pub const FR: u32 = 1 << 14;
    pub const CR: u32 = 1 << 15;
}

pub mod tfd {
    pub const STS_ERR: u32 = 1 << 0;
    pub const STS_DRQ: u32 = 1 << 3;
    pub const STS_BSY: u32 = 1 << 7;
}

pub mod pxis {
    pub const DHRS: u32 = 1 << 0;
    pub const PSS: u32 = 1 << 1;
    pub const DSS: u32 = 1 << 2;
    pub const SDBS: u32 = 1 << 3;
    pub const TFES: u32 = 1 << 30;
}

pub mod ata {
    pub const READ_DMA_EXT: u8 = 0x25;
    pub const WRITE_DMA_EXT: u8 = 0x35;
    pub const IDENTIFY: u8 = 0xEC;
    pub const FLUSH_CACHE_EXT: u8 = 0xEA;
}

pub mod fis {
    pub const REG_H2D: u8 = 0x27;
    pub const REG_D2H: u8 = 0x34;
    pub const DMA_ACTIVATE: u8 = 0x39;
    pub const DMA_SETUP: u8 = 0x41;
    pub const DATA: u8 = 0x46;
    pub const PIO_SETUP: u8 = 0x5F;
    pub const DEV_BITS: u8 = 0xA1;
}

pub mod align {
    pub const CMD_LIST: usize = 1024;
    pub const FIS: usize = 256;
    pub const CMD_TABLE: usize = 128;
}

pub mod size {
    pub const CMD_LIST: usize = 1024;
    pub const FIS: usize = 256;
    pub const CMD_TABLE: usize = 256;
    pub const CMD_HEADER: usize = 32;
    pub const PRDT_ENTRY: usize = 16;
}

pub mod signature {
    pub const ATA: u32 = 0x00000101;
    pub const ATAPI: u32 = 0xEB140101;
    pub const SEMB: u32 = 0xC33C0101;
    pub const PM: u32 = 0x96690101;
}
