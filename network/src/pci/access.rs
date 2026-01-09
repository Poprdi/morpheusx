//! PCI access abstraction.
//!
//! Wrapper over Legacy (CF8/CFC) and ECAM access methods.
//!
//! # Reference
//! ARCHITECTURE_V3.md

// TODO: Implement PciAccess
//
// pub trait PciAccess {
//     fn read32(&self, addr: PciAddress, reg: u8) -> u32;
//     fn write32(&self, addr: PciAddress, reg: u8, value: u32);
//     fn read16(&self, addr: PciAddress, reg: u8) -> u16;
//     fn write16(&self, addr: PciAddress, reg: u8, value: u16);
// }
//
// pub struct LegacyPciAccess;
// pub struct EcamPciAccess { ecam_base: u64 }
//
// pub enum PciAccessMethod {
//     Legacy(LegacyPciAccess),
//     Ecam(EcamPciAccess),
// }
