//! VirtIO transport abstraction over MMIO, PCI Modern (1.0+), and PCI Legacy.
//! Probed at runtime from device discovery.

use crate::asm::device as mmio_device;

/// Consumers (virtio-blk, virtio-net) map this into their own error enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtioTransportError {
    /// e.g. device reports `queue_num_max == 0`.
    QueueSetupFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TransportType {
    Mmio = 0,
    PciModern = 1,
    /// Not fully supported.
    PciLegacy = 2,
}

/// PCI Modern config: each base = BAR + cap_offset.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
#[derive(Default)]
pub struct PciModernConfig {
    pub common_cfg: u64,
    pub notify_cfg: u64,
    pub notify_off_multiplier: u32,
    pub isr_cfg: u64,
    pub device_cfg: u64,
    pub pci_cfg: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct VirtioTransport {
    pub transport_type: TransportType,
    /// MMIO base, or common_cfg base for PCI Modern.
    pub base: u64,
    /// Valid only when transport_type == PciModern.
    pub pci_modern: PciModernConfig,
}

impl VirtioTransport {
    pub fn mmio(mmio_base: u64) -> Self {
        Self {
            transport_type: TransportType::Mmio,
            base: mmio_base,
            pci_modern: PciModernConfig::default(),
        }
    }

    pub fn pci_modern(config: PciModernConfig) -> Self {
        Self {
            transport_type: TransportType::PciModern,
            base: config.common_cfg,
            pci_modern: config,
        }
    }

    pub fn get_status(&self) -> u8 {
        match self.transport_type {
            TransportType::Mmio => mmio_device::get_status(self.base),
            TransportType::PciModern => unsafe { pci_modern::get_status(self.base) as u8 },
            TransportType::PciLegacy => 0, // Not supported
        }
    }

    pub fn set_status(&self, status: u8) {
        match self.transport_type {
            TransportType::Mmio => mmio_device::set_status(self.base, status),
            TransportType::PciModern => unsafe { pci_modern::set_status(self.base, status) },
            TransportType::PciLegacy => {}, // Not supported
        }
    }

    pub fn reset(&self, tsc_freq: u64) -> bool {
        // Note: MMIO reset doesn't use tsc_freq, PCI Modern does for timeout
        match self.transport_type {
            TransportType::Mmio => mmio_device::reset(self.base),
            TransportType::PciModern => unsafe { pci_modern::reset(self.base, tsc_freq) == 0 },
            TransportType::PciLegacy => false,
        }
    }

    pub fn read_features(&self) -> u64 {
        match self.transport_type {
            TransportType::Mmio => mmio_device::read_features(self.base),
            TransportType::PciModern => unsafe { pci_modern::read_features(self.base) },
            TransportType::PciLegacy => 0,
        }
    }

    pub fn write_features(&self, features: u64) {
        match self.transport_type {
            TransportType::Mmio => mmio_device::write_features(self.base, features),
            TransportType::PciModern => unsafe { pci_modern::write_features(self.base, features) },
            TransportType::PciLegacy => {},
        }
    }

    pub fn get_num_queues(&self) -> u16 {
        match self.transport_type {
            TransportType::PciModern => unsafe { pci_modern::get_num_queues(self.base) as u16 },
            _ => 2, // MMIO net default: RX + TX
        }
    }

    pub fn select_queue(&self, queue_idx: u16) {
        unsafe {
            match self.transport_type {
                TransportType::Mmio => {
                    let queue_sel_addr = self.base + 0x030; // QueueSel
                    core::ptr::write_volatile(queue_sel_addr as *mut u32, queue_idx as u32);
                    core::arch::asm!("mfence", options(nostack, preserves_flags));
                },
                TransportType::PciModern => pci_modern::select_queue(self.base, queue_idx),
                TransportType::PciLegacy => {},
            }
        }
    }

    pub fn get_queue_size(&self) -> u16 {
        unsafe {
            match self.transport_type {
                TransportType::Mmio => {
                    let queue_num_max_addr = self.base + 0x034;
                    core::ptr::read_volatile(queue_num_max_addr as *const u32) as u16
                },
                TransportType::PciModern => pci_modern::get_queue_size(self.base) as u16,
                TransportType::PciLegacy => 0,
            }
        }
    }

    pub fn set_queue_size(&self, size: u16) {
        unsafe {
            match self.transport_type {
                TransportType::Mmio => {
                    let queue_num_addr = self.base + 0x038;
                    core::ptr::write_volatile(queue_num_addr as *mut u32, size as u32);
                    core::arch::asm!("mfence", options(nostack, preserves_flags));
                },
                TransportType::PciModern => pci_modern::set_queue_size(self.base, size),
                TransportType::PciLegacy => {},
            }
        }
    }

    pub fn set_queue_desc(&self, addr: u64) {
        unsafe {
            match self.transport_type {
                TransportType::Mmio => {
                    let lo_addr = self.base + 0x080;
                    let hi_addr = self.base + 0x084;
                    core::ptr::write_volatile(lo_addr as *mut u32, addr as u32);
                    core::ptr::write_volatile(hi_addr as *mut u32, (addr >> 32) as u32);
                    core::arch::asm!("mfence", options(nostack, preserves_flags));
                },
                TransportType::PciModern => pci_modern::set_queue_desc(self.base, addr),
                TransportType::PciLegacy => {},
            }
        }
    }

    pub fn set_queue_avail(&self, addr: u64) {
        unsafe {
            match self.transport_type {
                TransportType::Mmio => {
                    let lo_addr = self.base + 0x090;
                    let hi_addr = self.base + 0x094;
                    core::ptr::write_volatile(lo_addr as *mut u32, addr as u32);
                    core::ptr::write_volatile(hi_addr as *mut u32, (addr >> 32) as u32);
                    core::arch::asm!("mfence", options(nostack, preserves_flags));
                },
                TransportType::PciModern => pci_modern::set_queue_avail(self.base, addr),
                TransportType::PciLegacy => {},
            }
        }
    }

    pub fn set_queue_used(&self, addr: u64) {
        unsafe {
            match self.transport_type {
                TransportType::Mmio => {
                    let lo_addr = self.base + 0x0A0;
                    let hi_addr = self.base + 0x0A4;
                    core::ptr::write_volatile(lo_addr as *mut u32, addr as u32);
                    core::ptr::write_volatile(hi_addr as *mut u32, (addr >> 32) as u32);
                    core::arch::asm!("mfence", options(nostack, preserves_flags));
                },
                TransportType::PciModern => pci_modern::set_queue_used(self.base, addr),
                TransportType::PciLegacy => {},
            }
        }
    }

    pub fn enable_queue(&self) {
        unsafe {
            match self.transport_type {
                TransportType::Mmio => {
                    let queue_ready_addr = self.base + 0x044;
                    core::ptr::write_volatile(queue_ready_addr as *mut u32, 1);
                    core::arch::asm!("mfence", options(nostack, preserves_flags));
                },
                TransportType::PciModern => pci_modern::enable_queue(self.base, 1),
                TransportType::PciLegacy => {},
            }
        }
    }

    pub fn get_notify_addr(&self, queue_idx: u16) -> u64 {
        match self.transport_type {
            TransportType::Mmio => self.base + 0x050, // fixed notify register
            TransportType::PciModern => {
                // notify_cfg + notify_off * multiplier, per queue
                unsafe {
                    pci_modern::select_queue(self.base, queue_idx);
                    let queue_notify_off = pci_modern::get_queue_notify_off(self.base);

                    self.pci_modern.notify_cfg
                        + (queue_notify_off as u64 * self.pci_modern.notify_off_multiplier as u64)
                }
            },
            TransportType::PciLegacy => 0,
        }
    }

    pub fn notify_queue(&self, queue_idx: u16) {
        unsafe {
            match self.transport_type {
                TransportType::Mmio => {
                    let notify_addr = self.base + 0x050;
                    core::arch::asm!("mfence", options(nostack, preserves_flags));
                    core::ptr::write_volatile(notify_addr as *mut u32, queue_idx as u32);
                },
                TransportType::PciModern => {
                    let notify_addr = self.get_notify_addr(queue_idx);
                    pci_modern::notify_queue(notify_addr, queue_idx);
                },
                TransportType::PciLegacy => {},
            }
        }
    }

    pub fn read_mac(&self, mac_out: &mut [u8; 6]) -> bool {
        match self.transport_type {
            TransportType::Mmio => {
                if let Some(mac) = mmio_device::read_mac(self.base) {
                    *mac_out = mac;
                    true
                } else {
                    false
                }
            },
            TransportType::PciModern => {
                if self.pci_modern.device_cfg != 0 {
                    unsafe {
                        pci_modern::read_mac(self.pci_modern.device_cfg, mac_out.as_mut_ptr());
                    }
                    true
                } else {
                    false
                }
            },
            TransportType::PciLegacy => false,
        }
    }

    /// Returns the notify address on success.
    pub fn setup_queue(
        &self,
        queue_idx: u16,
        desc_addr: u64,
        avail_addr: u64,
        used_addr: u64,
        queue_size: u16,
    ) -> Result<u64, VirtioTransportError> {
        self.select_queue(queue_idx);

        let max_size = self.get_queue_size();
        if max_size == 0 {
            return Err(VirtioTransportError::QueueSetupFailed);
        }

        let actual_size = queue_size.min(max_size);
        self.set_queue_size(actual_size);

        self.set_queue_desc(desc_addr);
        self.set_queue_avail(avail_addr);
        self.set_queue_used(used_addr);

        self.enable_queue();

        let notify_addr = self.get_notify_addr(queue_idx);

        Ok(notify_addr)
    }

    /// virtio-blk capacity: device config offset 0 (8 bytes).
    pub fn read_blk_capacity(&self) -> u64 {
        match self.transport_type {
            TransportType::Mmio => {
                // MMIO device config at offset 0x100
                unsafe {
                    let config_base = self.base + 0x100;
                    core::ptr::read_volatile(config_base as *const u64)
                }
            },
            TransportType::PciModern => {
                if self.pci_modern.device_cfg != 0 {
                    unsafe { core::ptr::read_volatile(self.pci_modern.device_cfg as *const u64) }
                } else {
                    0
                }
            },
            TransportType::PciLegacy => 0,
        }
    }

    /// virtio-blk sector size: device config offset 20 (4 bytes). Valid only
    /// if VIRTIO_BLK_F_BLK_SIZE was negotiated.
    pub fn read_blk_size(&self) -> u32 {
        match self.transport_type {
            TransportType::Mmio => unsafe {
                let config_base = self.base + 0x100 + 20;
                core::ptr::read_volatile(config_base as *const u32)
            },
            TransportType::PciModern => {
                if self.pci_modern.device_cfg != 0 {
                    unsafe {
                        core::ptr::read_volatile((self.pci_modern.device_cfg + 20) as *const u32)
                    }
                } else {
                    512
                }
            },
            TransportType::PciLegacy => 512,
        }
    }
}

pub mod pci_modern {
    extern "win64" {
        #[link_name = "asm_virtio_pci_get_status"]
        pub fn get_status(common_cfg: u64) -> u32;

        #[link_name = "asm_virtio_pci_set_status"]
        pub fn set_status(common_cfg: u64, status: u8);

        #[link_name = "asm_virtio_pci_reset"]
        pub fn reset(common_cfg: u64, tsc_freq: u64) -> u32;

        #[link_name = "asm_virtio_pci_read_features"]
        pub fn read_features(common_cfg: u64) -> u64;

        #[link_name = "asm_virtio_pci_write_features"]
        pub fn write_features(common_cfg: u64, features: u64);

        #[link_name = "asm_virtio_pci_get_num_queues"]
        pub fn get_num_queues(common_cfg: u64) -> u32;

        #[link_name = "asm_virtio_pci_select_queue"]
        pub fn select_queue(common_cfg: u64, queue_idx: u16);

        #[link_name = "asm_virtio_pci_get_queue_size"]
        pub fn get_queue_size(common_cfg: u64) -> u32;

        #[link_name = "asm_virtio_pci_set_queue_size"]
        pub fn set_queue_size(common_cfg: u64, size: u16);

        #[link_name = "asm_virtio_pci_enable_queue"]
        pub fn enable_queue(common_cfg: u64, enable: u16);

        #[link_name = "asm_virtio_pci_set_queue_desc"]
        pub fn set_queue_desc(common_cfg: u64, addr: u64);

        #[link_name = "asm_virtio_pci_set_queue_avail"]
        pub fn set_queue_avail(common_cfg: u64, addr: u64);

        #[link_name = "asm_virtio_pci_set_queue_used"]
        pub fn set_queue_used(common_cfg: u64, addr: u64);

        #[link_name = "asm_virtio_pci_get_queue_notify_off"]
        pub fn get_queue_notify_off(common_cfg: u64) -> u32;

        #[link_name = "asm_virtio_pci_notify_queue"]
        pub fn notify_queue(notify_addr: u64, queue_idx: u16);

        #[link_name = "asm_virtio_pci_read_mac"]
        pub fn read_mac(device_cfg: u64, mac_out: *mut u8);
    }
}
