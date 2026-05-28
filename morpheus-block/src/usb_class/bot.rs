//! Bulk-Only Transport: CBW/CSW over bulk EPs + SCSI cmd wrapper. Legacy
//! Phase-9 diagnostic helpers; live USB-MSD path is in `usb_msd/`. Extension
//! trait so we can hang methods on `XhciController` across the crate split.

use morpheus_xhci::dma;
use morpheus_xhci::regs::*;
use morpheus_xhci::rings::vr32;
use morpheus_xhci::{XhciController, XhciError};

const CBW_SIG: u32 = 0x4342_5355;
const CSW_SIG: u32 = 0x5342_5355;

const SCSI_TEST_UNIT_READY: u8 = 0x00;
const SCSI_READ_CAPACITY_10: u8 = 0x25;
const SCSI_READ_10: u8 = 0x28;
#[allow(dead_code)]
const SCSI_REQUEST_SENSE: u8 = 0x03;

pub trait BotExt {
    /// Run a BOT command. Data lands at OFF_DATA. Returns bytes transferred.
    ///
    /// # Safety
    ///
    /// The controller must have an enumerated, configured mass-storage device
    /// on its active slot, and the BOT data buffer (OFF_DATA) must be valid for
    /// `data_len` bytes. Caller must hold exclusive access to the controller and
    /// its DMA regions for the duration of the transfer.
    unsafe fn bot_command(
        &mut self,
        scsi_cb: &[u8],
        data_len: u32,
        data_in: bool,
    ) -> Result<u32, XhciError>;

    /// Returns (last_lba, block_size).
    ///
    /// # Safety
    ///
    /// Same invariants as [`BotExt::bot_command`]: requires an enumerated
    /// mass-storage device and exclusive access to the controller's DMA regions.
    unsafe fn scsi_read_capacity(&mut self) -> Result<(u64, u32), XhciError>;
    /// # Safety
    ///
    /// Same invariants as [`BotExt::bot_command`]: requires an enumerated
    /// mass-storage device and exclusive access to the controller's DMA regions.
    /// `lba`/`count` must reference sectors within device capacity.
    unsafe fn scsi_read_sectors(&mut self, lba: u64, count: u32) -> Result<(), XhciError>;
    /// # Safety
    ///
    /// Same invariants as [`BotExt::bot_command`]: requires an enumerated
    /// mass-storage device and exclusive access to the controller's DMA regions.
    unsafe fn test_unit_ready(&mut self) -> Result<bool, XhciError>;
}

impl BotExt for XhciController {
    unsafe fn bot_command(
        &mut self,
        scsi_cb: &[u8],
        data_len: u32,
        data_in: bool,
    ) -> Result<u32, XhciError> {
        let tag = {
            static mut TAG: u32 = 1;
            let t = core::ptr::read_volatile(core::ptr::addr_of!(TAG));
            core::ptr::write_volatile(core::ptr::addr_of_mut!(TAG), t.wrapping_add(1));
            t
        };

        let cbw = self.dma_base + dma::OFF_CBW as u64;
        core::ptr::write_bytes(cbw as *mut u8, 0, 31);
        core::ptr::write_volatile((cbw) as *mut u32, CBW_SIG);
        core::ptr::write_volatile((cbw + 4) as *mut u32, tag);
        core::ptr::write_volatile((cbw + 8) as *mut u32, data_len);
        core::ptr::write_volatile(
            (cbw + 12) as *mut u8,
            if data_in && data_len > 0 { 0x80 } else { 0 },
        );
        core::ptr::write_volatile((cbw + 14) as *mut u8, scsi_cb.len().min(16) as u8);
        for (i, &b) in scsi_cb.iter().take(16).enumerate() {
            core::ptr::write_volatile((cbw + 15 + i as u64) as *mut u8, b);
        }

        // CBW on bulk-out.
        self.bout.enqueue(cbw, 31, TRB_NORMAL | TRB_IOC);
        self.ring_xfer_doorbell(self.dci_bulk_out as u32);
        self.wait_xfer(self.slot_id, self.dci_bulk_out as u32, 5000)?;

        let mut transferred = 0u32;
        if data_len > 0 {
            let buf = self.dma_base + dma::OFF_DATA as u64;
            if data_in {
                self.bin
                    .enqueue(buf, data_len, TRB_NORMAL | TRB_IOC | TRB_ISP);
                self.ring_xfer_doorbell(self.dci_bulk_in as u32);
            } else {
                self.bout.enqueue(buf, data_len, TRB_NORMAL | TRB_IOC);
                self.ring_xfer_doorbell(self.dci_bulk_out as u32);
            }
            let residue = self.wait_xfer(
                self.slot_id,
                if data_in {
                    self.dci_bulk_in as u32
                } else {
                    self.dci_bulk_out as u32
                },
                10000,
            )?;
            transferred = data_len.saturating_sub(residue);
        }

        // CSW on bulk-in.
        let csw = self.dma_base + dma::OFF_CSW as u64;
        core::ptr::write_bytes(csw as *mut u8, 0, 13);
        self.bin.enqueue(csw, 13, TRB_NORMAL | TRB_IOC);
        self.ring_xfer_doorbell(self.dci_bulk_in as u32);
        self.wait_xfer(self.slot_id, self.dci_bulk_in as u32, 5000)?;

        let sig = vr32(csw);
        let csw_tag = vr32(csw + 4);
        let status = core::ptr::read_volatile((csw + 12) as *const u8);
        if sig != CSW_SIG || csw_tag != tag || status != 0 {
            return Err(XhciError::IoError);
        }

        Ok(transferred)
    }

    unsafe fn scsi_read_capacity(&mut self) -> Result<(u64, u32), XhciError> {
        let cmd = [SCSI_READ_CAPACITY_10, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        self.bot_command(&cmd, 8, true)?;
        let data = self.dma_base + dma::OFF_DATA as u64;
        let last_lba = vr32(data) as u64;
        let blk_size = vr32(data + 4);
        Ok((last_lba, blk_size))
    }

    unsafe fn scsi_read_sectors(&mut self, lba: u64, count: u32) -> Result<(), XhciError> {
        let byte_count = count * 512;
        let mut cmd = [0u8; 10];
        cmd[0] = SCSI_READ_10;
        cmd[2] = (lba >> 24) as u8;
        cmd[3] = (lba >> 16) as u8;
        cmd[4] = (lba >> 8) as u8;
        cmd[5] = lba as u8;
        cmd[7] = (count >> 8) as u8;
        cmd[8] = count as u8;
        self.bot_command(&cmd, byte_count, true)?;
        Ok(())
    }

    unsafe fn test_unit_ready(&mut self) -> Result<bool, XhciError> {
        let cmd = [SCSI_TEST_UNIT_READY, 0, 0, 0, 0, 0];
        self.bot_command(&cmd, 0, false)?;
        Ok(true)
    }
}
