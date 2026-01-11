//! VirtIO Transport Abstraction
//!
//! Provides a unified interface for different VirtIO transports:
//! - MMIO (used by ARM, some embedded systems)
//! - PCI Modern (VirtIO 1.0+, uses capabilities)
//! - PCI Legacy (older QEMU, uses BAR0 I/O ports)
//!
//! The transport is probed at runtime based on device discovery.

use crate::asm::drivers::virtio::device as mmio_device;

/// VirtIO transport type, determined at probe time
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TransportType {
    /// VirtIO MMIO transport (direct register access)
    Mmio = 0,
    /// VirtIO PCI Modern transport (capability-based)
    PciModern = 1,
    /// VirtIO PCI Legacy transport (BAR0 I/O ports) - not fully supported
    PciLegacy = 2,
}

/// Configuration for VirtIO PCI Modern transport
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct PciModernConfig {
    /// Base address of common_cfg (BAR + cap_offset)
    pub common_cfg: u64,
    /// Base address of notify_cfg (BAR + cap_offset)
    pub notify_cfg: u64,
    /// Notify offset multiplier (from capability)
    pub notify_off_multiplier: u32,
    /// Base address of isr_cfg (BAR + cap_offset)
    pub isr_cfg: u64,
    /// Base address of device_cfg (BAR + cap_offset)
    pub device_cfg: u64,
    /// Base address of pci_cfg (optional, for config space access)
    pub pci_cfg: u64,
}

impl Default for PciModernConfig {
    fn default() -> Self {
        Self {
            common_cfg: 0,
            notify_cfg: 0,
            notify_off_multiplier: 0,
            isr_cfg: 0,
            device_cfg: 0,
            pci_cfg: 0,
        }
    }
}

/// Unified VirtIO transport handle
#[derive(Debug, Clone, Copy)]
pub struct VirtioTransport {
    /// Transport type
    pub transport_type: TransportType,
    /// For MMIO: the MMIO base address
    /// For PCI Modern: common_cfg base address
    pub base: u64,
    /// PCI Modern specific config (only valid if transport_type == PciModern)
    pub pci_modern: PciModernConfig,
}

impl VirtioTransport {
    /// Create MMIO transport
    pub fn mmio(mmio_base: u64) -> Self {
        Self {
            transport_type: TransportType::Mmio,
            base: mmio_base,
            pci_modern: PciModernConfig::default(),
        }
    }

    /// Create PCI Modern transport
    pub fn pci_modern(config: PciModernConfig) -> Self {
        Self {
            transport_type: TransportType::PciModern,
            base: config.common_cfg,
            pci_modern: config,
        }
    }

    /// Get device status
    pub fn get_status(&self) -> u8 {
        match self.transport_type {
            TransportType::Mmio => mmio_device::get_status(self.base),
            TransportType::PciModern => unsafe { pci_modern::get_status(self.base) as u8 },
            TransportType::PciLegacy => 0, // Not supported
        }
    }

    /// Set device status
    pub fn set_status(&self, status: u8) {
        match self.transport_type {
            TransportType::Mmio => mmio_device::set_status(self.base, status),
            TransportType::PciModern => unsafe { pci_modern::set_status(self.base, status) },
            TransportType::PciLegacy => {} // Not supported
        }
    }

    /// Reset device
    pub fn reset(&self, tsc_freq: u64) -> bool {
        // Note: MMIO reset doesn't use tsc_freq, PCI Modern does for timeout
        match self.transport_type {
            TransportType::Mmio => mmio_device::reset(self.base),
            TransportType::PciModern => unsafe { pci_modern::reset(self.base, tsc_freq) == 0 },
            TransportType::PciLegacy => false,
        }
    }

    /// Read device feature bits (64-bit)
    pub fn read_features(&self) -> u64 {
        match self.transport_type {
            TransportType::Mmio => mmio_device::read_features(self.base),
            TransportType::PciModern => unsafe { pci_modern::read_features(self.base) },
            TransportType::PciLegacy => 0,
        }
    }

    /// Write driver-accepted features
    pub fn write_features(&self, features: u64) {
        match self.transport_type {
            TransportType::Mmio => mmio_device::write_features(self.base, features),
            TransportType::PciModern => unsafe { pci_modern::write_features(self.base, features) },
            TransportType::PciLegacy => {}
        }
    }

    /// Get number of queues (PCI Modern only, MMIO needs different approach)
    pub fn get_num_queues(&self) -> u16 {
        match self.transport_type {
            TransportType::PciModern => unsafe { pci_modern::get_num_queues(self.base) as u16 },
            _ => 2, // Default for MMIO net devices: RX + TX
        }
    }

    /// Select a queue for configuration
    pub fn select_queue(&self, queue_idx: u16) {
        unsafe {
            match self.transport_type {
                TransportType::Mmio => {
                    // MMIO: write to QueueSel register
                    let queue_sel_addr = self.base + 0x030;
                    core::ptr::write_volatile(queue_sel_addr as *mut u32, queue_idx as u32);
                    core::arch::asm!("mfence", options(nostack, preserves_flags));
                }
                TransportType::PciModern => pci_modern::select_queue(self.base, queue_idx),
                TransportType::PciLegacy => {}
            }
        }
    }

    /// Get max queue size for selected queue
    pub fn get_queue_size(&self) -> u16 {
        unsafe {
            match self.transport_type {
                TransportType::Mmio => {
                    let queue_num_max_addr = self.base + 0x034;
                    core::ptr::read_volatile(queue_num_max_addr as *const u32) as u16
                }
                TransportType::PciModern => pci_modern::get_queue_size(self.base) as u16,
                TransportType::PciLegacy => 0,
            }
        }
    }

    /// Set queue size
    pub fn set_queue_size(&self, size: u16) {
        unsafe {
            match self.transport_type {
                TransportType::Mmio => {
                    let queue_num_addr = self.base + 0x038;
                    core::ptr::write_volatile(queue_num_addr as *mut u32, size as u32);
                    core::arch::asm!("mfence", options(nostack, preserves_flags));
                }
                TransportType::PciModern => pci_modern::set_queue_size(self.base, size),
                TransportType::PciLegacy => {}
            }
        }
    }

    /// Set queue descriptor table address
    pub fn set_queue_desc(&self, addr: u64) {
        unsafe {
            match self.transport_type {
                TransportType::Mmio => {
                    let lo_addr = self.base + 0x080;
                    let hi_addr = self.base + 0x084;
                    core::ptr::write_volatile(lo_addr as *mut u32, addr as u32);
                    core::ptr::write_volatile(hi_addr as *mut u32, (addr >> 32) as u32);
                    core::arch::asm!("mfence", options(nostack, preserves_flags));
                }
                TransportType::PciModern => pci_modern::set_queue_desc(self.base, addr),
                TransportType::PciLegacy => {}
            }
        }
    }

    /// Set queue available ring address
    pub fn set_queue_avail(&self, addr: u64) {
        unsafe {
            match self.transport_type {
                TransportType::Mmio => {
                    let lo_addr = self.base + 0x090;
                    let hi_addr = self.base + 0x094;
                    core::ptr::write_volatile(lo_addr as *mut u32, addr as u32);
                    core::ptr::write_volatile(hi_addr as *mut u32, (addr >> 32) as u32);
                    core::arch::asm!("mfence", options(nostack, preserves_flags));
                }
                TransportType::PciModern => pci_modern::set_queue_avail(self.base, addr),
                TransportType::PciLegacy => {}
            }
        }
    }

    /// Set queue used ring address
    pub fn set_queue_used(&self, addr: u64) {
        unsafe {
            match self.transport_type {
                TransportType::Mmio => {
                    let lo_addr = self.base + 0x0A0;
                    let hi_addr = self.base + 0x0A4;
                    core::ptr::write_volatile(lo_addr as *mut u32, addr as u32);
                    core::ptr::write_volatile(hi_addr as *mut u32, (addr >> 32) as u32);
                    core::arch::asm!("mfence", options(nostack, preserves_flags));
                }
                TransportType::PciModern => pci_modern::set_queue_used(self.base, addr),
                TransportType::PciLegacy => {}
            }
        }
    }

    /// Enable the selected queue
    pub fn enable_queue(&self) {
        unsafe {
            match self.transport_type {
                TransportType::Mmio => {
                    let queue_ready_addr = self.base + 0x044;
                    core::ptr::write_volatile(queue_ready_addr as *mut u32, 1);
                    core::arch::asm!("mfence", options(nostack, preserves_flags));
                }
                TransportType::PciModern => pci_modern::enable_queue(self.base, 1),
                TransportType::PciLegacy => {}
            }
        }
    }

    /// Get notify address for a queue
    /// Returns (notify_addr, queue_notify_offset)
    pub fn get_notify_addr(&self, queue_idx: u16) -> u64 {
        match self.transport_type {
            TransportType::Mmio => {
                // MMIO: fixed notify register
                self.base + 0x050
            }
            TransportType::PciModern => {
                // PCI Modern: need to select queue and read notify_off
                unsafe {
                    pci_modern::select_queue(self.base, queue_idx);
                    let queue_notify_off = pci_modern::get_queue_notify_off(self.base);
                    let notify_addr = self.pci_modern.notify_cfg
                        + (queue_notify_off as u64 * self.pci_modern.notify_off_multiplier as u64);
                    notify_addr
                }
            }
            TransportType::PciLegacy => 0,
        }
    }

    /// Notify device about queue
    pub fn notify_queue(&self, queue_idx: u16) {
        unsafe {
            match self.transport_type {
                TransportType::Mmio => {
                    let notify_addr = self.base + 0x050;
                    core::arch::asm!("mfence", options(nostack, preserves_flags));
                    core::ptr::write_volatile(notify_addr as *mut u32, queue_idx as u32);
                }
                TransportType::PciModern => {
                    let notify_addr = self.get_notify_addr(queue_idx);
                    pci_modern::notify_queue(notify_addr, queue_idx);
                }
                TransportType::PciLegacy => {}
            }
        }
    }

    /// Read MAC address (net device specific)
    pub fn read_mac(&self, mac_out: &mut [u8; 6]) -> bool {
        match self.transport_type {
            TransportType::Mmio => {
                if let Some(mac) = mmio_device::read_mac(self.base) {
                    *mac_out = mac;
                    true
                } else {
                    false
                }
            }
            TransportType::PciModern => {
                if self.pci_modern.device_cfg != 0 {
                    unsafe {
                        pci_modern::read_mac(self.pci_modern.device_cfg, mac_out.as_mut_ptr());
                    }
                    true
                } else {
                    false
                }
            }
            TransportType::PciLegacy => false,
        }
    }

    /// Setup a virtqueue. Returns the notify address on success.
    ///
    /// # Arguments
    /// - `queue_idx`: Queue index (0 for blk, 0/1 for net RX/TX)
    /// - `desc_addr`: Physical address of descriptor table
    /// - `avail_addr`: Physical address of available ring
    /// - `used_addr`: Physical address of used ring
    /// - `queue_size`: Number of descriptors
    pub fn setup_queue(
        &self,
        queue_idx: u16,
        desc_addr: u64,
        avail_addr: u64,
        used_addr: u64,
        queue_size: u16,
    ) -> Result<u64, crate::driver::virtio::init::VirtioInitError> {
        use crate::driver::virtio::init::VirtioInitError;

        // Select queue
        self.select_queue(queue_idx);

        // Check max size
        let max_size = self.get_queue_size();
        if max_size == 0 {
            return Err(VirtioInitError::QueueSetupFailed);
        }

        // Use min of requested and max
        let actual_size = queue_size.min(max_size);
        self.set_queue_size(actual_size);

        // Set addresses
        self.set_queue_desc(desc_addr);
        self.set_queue_avail(avail_addr);
        self.set_queue_used(used_addr);

        // Enable queue
        self.enable_queue();

        // Get notify address
        let notify_addr = self.get_notify_addr(queue_idx);

        Ok(notify_addr)
    }

    /// Read block device capacity (blk device specific)
    pub fn read_blk_capacity(&self) -> u64 {
        // VirtIO-blk device config: capacity is at offset 0 (8 bytes)
        match self.transport_type {
            TransportType::Mmio => {
                // MMIO: device config at offset 0x100
                unsafe {
                    let config_base = self.base + 0x100;
                    core::ptr::read_volatile(config_base as *const u64)
                }
            }
            TransportType::PciModern => {
                // PCI Modern: device_cfg points to device-specific config
                if self.pci_modern.device_cfg != 0 {
                    unsafe { core::ptr::read_volatile(self.pci_modern.device_cfg as *const u64) }
                } else {
                    0
                }
            }
            TransportType::PciLegacy => 0,
        }
    }

    /// Read block device sector size (blk device specific)
    pub fn read_blk_size(&self) -> u32 {
        // VirtIO-blk device config: blk_size is at offset 20 (4 bytes)
        // Only valid if VIRTIO_BLK_F_BLK_SIZE feature negotiated
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
            }
            TransportType::PciLegacy => 512,
        }
    }
}

/// PCI Modern transport ASM bindings
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
