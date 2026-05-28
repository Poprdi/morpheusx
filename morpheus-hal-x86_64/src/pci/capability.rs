//! PCI capability chain walking and VirtIO modern capability parsing.

use super::config::{pci_cfg_read16, pci_cfg_read8, PciAddr};

#[cfg(target_arch = "x86_64")]
extern "win64" {
    fn asm_pci_has_capabilities(bus: u8, device: u8, function: u8) -> u32;
    fn asm_pci_get_cap_ptr(bus: u8, device: u8, function: u8) -> u32;
    fn asm_pci_find_cap(bus: u8, device: u8, function: u8, cap_id: u8) -> u32;
    fn asm_pci_find_virtio_cap(bus: u8, device: u8, function: u8, cfg_type: u8) -> u32;

    fn asm_virtio_pci_parse_cap(
        bus: u8,
        device: u8,
        function: u8,
        cap_offset: u8,
        out: *mut VirtioCapInfo,
    ) -> u32;

    /// Returns RAX = address, RDX = 1 if memory / 0 if IO.
    fn asm_virtio_pci_read_bar(bus: u8, device: u8, function: u8, bar_idx: u8) -> u64;

    fn asm_virtio_pci_probe_caps(bus: u8, device: u8, function: u8, out: *mut VirtioCapInfo)
        -> u32;
}

/// Vendor-specific cap ID, used by VirtIO.
pub const PCI_CAP_ID_VNDR: u8 = 0x09;

pub const VIRTIO_PCI_CAP_COMMON: u8 = 1;
pub const VIRTIO_PCI_CAP_NOTIFY: u8 = 2;
pub const VIRTIO_PCI_CAP_ISR: u8 = 3;
pub const VIRTIO_PCI_CAP_DEVICE: u8 = 4;
pub const VIRTIO_PCI_CAP_PCI_CFG: u8 = 5;

/// Layout must match ASM expectations (24 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VirtioCapInfo {
    /// 1=common, 2=notify, 3=isr, 4=device, 5=pci_cfg.
    pub cfg_type: u8,
    pub bar: u8,
    pub _pad: [u8; 2],
    pub offset: u32,
    pub length: u32,
    /// Valid only for notify cap.
    pub notify_off_multiplier: u32,
    pub cap_offset: u8,
    pub _pad2: [u8; 7],
}

const _: () = assert!(core::mem::size_of::<VirtioCapInfo>() == 24);

#[derive(Debug, Clone, Copy, Default)]
pub struct VirtioPciCaps {
    pub common: Option<VirtioCapInfo>,
    pub notify: Option<VirtioCapInfo>,
    pub isr: Option<VirtioCapInfo>,
    pub device: Option<VirtioCapInfo>,
    pub pci_cfg: Option<VirtioCapInfo>,
    pub bar_addrs: [u64; 6],
    pub found_mask: u8,
}

impl VirtioPciCaps {
    /// Common + notify are the minimum for basic operation.
    pub fn has_required(&self) -> bool {
        self.common.is_some() && self.notify.is_some()
    }

    pub fn common_cfg_addr(&self) -> Option<u64> {
        self.common
            .map(|c| self.bar_addrs[c.bar as usize] + c.offset as u64)
    }

    pub fn notify_addr(&self) -> Option<u64> {
        self.notify
            .map(|n| self.bar_addrs[n.bar as usize] + n.offset as u64)
    }

    pub fn notify_multiplier(&self) -> u32 {
        self.notify.map(|n| n.notify_off_multiplier).unwrap_or(0)
    }

    pub fn device_cfg_addr(&self) -> Option<u64> {
        self.device
            .map(|d| self.bar_addrs[d.bar as usize] + d.offset as u64)
    }

    pub fn isr_addr(&self) -> Option<u64> {
        self.isr
            .map(|i| self.bar_addrs[i.bar as usize] + i.offset as u64)
    }
}

pub fn has_capabilities(addr: PciAddr) -> bool {
    unsafe { asm_pci_has_capabilities(addr.bus, addr.device, addr.function) != 0 }
}

pub fn get_cap_ptr(addr: PciAddr) -> Option<u8> {
    let ptr = unsafe { asm_pci_get_cap_ptr(addr.bus, addr.device, addr.function) };
    if ptr != 0 && ptr < 256 {
        Some(ptr as u8)
    } else {
        None
    }
}

pub fn find_cap(addr: PciAddr, cap_id: u8) -> Option<u8> {
    let offset = unsafe { asm_pci_find_cap(addr.bus, addr.device, addr.function, cap_id) };
    if offset != 0 && offset < 256 {
        Some(offset as u8)
    } else {
        None
    }
}

pub fn find_virtio_cap(addr: PciAddr, cfg_type: u8) -> Option<u8> {
    let offset = unsafe { asm_pci_find_virtio_cap(addr.bus, addr.device, addr.function, cfg_type) };
    if offset != 0 && offset < 256 {
        Some(offset as u8)
    } else {
        None
    }
}

pub fn parse_virtio_cap(addr: PciAddr, cap_offset: u8) -> Option<VirtioCapInfo> {
    let mut info = VirtioCapInfo::default();
    let result = unsafe {
        asm_virtio_pci_parse_cap(addr.bus, addr.device, addr.function, cap_offset, &mut info)
    };
    if result != 0 {
        Some(info)
    } else {
        None
    }
}

pub fn read_bar(addr: PciAddr, bar_idx: u8) -> u64 {
    unsafe { asm_virtio_pci_read_bar(addr.bus, addr.device, addr.function, bar_idx) }
}

/// Entry point for VirtIO PCI device discovery.
pub fn probe_virtio_caps(addr: PciAddr) -> VirtioPciCaps {
    let mut caps = VirtioPciCaps::default();
    let mut cap_array = [VirtioCapInfo::default(); 5];

    let found = unsafe {
        asm_virtio_pci_probe_caps(addr.bus, addr.device, addr.function, cap_array.as_mut_ptr())
    };

    caps.found_mask = found as u8;

    if found & (1 << 0) != 0 {
        caps.common = Some(cap_array[0]);
    }
    if found & (1 << 1) != 0 {
        caps.notify = Some(cap_array[1]);
    }
    if found & (1 << 2) != 0 {
        caps.isr = Some(cap_array[2]);
    }
    if found & (1 << 3) != 0 {
        caps.device = Some(cap_array[3]);
    }
    if found & (1 << 4) != 0 {
        caps.pci_cfg = Some(cap_array[4]);
    }

    for i in 0..6 {
        caps.bar_addrs[i] = read_bar(addr, i as u8);
    }

    caps
}

/// Pure-Rust capability walk; fallback when ASM bindings are unavailable.
pub fn walk_capabilities_rust(addr: PciAddr) -> impl Iterator<Item = (u8, u8)> {
    WalkCaps::new(addr)
}

struct WalkCaps {
    addr: PciAddr,
    current: u8,
    count: u8,
}

impl WalkCaps {
    fn new(addr: PciAddr) -> Self {
        let status = pci_cfg_read16(addr, super::config::offset::STATUS);
        let has_caps = (status & super::config::status::CAP_LIST) != 0;

        let start = if has_caps {
            pci_cfg_read8(addr, super::config::offset::CAP_PTR) & 0xFC
        } else {
            0
        };

        Self {
            addr,
            current: start,
            count: 0,
        }
    }
}

impl Iterator for WalkCaps {
    type Item = (u8, u8); // (offset, cap_id)

    fn next(&mut self) -> Option<Self::Item> {
        if self.current == 0 || self.count > 48 {
            return None;
        }

        self.count += 1;

        let offset = self.current;
        let header = pci_cfg_read16(self.addr, offset);
        let cap_id = (header & 0xFF) as u8;
        let next = ((header >> 8) & 0xFC) as u8;

        self.current = next;

        Some((offset, cap_id))
    }
}
