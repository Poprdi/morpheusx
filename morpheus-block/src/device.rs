//! `UnifiedBlockDevice` — dispatcher across VirtIO-blk, AHCI, SDHCI, USB-MSD.

use crate::ahci::{AhciDriver, AhciInitError};
use crate::block_traits::{BlockCompletion, BlockDeviceInfo, BlockDriver, BlockError};
use crate::sdhci::{SdhciDriver, SdhciInitError};
use crate::usb_msd::{UsbMsdDriver, UsbMsdInitError};
use crate::virtio_blk::{VirtioBlkDriver, VirtioBlkInitError};

pub enum UnifiedBlockDevice {
    VirtIO(VirtioBlkDriver),
    Ahci(AhciDriver),
    Sdhci(SdhciDriver),
    UsbMsd(UsbMsdDriver),
}

#[derive(Debug)]
pub enum UnifiedBlockError {
    NoDevice,
    VirtioError(VirtioBlkInitError),
    AhciError(AhciInitError),
    SdhciError(SdhciInitError),
    UsbMsdError(UsbMsdInitError),
}

crate::impl_from!(VirtioBlkInitError => UnifiedBlockError : VirtioError);
crate::impl_from!(AhciInitError => UnifiedBlockError : AhciError);
crate::impl_from!(SdhciInitError => UnifiedBlockError : SdhciError);
crate::impl_from!(UsbMsdInitError => UnifiedBlockError : UsbMsdError);

impl UnifiedBlockDevice {
    pub fn driver_type(&self) -> &'static str {
        match self {
            UnifiedBlockDevice::VirtIO(_) => "VirtIO-blk",
            UnifiedBlockDevice::Ahci(_) => "AHCI SATA",
            UnifiedBlockDevice::Sdhci(_) => "SDHCI",
            UnifiedBlockDevice::UsbMsd(_) => "USB-MSD",
        }
    }

    pub fn is_ready(&self) -> bool {
        match self {
            UnifiedBlockDevice::VirtIO(_) => true,
            UnifiedBlockDevice::Ahci(d) => d.link_up(),
            UnifiedBlockDevice::Sdhci(_) => true,
            UnifiedBlockDevice::UsbMsd(_) => true,
        }
    }
}

impl BlockDriver for UnifiedBlockDevice {
    fn info(&self) -> BlockDeviceInfo {
        match self {
            UnifiedBlockDevice::VirtIO(d) => d.info(),
            UnifiedBlockDevice::Ahci(d) => d.info(),
            UnifiedBlockDevice::Sdhci(d) => d.info(),
            UnifiedBlockDevice::UsbMsd(d) => d.info(),
        }
    }

    fn can_submit(&self) -> bool {
        match self {
            UnifiedBlockDevice::VirtIO(d) => d.can_submit(),
            UnifiedBlockDevice::Ahci(d) => d.can_submit(),
            UnifiedBlockDevice::Sdhci(d) => d.can_submit(),
            UnifiedBlockDevice::UsbMsd(d) => d.can_submit(),
        }
    }

    fn submit_read(
        &mut self,
        sector: u64,
        buffer_phys: u64,
        num_sectors: u32,
        request_id: u32,
    ) -> core::result::Result<(), BlockError> {
        match self {
            UnifiedBlockDevice::VirtIO(d) => {
                d.submit_read(sector, buffer_phys, num_sectors, request_id)
            },
            UnifiedBlockDevice::Ahci(d) => {
                d.submit_read(sector, buffer_phys, num_sectors, request_id)
            },
            UnifiedBlockDevice::Sdhci(d) => {
                d.submit_read(sector, buffer_phys, num_sectors, request_id)
            },
            UnifiedBlockDevice::UsbMsd(d) => {
                d.submit_read(sector, buffer_phys, num_sectors, request_id)
            },
        }
    }

    fn submit_write(
        &mut self,
        sector: u64,
        buffer_phys: u64,
        num_sectors: u32,
        request_id: u32,
    ) -> core::result::Result<(), BlockError> {
        match self {
            UnifiedBlockDevice::VirtIO(d) => {
                d.submit_write(sector, buffer_phys, num_sectors, request_id)
            },
            UnifiedBlockDevice::Ahci(d) => {
                d.submit_write(sector, buffer_phys, num_sectors, request_id)
            },
            UnifiedBlockDevice::Sdhci(d) => {
                d.submit_write(sector, buffer_phys, num_sectors, request_id)
            },
            UnifiedBlockDevice::UsbMsd(d) => {
                d.submit_write(sector, buffer_phys, num_sectors, request_id)
            },
        }
    }

    fn poll_completion(&mut self) -> Option<BlockCompletion> {
        match self {
            UnifiedBlockDevice::VirtIO(d) => d.poll_completion(),
            UnifiedBlockDevice::Ahci(d) => d.poll_completion(),
            UnifiedBlockDevice::Sdhci(d) => d.poll_completion(),
            UnifiedBlockDevice::UsbMsd(d) => d.poll_completion(),
        }
    }

    fn notify(&mut self) {
        match self {
            UnifiedBlockDevice::VirtIO(d) => d.notify(),
            UnifiedBlockDevice::Ahci(d) => d.notify(),
            UnifiedBlockDevice::Sdhci(d) => d.notify(),
            UnifiedBlockDevice::UsbMsd(d) => d.notify(),
        }
    }

    fn flush(&mut self) -> core::result::Result<(), BlockError> {
        match self {
            UnifiedBlockDevice::VirtIO(d) => d.flush(),
            UnifiedBlockDevice::Ahci(d) => d.flush(),
            UnifiedBlockDevice::Sdhci(d) => d.flush(),
            UnifiedBlockDevice::UsbMsd(d) => d.flush(),
        }
    }
}
