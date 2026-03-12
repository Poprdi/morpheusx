//! Persistent storage subsystem.
//!
//! Probes for a real block device (VirtIO-blk or AHCI) via the unified
//! block device layer in `morpheus-network`, then bridges it to HelixFS
//! through a [`RawBlockDevice`] function-pointer vtable.
//!
//! On success, replaces the RAM-disk root filesystem with the persistent
//! device.  If no block device is found, the RAM-disk stays in place.
//!
//! # DMA Layout (within hwinit 2 MB DMA region)
//!
//! ```text
//! Offset     Size     Purpose
//! 0x00000    512 B    VirtIO descriptor table (32 entries × 16 B)
//! 0x00200    70 B     VirtIO available ring
//! 0x00400    262 B    VirtIO used ring
//! 0x01000    512 B    VirtIO request headers (page-aligned)
//! 0x01200    32 B     VirtIO status bytes
//! 0x02000    1 KB     AHCI command list (1 KB-aligned)
//! 0x02400    256 B    AHCI FIS receive buffer
//! 0x02800    8 KB     AHCI command tables (128-byte aligned)
//! 0x04800    512 B    AHCI IDENTIFY buffer
//! 0x10000    64 KB    I/O DMA buffer (for UnifiedBlockIo transfers)
//! Total      ≈ 128 KB (well within 2 MB)
//! ```

use morpheus_helix::device::RawBlockDevice;
use morpheus_hwinit::dma::DmaRegion;
use morpheus_hwinit::paging::is_paging_initialized;
use morpheus_hwinit::serial::{log_error, log_info, log_ok, log_warn, puts};
use morpheus_hwinit::{kmap_mmio, pci_cfg_read16, pci_cfg_read32, PciAddr};
use morpheus_network::{
    create_unified_from_detected, create_unified_from_detected_ahci_port, scan_all_block_devices,
    BlockDmaConfig, BlockDriver, DetectedBlockDevice, UnifiedBlockDevice, UnifiedBlockIo,
};

// DMA LAYOUT CONSTANTS

const VIRTIO_QUEUE_SIZE: u16 = 32;

// VirtIO virtqueue structures
const OFF_VIRTIO_DESC: usize = 0x0_0000;
const OFF_VIRTIO_AVAIL: usize = 0x0_0200;
const OFF_VIRTIO_USED: usize = 0x0_0400;
const OFF_VIRTIO_HEADERS: usize = 0x0_1000; // page-aligned
const OFF_VIRTIO_STATUS: usize = 0x0_1200;

// AHCI structures
const OFF_AHCI_CMD_LIST: usize = 0x0_2000; // 1 KB-aligned
const OFF_AHCI_FIS: usize = 0x0_2400; // 256-byte aligned
const OFF_AHCI_CMD_TABLES: usize = 0x0_2800; // 128-byte aligned
const OFF_AHCI_IDENTIFY: usize = 0x0_4800;

// I/O transfer buffer — used by UnifiedBlockIo for synchronous read/write
const OFF_IO_BUFFER: usize = 0x1_0000;
const IO_BUFFER_SIZE: usize = 64 * 1024; // 64 KB = UnifiedBlockIo::MAX_TRANSFER_SIZE

// PCI BUS DUMP (diagnostic)

/// Dump all PCI devices with vendor/device IDs and BARs to serial.
///
/// This is invaluable for verifying that VirtIO-blk devices are present
/// and that OVMF assigned BAR addresses we can actually map.
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
            let class_code = pci_cfg_read32(addr, 0x08); // rev + class
            let cmd = pci_cfg_read16(addr, 0x04);

            puts("[PCI-DUMP]   00:");
            morpheus_hwinit::serial::put_hex8(dev);
            puts(".");
            morpheus_hwinit::serial::put_hex8(func);
            puts("  ven=");
            morpheus_hwinit::serial::put_hex32(vendor_id as u32);
            puts(" dev=");
            morpheus_hwinit::serial::put_hex32(device_id as u32);
            puts(" class=");
            morpheus_hwinit::serial::put_hex32(class_code >> 8); // class/subclass/progif
            puts(" cmd=");
            morpheus_hwinit::serial::put_hex32(cmd as u32);
            puts("\n");

            // Print BARs for VirtIO devices (vendor 0x1AF4)
            if vendor_id == 0x1AF4 {
                let mut bar_i = 0u8;
                while bar_i < 6 {
                    let raw = pci_cfg_read32(addr, 0x10 + bar_i * 4);
                    if raw != 0 {
                        let is_io = raw & 0x01 != 0;
                        let is_64 = !is_io && (raw >> 1) & 0x03 == 0x02;
                        puts("[PCI-DUMP]     BAR");
                        morpheus_hwinit::serial::put_hex8(bar_i);
                        puts(" raw=");
                        morpheus_hwinit::serial::put_hex32(raw);
                        if is_64 && bar_i < 5 {
                            let high = pci_cfg_read32(addr, 0x10 + (bar_i + 1) * 4);
                            puts(" BAR");
                            morpheus_hwinit::serial::put_hex8(bar_i + 1);
                            puts("=");
                            morpheus_hwinit::serial::put_hex32(high);
                            let full = ((high as u64) << 32) | ((raw & 0xFFFFFFF0) as u64);
                            puts(" -> ");
                            morpheus_hwinit::serial::put_hex64(full);
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

// MMIO BAR MAPPING

/// Identity-map PCI BAR MMIO regions for a VirtIO device with UC flags.
///
/// We do NOT rely on UEFI's page table mappings for MMIO space.
/// Instead we use `map_mmio()` which handles every case:
///   - Region inside an existing huge page → sets UC bits on it
///   - Region already mapped as 4K pages → sets UC bits
///   - Region not mapped at all → creates new identity-mapped UC entries
///
/// Each memory BAR gets a 16 KiB mapped region (4 × 4 KiB) — enough for
/// VirtIO common, ISR, device and notify capability structures.
///
/// # Safety
/// - Paging and MemoryRegistry must be initialized.
unsafe fn map_virtio_bars(bus: u8, dev: u8, func: u8) {
    if !is_paging_initialized() {
        log_warn("STORAGE", 820, "paging not initialized; skipping BAR UC mapping");
        return;
    }

    let addr = PciAddr::new(bus, dev, func);
    // 16 KiB covers all 4 VirtIO cap regions (each ~4 KiB, contiguous)
    const MAP_SIZE: u64 = 16 * 1024;

    // Walk BAR0..BAR5.  Skip 64-bit BAR high halves.
    let mut bar_idx = 0u8;
    while bar_idx < 6 {
        let bar_offset = 0x10u8 + bar_idx * 4;
        let bar_low = pci_cfg_read32(addr, bar_offset);

        if bar_low == 0 || bar_low & 0x01 != 0 {
            // Absent or I/O BAR — skip
            bar_idx += 1;
            continue;
        }

        // Memory BAR — check type (bits 2:1)
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
                Ok(()) => {}
                Err(e) => {
                    let _ = (bar_idx, e);
                    log_warn("STORAGE", 821, "map_mmio for VirtIO BAR failed");
                }
            }
        }

        bar_idx += if is_64bit { 2 } else { 1 };
    }
}

// STATIC STATE

/// The unified block device lives here for the kernel's lifetime.
static mut BLOCK_DEVICE: Option<UnifiedBlockDevice> = None;

/// DMA region reference (Copy — stored from init).
static mut STORAGE_DMA: Option<DmaRegion> = None;

/// TSC frequency for timeout computation.
static mut STORAGE_TSC_FREQ: u64 = 0;
/// Start LBA of selected persistent region (0 = whole disk).
static mut STORAGE_LBA_BASE: u64 = 0;
/// Sector count of selected persistent region.
static mut STORAGE_REGION_SECTORS: u64 = 0;

/// Whether persistent storage was successfully initialized.
static mut PERSISTENT_READY: bool = false;

const GPT_SIG: &[u8; 8] = b"EFI PART";
// EFI System Partition type GUID on disk (little-endian fields).
const GPT_TYPE_ESP: [u8; 16] = [
    0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11, 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9,
    0x3B,
];

#[derive(Clone, Copy)]
struct DataRegion {
    lba_start: u64,
    sectors: u64,
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

/// Select a writable data region from the currently active block device.
///
/// Policy:
/// - GPT disk: pick first non-ESP partition
/// - MBR-only disk: skip (treated as boot/system for now)
/// - No partition table: whole disk is data region
unsafe fn select_data_region(sector_size: u32, total_sectors: u64) -> Option<DataRegion> {
    let dev = BLOCK_DEVICE.as_mut()?;
    let dma = STORAGE_DMA.as_ref()?;

    let io_cpu = dma.cpu_base().add(OFF_IO_BUFFER);
    let io_phys = dma.bus_at(OFF_IO_BUFFER);
    let io_buf = core::slice::from_raw_parts_mut(io_cpu, IO_BUFFER_SIZE);
    let timeout = STORAGE_TSC_FREQ * 5;

    let mut bio = UnifiedBlockIo::new(dev, io_buf, io_phys, timeout).ok()?;

    use morpheus_network::GptBlockIo;
    use morpheus_network::GptLba;

    let mut first_two = alloc::vec![0u8; (sector_size as usize) * 2];
    if bio.read_blocks(GptLba(0), &mut first_two).is_err() {
        return None;
    }

    let has_mbr = first_two.len() >= 512 && first_two[510] == 0x55 && first_two[511] == 0xAA;
    let gpt_off = sector_size as usize;
    let has_gpt = first_two.len() >= gpt_off + 8 && &first_two[gpt_off..gpt_off + 8] == GPT_SIG;

    if has_gpt {
        let hdr = &first_two[gpt_off..gpt_off + sector_size as usize];
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

        for idx in 0..num_entries {
            let sector_delta = idx / entries_per_sector;
            let idx_in_sector = idx % entries_per_sector;
            let lba = entries_lba + sector_delta as u64;

            if bio.read_blocks(GptLba(lba), &mut sec).is_err() {
                return None;
            }

            let off = idx_in_sector * entry_size;
            let ent = &sec[off..off + entry_size];

            // Empty partition entry if type GUID is all zero.
            if ent[..16].iter().all(|b| *b == 0) {
                continue;
            }

            // Skip ESP partition.
            if ent[..16] == GPT_TYPE_ESP {
                continue;
            }

            let first_lba = le_u64(ent, 32);
            let last_lba = le_u64(ent, 40);
            if first_lba == 0 || last_lba < first_lba {
                continue;
            }

            let sectors = last_lba - first_lba + 1;
            if sectors == 0 {
                continue;
            }

            return Some(DataRegion {
                lba_start: first_lba,
                sectors,
            });
        }

        return None;
    }

    if has_mbr {
        // Keep conservative behavior for plain MBR disks until partition parser exists.
        return None;
    }

    Some(DataRegion {
        lba_start: 0,
        sectors: total_sectors,
    })
}

// spinner

static mut SPIN_ACTIVE: bool = false;
static mut SPIN_FRAME: usize = 0;
static mut SPIN_LAST_TSC: u64 = 0;

const SPIN_FRAMES: [u8; 4] = [b'|', b'/', b'-', b'\\'];

/// Start a spinner on both serial and the framebuffer. The initial frame
/// appears on the same line after the last log message.
fn spinner_start() {
    unsafe {
        SPIN_ACTIVE = true;
        SPIN_FRAME = 0;
        SPIN_LAST_TSC = morpheus_hwinit::tsc::read_tsc();
        // Serial: write the opening frame (no newline — we'll overwrite in place)
        morpheus_hwinit::serial::serial_puts("   ");
        morpheus_hwinit::serial::serial_putc(SPIN_FRAMES[0]);
        // Framebuffer: same
        morpheus_hwinit::serial::fb_puts("   ");
        morpheus_hwinit::serial::fb_putc(SPIN_FRAMES[0]);
    }
}

/// Advance the spinner frame if ~100 ms have passed.
/// Called at the top of every raw_read / raw_write so it fires naturally
/// during helix I/O without needing a separate timer or thread.
fn spinner_tick() {
    unsafe {
        if !SPIN_ACTIVE || STORAGE_TSC_FREQ == 0 {
            return;
        }
        let now = morpheus_hwinit::tsc::read_tsc();
        let interval = STORAGE_TSC_FREQ / 10; // 100 ms
        if now.wrapping_sub(SPIN_LAST_TSC) < interval {
            return;
        }
        SPIN_LAST_TSC = now;
        SPIN_FRAME = (SPIN_FRAME + 1) % SPIN_FRAMES.len();
        let frame = SPIN_FRAMES[SPIN_FRAME];
        // Serial: backspace over previous frame, write new one
        morpheus_hwinit::serial::serial_putc(b'\x08');
        morpheus_hwinit::serial::serial_putc(frame);
        // Framebuffer: same
        morpheus_hwinit::serial::fb_putc(b'\x08');
        morpheus_hwinit::serial::fb_putc(frame);
    }
}

/// Stop the spinner. \r returns cursor to col 0 on both serial and framebuffer
/// so the next `puts()` call overwrites the spinner line with the result.
fn spinner_done() {
    unsafe {
        SPIN_ACTIVE = false;
    }
    morpheus_hwinit::serial::serial_putc(b'\r');
    morpheus_hwinit::serial::fb_puts("\r");
}

// PUBLIC API

/// Probe for a persistent block device and mount HelixFS on it.
///
/// If a device is found, replaces the in-memory root FS with the persistent
/// one.  If no device is found, logs a warning and leaves the RAM-disk
/// in place.
///
/// # Safety
/// - `dma` must be a valid 2 MB DMA region from hwinit Phase 6.
/// - `tsc_freq` must be the calibrated TSC frequency.
/// - Must be called exactly once, after hwinit completes.
pub unsafe fn init_persistent_storage(dma: &DmaRegion, tsc_freq: u64) {
    log_info("STORAGE", 822, "probing block devices");

    // Dump PCI bus to serial for device identification diagnostics
    // (Commenting out PCI dump — enable if debugging PCI device discovery)
    // dump_pci_devices();

    STORAGE_DMA = Some(*dma);
    STORAGE_TSC_FREQ = tsc_freq;

    // NOTE: The DMA region is fully zeroed by hwinit Phase 6 at allocation
    // time, so VirtIO queue structures (desc, avail, used, headers, status)
    // are already clean.  No additional zeroing needed here.

    // build blockdmaconfig from the dma region
    let base_cpu = dma.cpu_base();
    let base_bus = dma.bus_base();

    let config = BlockDmaConfig {
        tsc_freq,

        // VirtIO-blk
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
        virtio_notify_addr: 0, // Filled by driver from PCI caps
        queue_size: VIRTIO_QUEUE_SIZE,

        // AHCI
        ahci_cmd_list_cpu: base_cpu.add(OFF_AHCI_CMD_LIST),
        ahci_cmd_list_phys: base_bus + OFF_AHCI_CMD_LIST as u64,
        ahci_fis_cpu: base_cpu.add(OFF_AHCI_FIS),
        ahci_fis_phys: base_bus + OFF_AHCI_FIS as u64,
        ahci_cmd_tables_cpu: base_cpu.add(OFF_AHCI_CMD_TABLES),
        ahci_cmd_tables_phys: base_bus + OFF_AHCI_CMD_TABLES as u64,
        ahci_identify_cpu: base_cpu.add(OFF_AHCI_IDENTIFY),
        ahci_identify_phys: base_bus + OFF_AHCI_IDENTIFY as u64,
    };

    // scan all block devices on pci bus
    let (devices, dev_count) = scan_all_block_devices();

    if dev_count == 0 {
        log_warn("STORAGE", 823, "no block device found; using RAM-disk");
        return;
    }
    let _ = dev_count;
    log_info("STORAGE", 824, "block devices detected; selecting data disk");

    // Try each device: skip boot disks (GPT/MBR), use the first blank or HelixFS one.
    let mut found_data_disk = false;
    'device_scan: for i in 0..dev_count {
        let detected = match &devices[i] {
            Some(d) => d,
            None => continue,
        };

        // Map high-address MMIO BARs before driver init touches them
        if let DetectedBlockDevice::VirtIO { pci_addr, .. } = detected {
            map_virtio_bars(pci_addr.bus, pci_addr.device, pci_addr.function);
        }

        let is_ahci = matches!(detected, DetectedBlockDevice::Ahci(_));
        let mut ahci_port_attempted = false;
        let mut ahci_port_found = false;

        let max_ports = if is_ahci { 32 } else { 1 };

        for port in 0..max_ports {
            let device = if is_ahci {
                ahci_port_attempted = true;
                match create_unified_from_detected_ahci_port(detected, &config, port as u32) {
                    Ok(dev) => dev,
                    Err(_) => continue,
                }
            } else {
                match create_unified_from_detected(detected, &config) {
                    Ok(dev) => dev,
                    Err(_) => {
                        log_warn("STORAGE", 825, "driver init failed for one candidate; skipping");
                        break;
                    }
                }
            };

            if is_ahci {
                ahci_port_found = true;
            }

            let info = device.info();

            // Store temporarily to check if it's a boot disk.
            BLOCK_DEVICE = Some(device);

            let region = match select_data_region(info.sector_size, info.total_sectors) {
                Some(r) => r,
                None => {
                    log_info("STORAGE", 826, "boot/system disk detected; skipping");
                    BLOCK_DEVICE = None;
                    continue;
                }
            };

            STORAGE_LBA_BASE = region.lba_start;
            STORAGE_REGION_SECTORS = region.sectors;

            if region.lba_start != 0 {
                log_info("STORAGE", 837, "selected GPT data partition region");
            }

            if region.sectors == 0 {
                BLOCK_DEVICE = None;
                continue;
            }

            // This device is NOT a boot disk — use it for HelixFS.
            log_ok("STORAGE", 827, "selected data disk");
            found_data_disk = true;

            // try to recover or format helixfs
            let mut raw_dev = make_raw_block_device();

            let needs_format = {
                let mut probe_dev = make_raw_block_device();
                match morpheus_helix::log::recovery::recover_superblock(
                    &mut probe_dev,
                    0,
                    info.sector_size,
                ) {
                    Ok(sb) => {
                        // Version mismatch: v2 changed the log payload format.
                        if sb.version != morpheus_helix::types::HELIX_VERSION {
                            log_warn("STORAGE", 828, "helixfs version mismatch; reformat required");
                            true
                        } else {
                            false
                        }
                    }
                    Err(_) => true,
                }
            };

            if needs_format {
                log_info("STORAGE", 829, "no valid helixfs; formatting disk");

                let uuid = [
                    0x4Du8, 0x58, 0x52, 0x4F, 0x4F, 0x54, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x01,
                ];
                spinner_start();
                match morpheus_helix::format::format_helix(
                    &mut raw_dev,
                    0,
                    info.total_sectors,
                    info.sector_size,
                    "root",
                    uuid,
                ) {
                    Ok(_sb) => {
                        spinner_done();
                        log_ok("STORAGE", 830, "format completed");
                    }
                    Err(e) => {
                        spinner_done();
                        let _ = e;
                        log_error("STORAGE", 831, "format failed");
                        BLOCK_DEVICE = None;
                        continue;
                    }
                }

                // Verify format succeeded
                match morpheus_helix::log::recovery::recover_superblock(
                    &mut raw_dev,
                    0,
                    info.sector_size,
                ) {
                    Ok(_sb) => {}
                    Err(e) => {
                        let _ = e;
                        log_warn("STORAGE", 832, "superblock readback failed after format");
                    }
                }
            } else {
                log_info("STORAGE", 833, "valid helixfs found; mounting");
            }

            // Now do the actual replace_root_device
            // If we already formatted above, pass do_format=false to avoid double-format
            let mount_dev = make_raw_block_device();
            spinner_start();
            match morpheus_helix::vfs::global::replace_root_device(mount_dev, false) {
                Ok(()) => {
                    spinner_done();
                    PERSISTENT_READY = true;
                    log_ok("STORAGE", 834, "persistent root filesystem mounted at /");
                    break 'device_scan;
                }
                Err(e) => {
                    spinner_done();
                    let _ = e;
                    log_error("STORAGE", 835, "failed to mount persistent filesystem");
                    BLOCK_DEVICE = None;
                    continue;
                }
            }
        }

        if is_ahci {
            if ahci_port_attempted && !ahci_port_found {
                log_warn("STORAGE", 825, "driver init failed for one candidate; skipping");
            }
            continue;
        }
    }

    if !found_data_disk {
        log_warn("STORAGE", 836, "no suitable data disk; using RAM-disk fallback");
    }
}

/// Whether persistent storage is active (vs RAM-disk fallback).
pub fn is_persistent() -> bool {
    unsafe { PERSISTENT_READY }
}

// BOOT DISK DETECTION

/// Check if the currently probed block device is a boot disk (GPT or MBR).
///
/// We NEVER format a disk that has an existing partition table — it's
/// almost certainly the system ESP or a user disk with data.
///
/// Checks:
/// 1. LBA 0 bytes [510..512] == 0xAA55 (MBR signature)
/// 2. LBA 1 bytes [0..8] == "EFI PART" (GPT header magic)
///
/// Returns `true` if partition table detected → disk should be skipped.
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

    // Read first 2 sectors (need LBA 0 for MBR, LBA 1 for GPT).
    let read_size = (sector_size as usize) * 2;
    let mut buf = alloc::vec![0u8; read_size];

    use morpheus_network::GptBlockIo;
    use morpheus_network::GptLba;
    if bio.read_blocks(GptLba(0), &mut buf).is_err() {
        return false;
    }

    // Check MBR signature at offset 510-511
    if buf.len() >= 512 && buf[510] == 0x55 && buf[511] == 0xAA {
        return true;
    }

    // Check GPT header at LBA 1
    let gpt_offset = sector_size as usize;
    if buf.len() >= gpt_offset + 8 && &buf[gpt_offset..gpt_offset + 8] == b"EFI PART" {
        return true;
    }

    false
}

/// Create the standard initFS directory structure.
///
/// Idempotent — silently ignores directories that already exist.
/// Called after the root FS is mounted (whether RAM-disk or persistent).
pub fn create_init_directories() {
    use morpheus_hwinit::cpu::tsc::read_tsc;

    let dirs = ["/bin", "/etc", "/tmp", "/home", "/var", "/dev"];

    let ts = read_tsc();

    let fs = match unsafe { morpheus_helix::vfs::global::fs_global_mut() } {
        Some(f) => f,
        None => {
            log_warn("INITFS", 840, "no root fs; skipping directory bootstrap");
            return;
        }
    };

    for dir in &dirs {
        match morpheus_helix::vfs::vfs_mkdir(&mut fs.mount_table, dir, ts) {
            Ok(_) => {}
            Err(morpheus_helix::error::HelixError::AlreadyExists) => {}
            Err(_) => {
                let _ = dir;
                log_warn("INITFS", 841, "failed to create one startup directory");
            }
        }
    }

    log_ok("INITFS", 842, "directory structure ready");
}

// RAW BLOCK DEVICE BRIDGE
//
// We create a `RawBlockDevice` whose function pointers call through
// the network crate's `UnifiedBlockIo` adapter for each operation.
// This reuses 100% of the existing synchronous DMA read/write logic
// (chunked transfers, timeout, completion polling) without duplication.

/// Build a `RawBlockDevice` backed by the static `BLOCK_DEVICE`.
///
/// # Safety
/// `BLOCK_DEVICE` must be `Some` before calling this.
unsafe fn make_raw_block_device() -> RawBlockDevice {
    let dev = BLOCK_DEVICE.as_ref().unwrap();
    let info = dev.info();
    let total = if STORAGE_REGION_SECTORS != 0 {
        STORAGE_REGION_SECTORS
    } else {
        info.total_sectors
    };

    RawBlockDevice::new(
        core::ptr::null_mut(), // ctx unused — we access statics directly
        total,
        info.sector_size,
        raw_read,
        raw_write,
        raw_flush,
    )
}

/// Read callback for `RawBlockDevice`.
///
/// Creates a temporary `UnifiedBlockIo` from the static device + DMA
/// region, then delegates to the existing chunked read_blocks() impl.
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
    let timeout = STORAGE_TSC_FREQ * 5; // 5 second timeout

    let mut bio = match UnifiedBlockIo::new(dev, io_buf, io_phys, timeout) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let dst_slice = core::slice::from_raw_parts_mut(dst, len);

    use morpheus_network::GptBlockIo;
    use morpheus_network::GptLba;
    bio.read_blocks(GptLba(lba + STORAGE_LBA_BASE), dst_slice)
        .is_ok()
}

/// Write callback for `RawBlockDevice`.
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

    use morpheus_network::GptBlockIo;
    use morpheus_network::GptLba;
    bio.write_blocks(GptLba(lba + STORAGE_LBA_BASE), src_slice)
        .is_ok()
}

/// Flush callback for `RawBlockDevice`.
unsafe fn raw_flush(_ctx: *mut u8) -> bool {
    let dev = match BLOCK_DEVICE.as_mut() {
        Some(d) => d,
        None => return false,
    };

    dev.flush().is_ok()
}
