//! PCI type definitions.
//!
//! # Reference
//! PCI Local Bus Specification

// TODO: Implement PCI types
//
// #[derive(Debug, Clone, Copy)]
// pub struct PciAddress {
//     pub bus: u8,
//     pub device: u8,
//     pub function: u8,
// }
//
// #[derive(Debug, Clone)]
// pub struct PciDeviceInfo {
//     pub address: PciAddress,
//     pub vendor_id: u16,
//     pub device_id: u16,
//     pub class_code: u8,
//     pub subclass: u8,
//     pub prog_if: u8,
//     pub bar0: u64,
//     pub bar1: u64,
// }
//
// impl PciAddress {
//     pub fn config_addr(&self, reg: u8) -> u32 {
//         0x8000_0000
//             | ((self.bus as u32) << 16)
//             | ((self.device as u32) << 11)
//             | ((self.function as u32) << 8)
//             | ((reg as u32) & 0xFC)
//     }
// }
