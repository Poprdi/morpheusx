//! Persistent storage probe + RawBlockDevice bridge to HelixFS.
//!
//! DMA layout within hwinit's 2 MB region:
//!   0x00000  VirtIO desc/avail/used/headers/status  (≤ 0x01400)
//!   0x02000  AHCI cmd_list/FIS/cmd_tables/IDENTIFY  (≤ 0x05000)
//!   0x10000  64 KB I/O buffer for UnifiedBlockIo

use morpheus_helix::device::{MemBlockDevice, RawBlockDevice};
/// In-RAM staged Helix partition (Some after successful RAM staging).
static mut RAM_HELIX_DEVICE: Option<MemBlockDevice> = None;
use morpheus_block::ahci::AhciInitError;
use morpheus_block::boot_probe::{
    create_unified_from_detected, scan_all_block_devices, BlockDmaConfig, DetectedBlockDevice,
};
use morpheus_block::device::{UnifiedBlockDevice, UnifiedBlockError};
use morpheus_block::sdhci::SdhciInitError;
use morpheus_block::unified_block_io::UnifiedBlockIo;
use morpheus_block::usb_msd::UsbMsdInitError;
use morpheus_block::virtio_blk::VirtioBlkInitError;
use morpheus_block::BlockDriver;
use morpheus_hal_x86_64::dma::DmaRegion;
use morpheus_hal_x86_64::memory::{global_registry_mut, AllocateType, MemoryType};
use morpheus_hal_x86_64::paging::is_paging_initialized;
use morpheus_hal_x86_64::paging::kmap_mmio;
use morpheus_hal_x86_64::pci::{pci_cfg_read16, pci_cfg_read32, PciAddr};
use morpheus_hal_x86_64::serial::{log_error, log_warn, puts};

const VIRTIO_QUEUE_SIZE: u16 = 32;

const OFF_VIRTIO_DESC: usize = 0x0_0000;
const OFF_VIRTIO_AVAIL: usize = 0x0_0200;
const OFF_VIRTIO_USED: usize = 0x0_0400;
const OFF_VIRTIO_HEADERS: usize = 0x0_1000; // page-aligned
const OFF_VIRTIO_STATUS: usize = 0x0_1200;

const OFF_AHCI_CMD_LIST: usize = 0x0_2000; // 1 KB align
const OFF_AHCI_FIS: usize = 0x0_2400; // 256 B align
const OFF_AHCI_CMD_TABLES: usize = 0x0_2800; // 128 B align
const OFF_AHCI_IDENTIFY: usize = 0x0_4800;

const OFF_IO_BUFFER: usize = 0x1_0000;
const IO_BUFFER_SIZE: usize = 64 * 1024; // == UnifiedBlockIo::MAX_TRANSFER_SIZE
const UNKNOWN_TOTAL_SECTORS: u64 = u32::MAX as u64;

unsafe fn dump_pci_devices() {
    puts("[PCI-DUMP] scanning bus 0...\n");
    for dev in 0..32u8 {
        for func in 0..8u8 {
            let addr = PciAddr::new(0, dev, func);
            let vendor_id = pci_cfg_read16(addr, 0x00);
            if vendor_id == 0xFFFF {
                if func == 0 {
                    break;
                }
                continue;
            }
            let device_id = pci_cfg_read16(addr, 0x02);
            let class_code = pci_cfg_read32(addr, 0x08);
            let cmd = pci_cfg_read16(addr, 0x04);

            puts("[PCI-DUMP]   00:");
            morpheus_hal_x86_64::serial::put_hex8(dev);
            puts(".");
            morpheus_hal_x86_64::serial::put_hex8(func);
            puts("  ven=");
            morpheus_hal_x86_64::serial::put_hex32(vendor_id as u32);
            puts(" dev=");
            morpheus_hal_x86_64::serial::put_hex32(device_id as u32);
            puts(" class=");
            morpheus_hal_x86_64::serial::put_hex32(class_code >> 8);
            puts(" cmd=");
            morpheus_hal_x86_64::serial::put_hex32(cmd as u32);
            puts("\n");

            if vendor_id == 0x1AF4 {
                let mut bar_i = 0u8;
                while bar_i < 6 {
                    let raw = pci_cfg_read32(addr, 0x10 + bar_i * 4);
                    if raw != 0 {
                        let is_io = raw & 0x01 != 0;
                        let is_64 = !is_io && (raw >> 1) & 0x03 == 0x02;
                        puts("[PCI-DUMP]     BAR");
                        morpheus_hal_x86_64::serial::put_hex8(bar_i);
                        puts(" raw=");
                        morpheus_hal_x86_64::serial::put_hex32(raw);
                        if is_64 && bar_i < 5 {
                            let high = pci_cfg_read32(addr, 0x10 + (bar_i + 1) * 4);
                            puts(" BAR");
                            morpheus_hal_x86_64::serial::put_hex8(bar_i + 1);
                            puts("=");
                            morpheus_hal_x86_64::serial::put_hex32(high);
                            let full = ((high as u64) << 32) | ((raw & 0xFFFFFFF0) as u64);
                            puts(" -> ");
                            morpheus_hal_x86_64::serial::put_hex64(full);
                        }
                        if is_io {
                            puts(" (IO)");
                        }
                        puts("\n");
                        bar_i += if is_64 { 2 } else { 1 };
                    } else {
                        bar_i += 1;
                    }
                }
            }

            if func == 0 {
                let header = pci_cfg_read16(addr, 0x0E) & 0x80;
                if header == 0 {
                    break;
                }
            }
        }
    }
    puts("[PCI-DUMP] done\n");
}

/// Identity-map VirtIO BAR MMIO as UC. 16 KiB covers all VirtIO cap regions.
///
/// # Safety
/// Paging and MemoryRegistry must be initialized.
unsafe fn map_virtio_bars(bus: u8, dev: u8, func: u8) {
    if !is_paging_initialized() {
        log_warn(
            "STORAGE",
            820,
            "paging not initialized; skipping BAR UC mapping",
        );
        return;
    }

    let addr = PciAddr::new(bus, dev, func);
    const MAP_SIZE: u64 = 16 * 1024;

    let mut bar_idx = 0u8;
    while bar_idx < 6 {
        let bar_offset = 0x10u8 + bar_idx * 4;
        let bar_low = pci_cfg_read32(addr, bar_offset);

        if bar_low == 0 || bar_low & 0x01 != 0 {
            bar_idx += 1;
            continue;
        }

        let bar_type = (bar_low >> 1) & 0x03;
        let base_low = (bar_low & 0xFFFF_FFF0) as u64;

        let (base_addr, is_64bit) = if bar_type == 0x02 && bar_idx < 5 {
            let bar_high = pci_cfg_read32(addr, bar_offset + 4);
            (((bar_high as u64) << 32) | base_low, true)
        } else {
            (base_low, false)
        };

        if base_addr != 0 {
            match kmap_mmio(base_addr, MAP_SIZE) {
                Ok(()) => {},
                Err(e) => {
                    let _ = (bar_idx, e);
                    log_warn("STORAGE", 821, "map_mmio for VirtIO BAR failed");
                },
            }
        }

        bar_idx += if is_64bit { 2 } else { 1 };
    }
}

static mut BLOCK_DEVICE: Option<UnifiedBlockDevice> = None;
static mut STORAGE_DMA: Option<DmaRegion> = None;
static mut STORAGE_TSC_FREQ: u64 = 0;
/// Start LBA of selected region (0 = whole disk).
static mut STORAGE_LBA_BASE: u64 = 0;
static mut STORAGE_REGION_SECTORS: u64 = 0;
static mut PERSISTENT_READY: bool = false;

const RAM_STAGE_MAX_BYTES: u64 = 512 * 1024 * 1024;
static mut RAM_STAGE_LAST_REASON: &str = "none";

const GPT_SIG: &[u8; 8] = b"EFI PART";
/// ESP type GUID, little-endian on-disk.
const GPT_TYPE_ESP: [u8; 16] = [
    0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11, 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B,
];

#[derive(Clone, Copy)]
struct DataRegion {
    lba_start: u64,
    sectors: u64,
}

#[derive(Clone, Copy)]
struct GptPartition {
    type_guid: [u8; 16],
    first_lba: u64,
    last_lba: u64,
}

#[inline(always)]
fn le_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

#[inline(always)]
fn le_u64(buf: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        buf[off],
        buf[off + 1],
        buf[off + 2],
        buf[off + 3],
        buf[off + 4],
        buf[off + 5],
        buf[off + 6],
        buf[off + 7],
    ])
}

#[inline(always)]
fn select_largest_gpt_free_region(
    first_usable: u64,
    last_usable: u64,
    used_ranges: &mut [(u64, u64)],
    sector_size: u32,
) -> Option<DataRegion> {
    if first_usable == 0 || last_usable < first_usable {
        return None;
    }

    used_ranges.sort_unstable_by_key(|(start, _)| *start);

    let mut cursor = first_usable;
    let mut best_start = 0u64;
    let mut best_sectors = 0u64;

    for (raw_start, raw_end) in used_ranges.iter().copied() {
        if raw_end < first_usable || raw_start > last_usable {
            continue;
        }

        let start = raw_start.max(first_usable);
        let end = raw_end.min(last_usable);

        if start > cursor {
            let gap = start - cursor;
            if gap > best_sectors {
                best_start = cursor;
                best_sectors = gap;
            }
        }

        let next = end.saturating_add(1);
        if next > cursor {
            cursor = next;
        }

        if cursor > last_usable {
            break;
        }
    }

    if cursor <= last_usable {
        let tail = (last_usable - cursor) + 1;
        if tail > best_sectors {
            best_start = cursor;
            best_sectors = tail;
        }
    }

    let min_free_sectors = (64u64 * 1024 * 1024).div_ceil(sector_size as u64);
    if best_sectors >= min_free_sectors {
        Some(DataRegion {
            lba_start: best_start,
            sectors: best_sectors,
        })
    } else {
        None
    }
}

/// Policy: GPT → partition immediately after ESP, else largest free region,
/// else first non-ESP; MBR → partition after 0xEF, else largest primary;
/// no partition table → whole disk.
unsafe fn select_data_region(sector_size: u32, total_sectors: u64) -> Option<DataRegion> {
    let dev = BLOCK_DEVICE.as_mut()?;
    let dma = STORAGE_DMA.as_ref()?;

    let io_cpu = dma.cpu_base().add(OFF_IO_BUFFER);
    let io_phys = dma.bus_at(OFF_IO_BUFFER);
    let io_buf = core::slice::from_raw_parts_mut(io_cpu, IO_BUFFER_SIZE);
    let timeout = STORAGE_TSC_FREQ * 5;

    let mut bio = UnifiedBlockIo::new(dev, io_buf, io_phys, timeout).ok()?;

    use gpt_disk_io::BlockIo as GptBlockIo;
    use gpt_disk_types::Lba as GptLba;

    let mut first_two = alloc::vec![0u8; (sector_size as usize) * 2];
    if bio.read_blocks(GptLba(0), &mut first_two).is_err() {
        return None;
    }

    let has_mbr = first_two.len() >= 512 && first_two[510] == 0x55 && first_two[511] == 0xAA;
    let gpt_off = sector_size as usize;
    let has_gpt = first_two.len() >= gpt_off + 8 && &first_two[gpt_off..gpt_off + 8] == GPT_SIG;

    if has_gpt {
        let hdr = &first_two[gpt_off..gpt_off + sector_size as usize];
        let first_usable = le_u64(hdr, 40);
        let last_usable = le_u64(hdr, 48);
        let entries_lba = le_u64(hdr, 72);
        let num_entries = le_u32(hdr, 80) as usize;
        let entry_size = le_u32(hdr, 84) as usize;

        if entry_size < 56 || num_entries == 0 {
            return None;
        }

        let entries_per_sector = (sector_size as usize) / entry_size;
        if entries_per_sector == 0 {
            return None;
        }

        let mut sec = alloc::vec![0u8; sector_size as usize];
        let mut used_ranges = alloc::vec::Vec::<(u64, u64)>::new();
        let mut partitions = alloc::vec::Vec::<GptPartition>::new();
        let mut first_non_esp: Option<DataRegion> = None;

        for idx in 0..num_entries {
            let sector_delta = idx / entries_per_sector;
            let idx_in_sector = idx % entries_per_sector;
            let lba = entries_lba + sector_delta as u64;

            if bio.read_blocks(GptLba(lba), &mut sec).is_err() {
                return None;
            }

            let off = idx_in_sector * entry_size;
            let ent = &sec[off..off + entry_size];

            if ent[..16].iter().all(|b| *b == 0) {
                continue;
            }

            let first_lba = le_u64(ent, 32);
            let last_lba = le_u64(ent, 40);
            if first_lba == 0 || last_lba < first_lba {
                continue;
            }

            used_ranges.push((first_lba, last_lba));

            partitions.push(GptPartition {
                type_guid: [
                    ent[0], ent[1], ent[2], ent[3], ent[4], ent[5], ent[6], ent[7], ent[8], ent[9],
                    ent[10], ent[11], ent[12], ent[13], ent[14], ent[15],
                ],
                first_lba,
                last_lba,
            });

            if ent[..16] == GPT_TYPE_ESP {
                continue;
            }

            let sectors = last_lba - first_lba + 1;
            if sectors == 0 {
                continue;
            }

            if first_non_esp.is_none() {
                first_non_esp = Some(DataRegion {
                    lba_start: first_lba,
                    sectors,
                });
            }
        }

        // Pair Helix with boot ESP's media: prefer the partition right after ESP.
        partitions.sort_unstable_by_key(|p| p.first_lba);
        if let Some((boot_idx, boot_part)) = partitions
            .iter()
            .enumerate()
            .find(|(_, p)| p.type_guid == GPT_TYPE_ESP)
        {
            let mut next_after_boot: Option<DataRegion> = None;
            for part in partitions.iter().skip(boot_idx + 1) {
                if part.type_guid == GPT_TYPE_ESP {
                    continue;
                }
                if part.first_lba <= boot_part.last_lba {
                    continue;
                }

                let sectors = part.last_lba - part.first_lba + 1;
                if sectors == 0 {
                    continue;
                }

                if part.first_lba == boot_part.last_lba.saturating_add(1) {
                    return Some(DataRegion {
                        lba_start: part.first_lba,
                        sectors,
                    });
                }

                if next_after_boot.is_none() {
                    next_after_boot = Some(DataRegion {
                        lba_start: part.first_lba,
                        sectors,
                    });
                }
            }

            if let Some(region) = next_after_boot {
                return Some(region);
            }
        }

        if let Some(region) =
            select_largest_gpt_free_region(first_usable, last_usable, &mut used_ranges, sector_size)
        {
            return Some(region);
        }

        if let Some(region) = first_non_esp {
            return Some(region);
        }

        return None;
    }

    if has_mbr {
        const MBR_PART_OFF: usize = 446;
        const MBR_PART_SIZE: usize = 16;
        const MBR_PARTS: usize = 4;
        let mut mbr_parts: [Option<(u8, u64, u64)>; MBR_PARTS] = [None, None, None, None];

        let mut best_start = 0u64;
        let mut best_sectors = 0u64;

        // Index `i` also computes the byte offset into the MBR table, not just a slice index.
        #[allow(clippy::needless_range_loop)]
        for i in 0..MBR_PARTS {
            let off = MBR_PART_OFF + (i * MBR_PART_SIZE);
            let ptype = first_two[off + 4];

            // empty / GPT protective / ESP
            if ptype == 0x00 || ptype == 0xEE || ptype == 0xEF {
                continue;
            }

            // extended partition containers
            if ptype == 0x05 || ptype == 0x0F || ptype == 0x85 {
                continue;
            }

            let start = le_u32(&first_two, off + 8) as u64;
            let sectors = le_u32(&first_two, off + 12) as u64;

            if start == 0 || sectors == 0 {
                continue;
            }

            if total_sectors != UNKNOWN_TOTAL_SECTORS
                && start.saturating_add(sectors) > total_sectors
            {
                continue;
            }

            mbr_parts[i] = Some((ptype, start, sectors));

            if sectors > best_sectors {
                best_start = start;
                best_sectors = sectors;
            }
        }

        if let Some((boot_idx, _)) = mbr_parts
            .iter()
            .enumerate()
            .find(|(_, p)| matches!(p, Some((0xEF, _, _))))
        {
            for part in mbr_parts.iter().skip(boot_idx + 1) {
                if let Some((ptype, start, sectors)) = *part {
                    if ptype == 0x00 || ptype == 0xEE || ptype == 0xEF {
                        continue;
                    }
                    if ptype == 0x05 || ptype == 0x0F || ptype == 0x85 {
                        continue;
                    }
                    if start == 0 || sectors == 0 {
                        continue;
                    }

                    return Some(DataRegion {
                        lba_start: start,
                        sectors,
                    });
                }
            }
        }

        if best_sectors != 0 {
            return Some(DataRegion {
                lba_start: best_start,
                sectors: best_sectors,
            });
        }

        return None;
    }

    if total_sectors == UNKNOWN_TOTAL_SECTORS {
        return None;
    }

    Some(DataRegion {
        lba_start: 0,
        sectors: total_sectors,
    })
}

static mut SPIN_ACTIVE: bool = false;
static mut SPIN_FRAME: usize = 0;
static mut SPIN_LAST_TSC: u64 = 0;

const SPIN_FRAMES: [u8; 4] = [b'|', b'/', b'-', b'\\'];

fn spinner_start() {
    unsafe {
        SPIN_ACTIVE = true;
        SPIN_FRAME = 0;
        SPIN_LAST_TSC = morpheus_hal_x86_64::cpu::tsc::read_tsc();
        morpheus_hal_x86_64::serial::serial_puts("   ");
        morpheus_hal_x86_64::serial::serial_putc(SPIN_FRAMES[0]);
        morpheus_hal_x86_64::serial::fb_puts("   ");
        morpheus_hal_x86_64::serial::fb_putc(SPIN_FRAMES[0]);
    }
}

/// Advances every ~100 ms; called from raw_read/raw_write so I/O drives the animation.
fn spinner_tick() {
    unsafe {
        if !SPIN_ACTIVE || STORAGE_TSC_FREQ == 0 {
            return;
        }
        let now = morpheus_hal_x86_64::cpu::tsc::read_tsc();
        let interval = STORAGE_TSC_FREQ / 10;
        if now.wrapping_sub(SPIN_LAST_TSC) < interval {
            return;
        }
        SPIN_LAST_TSC = now;
        SPIN_FRAME = (SPIN_FRAME + 1) % SPIN_FRAMES.len();
        let frame = SPIN_FRAMES[SPIN_FRAME];
        morpheus_hal_x86_64::serial::serial_putc(b'\x08');
        morpheus_hal_x86_64::serial::serial_putc(frame);
        morpheus_hal_x86_64::serial::fb_putc(b'\x08');
        morpheus_hal_x86_64::serial::fb_putc(frame);
    }
}

fn spinner_done() {
    unsafe {
        SPIN_ACTIVE = false;
    }
    morpheus_hal_x86_64::serial::serial_putc(b'\r');
    morpheus_hal_x86_64::serial::fb_puts("\r");
}

/// # Safety
/// `dma` must be the hwinit Phase 6 DMA region; `tsc_freq` calibrated; call once after hwinit.
pub unsafe fn init_persistent_storage(
    dma: &DmaRegion,
    tsc_freq: u64,
    pre_ebs_helix: Option<crate::boot::PreEbsHelixImage>,
) {
    if let Some(img) = pre_ebs_helix {
        RAM_HELIX_DEVICE = Some(MemBlockDevice::new(
            img.base as *mut u8,
            img.size,
            img.sector_size,
        ));
        if let Some(mem_dev) = RAM_HELIX_DEVICE.as_mut() {
            match morpheus_helix::vfs::global::replace_root_device(
                MemBlockDevice::into_raw(mem_dev),
                false,
            ) {
                Ok(()) => {
                    if root_path_exists("/bin/init") {
                        PERSISTENT_READY = true;
                        return;
                    }
                    log_warn(
                        "STORAGE",
                        827,
                        "pre-EBS staged root missing /bin/init; falling back to probe",
                    );
                },
                Err(_) => {
                    log_warn(
                        "STORAGE",
                        827,
                        "pre-EBS staged root mount failed; falling back to probe",
                    );
                },
            }
        }
    }

    // dump_pci_devices(); // enable when debugging device discovery

    STORAGE_DMA = Some(*dma);
    STORAGE_TSC_FREQ = tsc_freq;

    // DMA region is zeroed by hwinit Phase 6; VirtIO queue structs are clean.

    let base_cpu = dma.cpu_base();
    let base_bus = dma.bus_base();

    let config = BlockDmaConfig {
        tsc_freq,

        virtio_desc_cpu: base_cpu.add(OFF_VIRTIO_DESC),
        virtio_desc_phys: base_bus + OFF_VIRTIO_DESC as u64,
        virtio_avail_cpu: base_cpu.add(OFF_VIRTIO_AVAIL),
        virtio_avail_phys: base_bus + OFF_VIRTIO_AVAIL as u64,
        virtio_used_cpu: base_cpu.add(OFF_VIRTIO_USED),
        virtio_used_phys: base_bus + OFF_VIRTIO_USED as u64,
        virtio_headers_cpu: base_cpu.add(OFF_VIRTIO_HEADERS),
        virtio_headers_phys: base_bus + OFF_VIRTIO_HEADERS as u64,
        virtio_status_cpu: base_cpu.add(OFF_VIRTIO_STATUS),
        virtio_status_phys: base_bus + OFF_VIRTIO_STATUS as u64,
        virtio_notify_addr: 0, // driver fills from PCI caps
        queue_size: VIRTIO_QUEUE_SIZE,

        ahci_cmd_list_cpu: base_cpu.add(OFF_AHCI_CMD_LIST),
        ahci_cmd_list_phys: base_bus + OFF_AHCI_CMD_LIST as u64,
        ahci_fis_cpu: base_cpu.add(OFF_AHCI_FIS),
        ahci_fis_phys: base_bus + OFF_AHCI_FIS as u64,
        ahci_cmd_tables_cpu: base_cpu.add(OFF_AHCI_CMD_TABLES),
        ahci_cmd_tables_phys: base_bus + OFF_AHCI_CMD_TABLES as u64,
        ahci_identify_cpu: base_cpu.add(OFF_AHCI_IDENTIFY),
        ahci_identify_phys: base_bus + OFF_AHCI_IDENTIFY as u64,
    };

    let (devices, dev_count) = scan_all_block_devices();

    if dev_count == 0 {
        log_warn("STORAGE", 823, "no block device found; using RAM-disk");
        return;
    }
    let _ = dev_count;

    // Skip boot disks (GPT/MBR), accept first blank or HelixFS device.
    let mut found_data_disk = false;
    let mut saw_unimplemented_backend = false;
    #[allow(clippy::needless_range_loop)]
    'device_scan: for i in 0..dev_count {
        let detected = match &devices[i] {
            Some(d) => d,
            None => continue,
        };

        // Map BARs UC before driver touches them.
        if let DetectedBlockDevice::VirtIO { pci_addr, .. } = detected {
            map_virtio_bars(pci_addr.bus, pci_addr.device, pci_addr.function);
        } else if let DetectedBlockDevice::Ahci(info) = detected {
            if is_paging_initialized() {
                // ABAR: HBA regs + 32 ports × 0x80 = 0x1100, round to 2 pages.
                let _ = kmap_mmio(info.abar, 0x2000);
            }
        } else if let DetectedBlockDevice::Sdhci(info) = detected {
            if is_paging_initialized() {
                let _ = kmap_mmio(info.mmio_base, 0x1000);
            }
        } else if let DetectedBlockDevice::UsbMsd(info) = detected {
            if is_paging_initialized() {
                let _ = kmap_mmio(info.mmio_base, 0x4000);
            }
        }

        let is_ahci = matches!(detected, DetectedBlockDevice::Ahci(_));

        // Mask BSP interrupts across driver init: 100 Hz LAPIC timer ISRs
        // mid-PCH-MMIO extend bus cycles on real Intel silicon (same root
        // cause as the AHCI BIOS/OS handoff stall). Init polls on TSC.
        morpheus_hal_x86_64::cpu::idt::disable_interrupts();
        let device_result = create_unified_from_detected(detected, &config);
        morpheus_hal_x86_64::cpu::idt::enable_interrupts();

        let device = match device_result {
            Ok(dev) => dev,
            Err(err) => {
                match err {
                    UnifiedBlockError::AhciError(e) => {
                        match e {
                            AhciInitError::InvalidConfig => {
                                log_warn("STORAGE", 825, "AHCI init failed: invalid config");
                            },
                            AhciInitError::ResetFailed => {
                                log_warn("STORAGE", 825, "AHCI init failed: HBA reset timeout");
                            },
                            AhciInitError::NoDeviceFound => {
                                log_warn("STORAGE", 825, "AHCI init failed: no SATA device found");
                            },
                            AhciInitError::PortStopTimeout => {
                                log_warn("STORAGE", 825, "AHCI init failed: port stop timeout");
                            },
                            AhciInitError::PortStartFailed => {
                                log_warn("STORAGE", 825, "AHCI init failed: port start failed");
                            },
                            AhciInitError::IdentifyFailed => {
                                log_warn("STORAGE", 825, "AHCI init failed: IDENTIFY failed");
                            },
                            AhciInitError::No64BitSupport => {
                                log_warn("STORAGE", 825, "AHCI init failed: no 64-bit DMA support");
                            },
                            AhciInitError::DeviceNotResponding => {
                                log_warn("STORAGE", 825, "AHCI init failed: device not responding");
                            },
                            AhciInitError::DmaSetupFailed => {
                                log_warn("STORAGE", 825, "AHCI init failed: DMA setup failed");
                            },
                        }
                        log_warn("STORAGE", 825, "AHCI candidate skipped");
                    },
                    UnifiedBlockError::VirtioError(e) => {
                        match e {
                            VirtioBlkInitError::ResetFailed => {
                                log_warn("STORAGE", 825, "VirtIO init failed: reset failed");
                            },
                            VirtioBlkInitError::FeatureNegotiationFailed => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "VirtIO init failed: feature negotiation failed",
                                );
                            },
                            VirtioBlkInitError::QueueSetupFailed => {
                                log_warn("STORAGE", 825, "VirtIO init failed: queue setup failed");
                            },
                            VirtioBlkInitError::DeviceFailed => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "VirtIO init failed: device failed status",
                                );
                            },
                            VirtioBlkInitError::InvalidConfig => {
                                log_warn("STORAGE", 825, "VirtIO init failed: invalid config");
                            },
                            VirtioBlkInitError::TransportError => {
                                log_warn("STORAGE", 825, "VirtIO init failed: transport error");
                            },
                        }
                        log_warn("STORAGE", 825, "VirtIO candidate skipped");
                    },
                    UnifiedBlockError::NoDevice => {
                        if is_ahci {
                            log_warn(
                                "STORAGE",
                                825,
                                "AHCI controller init failed; skipping candidate",
                            );
                        } else {
                            log_warn(
                                "STORAGE",
                                825,
                                "driver init failed for one candidate; skipping",
                            );
                        }
                    },
                    UnifiedBlockError::SdhciError(e) => {
                        match e {
                            SdhciInitError::InvalidConfig => {
                                log_warn("STORAGE", 825, "SDHCI init failed: invalid config");
                            },
                            SdhciInitError::ControllerResetFailed => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "SDHCI init failed: controller reset failed",
                                );
                            },
                            SdhciInitError::NoCardPresent => {
                                log_warn("STORAGE", 825, "SDHCI init failed: no card present");
                            },
                            SdhciInitError::VoltageSwitchFailed => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "SDHCI init failed: voltage switch failed",
                                );
                            },
                            SdhciInitError::ClockSetupFailed => {
                                log_warn("STORAGE", 825, "SDHCI init failed: clock setup failed");
                            },
                            SdhciInitError::CommandTimeout => {
                                log_warn("STORAGE", 825, "SDHCI init failed: command timeout");
                            },
                            SdhciInitError::DataTimeout => {
                                log_warn("STORAGE", 825, "SDHCI init failed: data timeout");
                            },
                            SdhciInitError::IoError => {
                                log_warn("STORAGE", 825, "SDHCI init failed: I/O error");
                            },
                            SdhciInitError::NotImplemented => {
                                saw_unimplemented_backend = true;
                                log_warn("STORAGE", 825, "SDHCI init failed: not implemented");
                            },
                        }
                        log_warn("STORAGE", 825, "SDHCI candidate skipped");
                    },
                    UnifiedBlockError::UsbMsdError(e) => {
                        match e {
                            UsbMsdInitError::InvalidConfig => {
                                log_warn("STORAGE", 825, "USB-MSD init failed: invalid config");
                            },
                            UsbMsdInitError::ControllerInitFailed => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "USB-MSD init failed: controller init failed",
                                );
                            },
                            UsbMsdInitError::ControllerProbeFailed => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "USB-MSD init failed: controller probe failed",
                                );
                            },
                            UsbMsdInitError::ControllerResetFailed => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "USB-MSD init failed: controller reset failed",
                                );
                            },
                            UsbMsdInitError::ControllerScratchpadUnsupported => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "USB-MSD init failed: scratchpad requirement unsupported",
                                );
                            },
                            UsbMsdInitError::ControllerStartFailed => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "USB-MSD init failed: controller start failed (HCH stuck)",
                                );
                            },
                            UsbMsdInitError::HubUnsupported => {
                                log_warn("STORAGE", 825, "USB-MSD init failed: USB hub detected; downstream hub traversal not implemented");
                            },
                            UsbMsdInitError::PortResetFailed => {
                                log_warn("STORAGE", 825, "USB-MSD init failed: port reset failed");
                            },
                            UsbMsdInitError::PortResetTimeout => {
                                log_warn("STORAGE", 825, "USB-MSD init failed: port reset timeout");
                            },
                            UsbMsdInitError::PortResetHotCmdTimeout => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "USB-MSD init failed: hot reset command timeout",
                                );
                            },
                            UsbMsdInitError::PortResetHotSettleTimeout => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "USB-MSD init failed: hot reset settle timeout",
                                );
                            },
                            UsbMsdInitError::PortResetWarmTimeout => {
                                log_warn("STORAGE", 825, "USB-MSD init failed: warm reset timeout");
                            },
                            UsbMsdInitError::PortResetNoLink => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "USB-MSD init failed: USB link not usable",
                                );
                            },
                            UsbMsdInitError::EnableSlotFailed => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "USB-MSD init failed: enable-slot command failed",
                                );
                            },
                            UsbMsdInitError::AddressDeviceFailed => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "USB-MSD init failed: address-device command failed",
                                );
                            },
                            UsbMsdInitError::DeviceDescriptorFailed => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "USB-MSD init failed: GET_DESCRIPTOR(device) failed",
                                );
                            },
                            UsbMsdInitError::ConfigDescriptorFailed => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "USB-MSD init failed: GET_DESCRIPTOR(config) failed",
                                );
                            },
                            UsbMsdInitError::MassStorageProtocolUnsupported => {
                                log_warn("STORAGE", 825, "USB-MSD init failed: mass-storage protocol unsupported (need BOT)");
                            },
                            UsbMsdInitError::NoBotMassStorageInterface => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "USB-MSD init failed: no BOT mass-storage interface found",
                                );
                            },
                            UsbMsdInitError::ActivePortsNoConnectedDevice => {
                                log_warn("STORAGE", 825, "USB-MSD init failed: root ports active but no connected device detected");
                            },
                            UsbMsdInitError::SetConfigurationFailed => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "USB-MSD init failed: SET_CONFIGURATION failed",
                                );
                            },
                            UsbMsdInitError::ConfigureEndpointsFailed => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "USB-MSD init failed: configure endpoint command failed",
                                );
                            },
                            UsbMsdInitError::DeviceEnumerationFailed => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "USB-MSD init failed: device enumeration failed",
                                );
                            },
                            UsbMsdInitError::TransportInitFailed => {
                                log_warn(
                                    "STORAGE",
                                    825,
                                    "USB-MSD init failed: transport init failed",
                                );
                            },
                            UsbMsdInitError::NoMedia => {
                                log_warn("STORAGE", 825, "USB-MSD init failed: no media");
                            },
                            UsbMsdInitError::CommandTimeout => {
                                log_warn("STORAGE", 825, "USB-MSD init failed: command timeout");
                            },
                            UsbMsdInitError::IoError => {
                                log_warn("STORAGE", 825, "USB-MSD init failed: I/O error");
                            },
                            UsbMsdInitError::NotImplemented => {
                                saw_unimplemented_backend = true;
                                log_warn("STORAGE", 825, "USB-MSD init failed: not implemented");
                            },
                        }
                        log_warn("STORAGE", 825, "USB-MSD candidate skipped");
                    },
                }
                continue;
            },
        };

        let info = device.info();

        // Park here so select_data_region/recovery probes can use it.
        BLOCK_DEVICE = Some(device);

        let region = match select_data_region(info.sector_size, info.total_sectors) {
            Some(r) => r,
            None => {
                BLOCK_DEVICE = None;
                continue;
            },
        };

        STORAGE_LBA_BASE = region.lba_start;
        STORAGE_REGION_SECTORS = region.sectors;

        if region.sectors == 0 {
            BLOCK_DEVICE = None;
            continue;
        }

        // Commit only after proving the candidate holds HelixFS.
        let _raw_dev = make_raw_block_device();

        let needs_format = {
            let mut probe_dev = make_raw_block_device();
            match morpheus_helix::log::recovery::recover_superblock(
                &mut probe_dev,
                0,
                info.sector_size,
            ) {
                Ok(sb) => {
                    // v2 changed the log payload format.
                    if sb.version != morpheus_helix::types::HELIX_VERSION {
                        log_warn(
                            "STORAGE",
                            828,
                            "helixfs version mismatch; reformat required",
                        );
                        true
                    } else {
                        false
                    }
                },
                Err(_) => true,
            }
        };

        if needs_format {
            // Bring-up safety: never auto-format unknown disks. Keep scanning.
            log_warn(
                "STORAGE",
                829,
                "no valid helixfs on candidate; skipping disk",
            );
            BLOCK_DEVICE = None;
            continue;
        }

        let mut mounted_from_ram = false;
        let mount_dev = match stage_selected_region_to_ram(info.sector_size) {
            Some(mem_dev) => {
                mounted_from_ram = true;
                mem_dev
            },
            None => {
                let reason = RAM_STAGE_LAST_REASON;
                if reason == "none" {
                    log_warn(
                        "STORAGE",
                        838,
                        "RAM staging failed; mounting directly from media",
                    );
                } else {
                    log_warn("STORAGE", 838, reason);
                }
                make_raw_block_device()
            },
        };

        spinner_start();
        match morpheus_helix::vfs::global::replace_root_device(mount_dev, false) {
            Ok(()) => {
                spinner_done();
                let mut root_has_init = root_path_exists("/bin/init");
                if mounted_from_ram && !root_path_exists("/bin/init") {
                    log_warn(
                        "STORAGE",
                        844,
                        "RAM-staged root missing /bin/init; remounting directly from media",
                    );
                    spinner_start();
                    match morpheus_helix::vfs::global::replace_root_device(
                        make_raw_block_device(),
                        false,
                    ) {
                        Ok(()) => {
                            spinner_done();
                            if root_path_exists("/bin/init") {
                                root_has_init = true;
                            } else {
                                root_has_init = false;
                                log_warn(
                                    "STORAGE",
                                    846,
                                    "direct-media root still missing /bin/init",
                                );
                            }
                        },
                        Err(_) => {
                            spinner_done();
                            root_has_init = false;
                            log_warn("STORAGE", 847, "direct-media root remount failed");
                        },
                    }
                }

                if !root_has_init {
                    log_warn(
                        "STORAGE",
                        851,
                        "candidate root rejected: /bin/init missing; scanning next disk",
                    );
                    BLOCK_DEVICE = None;
                    continue;
                }

                PERSISTENT_READY = true;
                found_data_disk = true;
                if !root_path_exists("/bin/init") {
                    log_warn("STORAGE", 848, "root check: /bin/init missing");
                }
                if !root_path_exists("/bin/compd") {
                    log_warn("STORAGE", 849, "root check: /bin/compd missing");
                }
                if !root_path_exists("/bin/shelld") {
                    log_warn("STORAGE", 850, "root check: /bin/shelld missing");
                }
                break 'device_scan;
            },
            Err(e) => {
                spinner_done();
                let _ = e;
                log_error("STORAGE", 835, "failed to mount persistent filesystem");
                BLOCK_DEVICE = None;
                continue;
            },
        }
    }

    if !found_data_disk {
        log_warn(
            "STORAGE",
            836,
            "no suitable data disk; using RAM-disk fallback",
        );
        log_warn(
            "STORAGE",
            839,
            "runtime persistent backends currently support AHCI/VirtIO/SDHCI (USB/NVMe pending)",
        );
        if saw_unimplemented_backend {
            log_error(
                "STORAGE",
                852,
                "boot medium backend is scaffold-only (SDHCI/USB-MSD not implemented); /bin/init will be unavailable",
            );
        }
    }
}

fn root_path_exists(path: &str) -> bool {
    let fs = match unsafe { morpheus_helix::vfs::global::fs_global_mut() } {
        Some(f) => f,
        None => return false,
    };

    morpheus_helix::vfs::vfs_stat(&fs.mount_table, path).is_ok()
}

pub fn is_persistent() -> bool {
    unsafe { PERSISTENT_READY }
}

/// True if LBA0/1 holds an MBR sig or GPT magic — never auto-format these.
unsafe fn is_boot_disk(sector_size: u32) -> bool {
    let dev = match BLOCK_DEVICE.as_mut() {
        Some(d) => d,
        None => return false,
    };
    let dma = match STORAGE_DMA.as_ref() {
        Some(d) => d,
        None => return false,
    };

    let io_cpu = dma.cpu_base().add(OFF_IO_BUFFER);
    let io_phys = dma.bus_at(OFF_IO_BUFFER);
    let io_buf = core::slice::from_raw_parts_mut(io_cpu, IO_BUFFER_SIZE);
    let timeout = STORAGE_TSC_FREQ * 5;

    let mut bio = match UnifiedBlockIo::new(dev, io_buf, io_phys, timeout) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let read_size = (sector_size as usize) * 2;
    let mut buf = alloc::vec![0u8; read_size];

    use gpt_disk_io::BlockIo as GptBlockIo;
    use gpt_disk_types::Lba as GptLba;
    if bio.read_blocks(GptLba(0), &mut buf).is_err() {
        return false;
    }

    if buf.len() >= 512 && buf[510] == 0x55 && buf[511] == 0xAA {
        return true;
    }

    let gpt_offset = sector_size as usize;
    if buf.len() >= gpt_offset + 8 && &buf[gpt_offset..gpt_offset + 8] == b"EFI PART" {
        return true;
    }

    false
}

/// Copy selected Helix partition into RAM, return a RawBlockDevice over it.
unsafe fn stage_selected_region_to_ram(sector_size: u32) -> Option<RawBlockDevice> {
    RAM_STAGE_LAST_REASON = "none";

    let mut probe = make_raw_block_device();
    let sb = match morpheus_helix::log::recovery::recover_superblock(&mut probe, 0, sector_size) {
        Ok(sb) => sb,
        Err(_) => {
            RAM_STAGE_LAST_REASON =
                "RAM stage: superblock probe failed; mounting directly from media";
            return None;
        },
    };

    // Live FS footprint only, not full partition capacity.
    let mut stage_blocks = 2u64;
    let log_hi = sb.log_end_block.saturating_add(1);
    if log_hi > stage_blocks {
        stage_blocks = log_hi;
    }

    let data_hi = sb.data_start_block.saturating_add(sb.blocks_used);
    if data_hi > stage_blocks {
        stage_blocks = data_hi;
    }

    if stage_blocks > sb.total_blocks {
        stage_blocks = sb.total_blocks;
    }

    if stage_blocks == 0 {
        RAM_STAGE_LAST_REASON = "RAM stage: empty footprint; mounting directly from media";
        return None;
    }

    let fs_bytes = match stage_blocks.checked_mul(sb.block_size as u64) {
        Some(v) => v,
        None => {
            RAM_STAGE_LAST_REASON = "RAM stage: byte size overflow; mounting directly from media";
            return None;
        },
    };
    if fs_bytes == 0 {
        return None;
    }

    if fs_bytes > RAM_STAGE_MAX_BYTES {
        RAM_STAGE_LAST_REASON =
            "RAM stage: footprint exceeds RAM cap; mounting directly from media";
        return None;
    }

    if fs_bytes > usize::MAX as u64 {
        RAM_STAGE_LAST_REASON = "RAM stage: size exceeds usize; mounting directly from media";
        return None;
    }

    let sector_bytes = sector_size as usize;
    let mut copy_bytes = fs_bytes as usize;
    let rem = copy_bytes % sector_bytes;
    if rem != 0 {
        copy_bytes = match copy_bytes.checked_add(sector_bytes - rem) {
            Some(v) => v,
            None => {
                RAM_STAGE_LAST_REASON =
                    "RAM stage: alignment overflow; mounting directly from media";
                return None;
            },
        };
    }

    let region_bytes = match (STORAGE_REGION_SECTORS as usize).checked_mul(sector_bytes) {
        Some(v) => v,
        None => {
            RAM_STAGE_LAST_REASON = "RAM stage: region size overflow; mounting directly from media";
            return None;
        },
    };
    if copy_bytes > region_bytes {
        RAM_STAGE_LAST_REASON =
            "RAM stage: footprint exceeds selected region; mounting directly from media";
        return None;
    }

    let stage_pages = copy_bytes.div_ceil(4096);
    let stage_base = {
        let mut registry = global_registry_mut();
        match registry.allocate_pages(
            AllocateType::AnyPages,
            MemoryType::LoaderData,
            stage_pages as u64,
        ) {
            Ok(p) => p,
            Err(_) => {
                RAM_STAGE_LAST_REASON =
                    "RAM stage: page allocation failed; mounting directly from media";
                return None;
            },
        }
    };

    if stage_base == 0 {
        RAM_STAGE_LAST_REASON = "RAM stage: zero allocation base; mounting directly from media";
        return None;
    }

    let image = core::slice::from_raw_parts_mut(stage_base as *mut u8, copy_bytes);

    const CHUNK_SECTORS: usize = 256;
    let chunk_bytes = CHUNK_SECTORS * sector_bytes;
    let mut lba = 0u64;
    let mut off = 0usize;

    while off < copy_bytes {
        let this_chunk = core::cmp::min(chunk_bytes, copy_bytes - off);
        if !raw_read(
            core::ptr::null_mut(),
            lba,
            image.as_mut_ptr().add(off),
            this_chunk,
        ) {
            RAM_STAGE_LAST_REASON = "RAM stage: media read failed; mounting directly from media";
            return None;
        }

        off += this_chunk;
        lba = lba.saturating_add((this_chunk / sector_bytes) as u64);
    }

    let base = image.as_mut_ptr();
    let size = image.len();

    RAM_HELIX_DEVICE = Some(MemBlockDevice::new(base, size, sector_size));
    let mem_dev = RAM_HELIX_DEVICE.as_mut()?;
    Some(MemBlockDevice::into_raw(mem_dev))
}

/// Idempotent; call after root FS is mounted.
pub fn create_init_directories() {
    use morpheus_hal_x86_64::cpu::tsc::read_tsc;

    let dirs = ["/bin", "/etc", "/tmp", "/home", "/var", "/dev"];

    let ts = read_tsc();

    let fs = match unsafe { morpheus_helix::vfs::global::fs_global_mut() } {
        Some(f) => f,
        None => {
            log_warn("INITFS", 840, "no root fs; skipping directory bootstrap");
            return;
        },
    };

    for dir in &dirs {
        match morpheus_helix::vfs::vfs_mkdir(&mut fs.mount_table, dir, ts) {
            Ok(_) => {},
            Err(morpheus_helix::error::HelixError::AlreadyExists) => {},
            Err(_) => {
                let _ = dir;
                log_warn("INITFS", 841, "failed to create one startup directory");
            },
        }
    }
}

// RawBlockDevice fn-ptr vtable → UnifiedBlockIo, reuses chunked DMA r/w + timeout.

/// # Safety
/// `BLOCK_DEVICE` must be `Some`.
unsafe fn make_raw_block_device() -> RawBlockDevice {
    let dev = BLOCK_DEVICE.as_ref().unwrap();
    let info = dev.info();
    let total = if STORAGE_REGION_SECTORS != 0 {
        STORAGE_REGION_SECTORS
    } else {
        info.total_sectors
    };

    RawBlockDevice::new(
        core::ptr::null_mut(), // ctx unused; callbacks access statics
        total,
        info.sector_size,
        raw_read,
        raw_write,
        raw_flush,
    )
}

unsafe fn raw_read(_ctx: *mut u8, lba: u64, dst: *mut u8, len: usize) -> bool {
    spinner_tick();
    let dev = match BLOCK_DEVICE.as_mut() {
        Some(d) => d,
        None => return false,
    };
    let dma = match STORAGE_DMA.as_ref() {
        Some(d) => d,
        None => return false,
    };

    let io_cpu = dma.cpu_base().add(OFF_IO_BUFFER);
    let io_phys = dma.bus_at(OFF_IO_BUFFER);
    let io_buf = core::slice::from_raw_parts_mut(io_cpu, IO_BUFFER_SIZE);
    let timeout = STORAGE_TSC_FREQ * 5;

    let mut bio = match UnifiedBlockIo::new(dev, io_buf, io_phys, timeout) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let dst_slice = core::slice::from_raw_parts_mut(dst, len);

    use gpt_disk_io::BlockIo as GptBlockIo;
    use gpt_disk_types::Lba as GptLba;
    bio.read_blocks(GptLba(lba + STORAGE_LBA_BASE), dst_slice)
        .is_ok()
}

unsafe fn raw_write(_ctx: *mut u8, lba: u64, src: *const u8, len: usize) -> bool {
    spinner_tick();
    let dev = match BLOCK_DEVICE.as_mut() {
        Some(d) => d,
        None => return false,
    };
    let dma = match STORAGE_DMA.as_ref() {
        Some(d) => d,
        None => return false,
    };

    let io_cpu = dma.cpu_base().add(OFF_IO_BUFFER);
    let io_phys = dma.bus_at(OFF_IO_BUFFER);
    let io_buf = core::slice::from_raw_parts_mut(io_cpu, IO_BUFFER_SIZE);
    let timeout = STORAGE_TSC_FREQ * 5;

    let mut bio = match UnifiedBlockIo::new(dev, io_buf, io_phys, timeout) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let src_slice = core::slice::from_raw_parts(src, len);

    use gpt_disk_io::BlockIo as GptBlockIo;
    use gpt_disk_types::Lba as GptLba;
    bio.write_blocks(GptLba(lba + STORAGE_LBA_BASE), src_slice)
        .is_ok()
}

unsafe fn raw_flush(_ctx: *mut u8) -> bool {
    let dev = match BLOCK_DEVICE.as_mut() {
        Some(d) => d,
        None => return false,
    };

    dev.flush().is_ok()
}
