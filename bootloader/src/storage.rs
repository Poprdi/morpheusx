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
//! ──────────────────────────────────────────────────
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
//! ──────────────────────────────────────────────────
//! Total      ≈ 128 KB (well within 2 MB)
//! ```

use morpheus_helix::device::RawBlockDevice;
use morpheus_hwinit::dma::DmaRegion;
use morpheus_hwinit::paging::is_paging_initialized;
use morpheus_hwinit::serial::puts;
use morpheus_hwinit::{kmap_mmio, pci_cfg_read16, pci_cfg_read32, PciAddr};
use morpheus_network::{
    create_unified_from_detected, scan_all_block_devices, BlockDmaConfig, BlockDriver,
    DetectedBlockDevice, UnifiedBlockDevice, UnifiedBlockIo,
};

// ═══════════════════════════════════════════════════════════════════════════
// DMA LAYOUT CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

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

// ═══════════════════════════════════════════════════════════════════════════
// PCI BUS DUMP (diagnostic)
// ═══════════════════════════════════════════════════════════════════════════

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

// ═══════════════════════════════════════════════════════════════════════════
// MMIO BAR MAPPING
// ═══════════════════════════════════════════════════════════════════════════

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
        puts("[STORAGE] WARNING: paging not initialized, cannot map BARs\n");
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
            puts("[STORAGE] BAR");
            morpheus_hwinit::serial::put_hex32(bar_idx as u32);
            puts(" @ ");
            morpheus_hwinit::serial::put_hex64(base_addr);
            puts(" (type=");
            morpheus_hwinit::serial::put_hex32(bar_type);
            puts(") mapping...\n");

            match kmap_mmio(base_addr, MAP_SIZE) {
                Ok(()) => puts("[STORAGE]   mapped UC OK\n"),
                Err(e) => {
                    puts("[STORAGE]   map_mmio FAILED: ");
                    puts(e);
                    puts("\n");
                }
            }
        }

        bar_idx += if is_64bit { 2 } else { 1 };
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// STATIC STATE
// ═══════════════════════════════════════════════════════════════════════════

/// The unified block device lives here for the kernel's lifetime.
static mut BLOCK_DEVICE: Option<UnifiedBlockDevice> = None;

/// DMA region reference (Copy — stored from init).
static mut STORAGE_DMA: Option<DmaRegion> = None;

/// TSC frequency for timeout computation.
static mut STORAGE_TSC_FREQ: u64 = 0;

/// Whether persistent storage was successfully initialized.
static mut PERSISTENT_READY: bool = false;

// ═══════════════════════════════════════════════════════════════════════════
// PUBLIC API
// ═══════════════════════════════════════════════════════════════════════════

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
    puts("[STORAGE] probing for block device\n");

    // Dump PCI bus to serial for device identification diagnostics
    dump_pci_devices();

    STORAGE_DMA = Some(*dma);
    STORAGE_TSC_FREQ = tsc_freq;

    // NOTE: The DMA region is fully zeroed by hwinit Phase 6 at allocation
    // time, so VirtIO queue structures (desc, avail, used, headers, status)
    // are already clean.  No additional zeroing needed here.

    // ── Build BlockDmaConfig from the DMA region ────────────────────────
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

    // ── Scan all block devices on PCI bus ─────────────────────────────
    let (devices, dev_count) = scan_all_block_devices();

    if dev_count == 0 {
        puts("[STORAGE] no block device found — using RAM-disk\n");
        return;
    }

    puts("[STORAGE] found ");
    morpheus_hwinit::serial::put_hex32(dev_count as u32);
    puts(" block device(s), scanning for data disk...\n");

    // Try each device: skip boot disks (GPT/MBR), use the first blank or HelixFS one.
    let mut found_data_disk = false;
    #[allow(clippy::needless_range_loop)]
    for i in 0..dev_count {
        let detected = match &devices[i] {
            Some(d) => d,
            None => continue,
        };

        puts("[STORAGE] trying device ");
        morpheus_hwinit::serial::put_hex32(i as u32);
        puts("...\n");

        // Map high-address MMIO BARs before driver init touches them
        if let DetectedBlockDevice::VirtIO { pci_addr, .. } = detected {
            // Read CMD before driver enable for diagnostics
            let haddr = PciAddr::new(pci_addr.bus, pci_addr.device, pci_addr.function);
            let cmd_before = pci_cfg_read16(haddr, 0x04);
            puts("[STORAGE]   CMD before enable: ");
            morpheus_hwinit::serial::put_hex32(cmd_before as u32);
            puts(" MEM=");
            puts(if cmd_before & 0x02 != 0 { "Y" } else { "N" });
            puts(" BM=");
            puts(if cmd_before & 0x04 != 0 { "Y" } else { "N" });
            puts("\n");

            map_virtio_bars(pci_addr.bus, pci_addr.device, pci_addr.function);
        }

        let device = match create_unified_from_detected(detected, &config) {
            Ok(dev) => dev,
            Err(_) => {
                puts("[STORAGE]   driver init failed, skipping\n");
                continue;
            }
        };

        let info = device.info();
        puts("[STORAGE]   ");
        puts(device.driver_type());
        puts(": ");
        morpheus_hwinit::serial::put_hex64(info.total_sectors);
        puts(" sectors × ");
        morpheus_hwinit::serial::put_hex32(info.sector_size);
        puts(" B = ");
        let mb = (info.total_sectors * info.sector_size as u64) / (1024 * 1024);
        morpheus_hwinit::serial::put_hex64(mb);
        puts(" MB\n");

        // Store temporarily to check if it's a boot disk.
        BLOCK_DEVICE = Some(device);

        if is_boot_disk(info.sector_size) {
            puts("[STORAGE]   has GPT/MBR partition table — skipping (boot disk)\n");
            BLOCK_DEVICE = None;
            continue;
        }

        // This device is NOT a boot disk — use it for HelixFS.
        puts("[STORAGE]   selected as data disk\n");
        found_data_disk = true;

        // ── Try to recover or format HelixFS ──────────────────────────
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
                        puts("[STORAGE] HelixFS version mismatch (v");
                        morpheus_hwinit::serial::put_hex32(sb.version);
                        puts(" != v");
                        morpheus_hwinit::serial::put_hex32(morpheus_helix::types::HELIX_VERSION);
                        puts(") — reformat needed\n");
                        true
                    } else {
                        false
                    }
                }
                Err(_) => true,
            }
        };

        if needs_format {
            puts("[STORAGE] no valid HelixFS — formatting\n");

            // Test write + readback before formatting using raw callbacks
            {
                let mut wbuf = [0u8; 512];
                wbuf[..8].copy_from_slice(b"MXTEST01");
                let write_ok = raw_write(core::ptr::null_mut(), 0, wbuf.as_ptr(), 512);
                puts("[STORAGE]   write test LBA0: ");
                puts(if write_ok { "OK" } else { "FAIL" });
                puts("\n");

                if write_ok {
                    let mut rbuf = [0u8; 512];
                    let read_ok = raw_read(core::ptr::null_mut(), 0, rbuf.as_mut_ptr(), 512);
                    puts("[STORAGE]   read test LBA0: ");
                    puts(if read_ok { "OK" } else { "FAIL" });
                    if read_ok {
                        puts(" first8=");
                        for byte in rbuf.iter().take(8) {
                            morpheus_hwinit::serial::put_hex8(*byte);
                            puts(" ");
                        }
                    }
                    puts("\n");
                }
            }

            let uuid = [
                0x4Du8, 0x58, 0x52, 0x4F, 0x4F, 0x54, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x01,
            ];
            match morpheus_helix::format::format_helix(
                &mut raw_dev,
                0,
                info.total_sectors,
                info.sector_size,
                "root",
                uuid,
            ) {
                Ok(_sb) => {
                    puts("[STORAGE] format_helix OK\n");
                }
                Err(e) => {
                    puts("[STORAGE] format_helix FAILED: ");
                    match e {
                        morpheus_helix::error::HelixError::IoWriteFailed => puts("IoWriteFailed"),
                        morpheus_helix::error::HelixError::IoFlushFailed => puts("IoFlushFailed"),
                        morpheus_helix::error::HelixError::FormatTooSmall => puts("FormatTooSmall"),
                        _ => puts("(other)"),
                    }
                    puts("\n");
                    BLOCK_DEVICE = None;
                    break;
                }
            }

            // Re-read superblock after format to verify
            match morpheus_helix::log::recovery::recover_superblock(
                &mut raw_dev,
                0,
                info.sector_size,
            ) {
                Ok(_sb) => puts("[STORAGE] superblock readback OK\n"),
                Err(e) => {
                    puts("[STORAGE] superblock readback FAILED: ");
                    match e {
                        morpheus_helix::error::HelixError::NoValidSuperblock => {
                            puts("NoValidSuperblock")
                        }
                        morpheus_helix::error::HelixError::IoReadFailed => puts("IoReadFailed"),
                        _ => puts("(other)"),
                    }
                    puts("\n");
                }
            }
        } else {
            puts("[STORAGE] valid HelixFS found — mounting\n");
        }

        // Now do the actual replace_root_device
        // If we already formatted above, pass do_format=false to avoid double-format
        let mount_dev = make_raw_block_device();
        match morpheus_helix::vfs::global::replace_root_device(mount_dev, false) {
            Ok(()) => {
                PERSISTENT_READY = true;
                puts("[STORAGE] persistent root FS mounted at /\n");
            }
            Err(e) => {
                puts("[STORAGE] ERROR: failed to mount persistent FS: ");
                match e {
                    morpheus_helix::error::HelixError::IoReadFailed => puts("IoReadFailed"),
                    morpheus_helix::error::HelixError::IoWriteFailed => puts("IoWriteFailed"),
                    morpheus_helix::error::HelixError::IoFlushFailed => puts("IoFlushFailed"),
                    morpheus_helix::error::HelixError::NoValidSuperblock => {
                        puts("NoValidSuperblock")
                    }
                    morpheus_helix::error::HelixError::IncompatibleVersion => {
                        puts("IncompatibleVersion")
                    }
                    morpheus_helix::error::HelixError::FormatTooSmall => puts("FormatTooSmall"),
                    morpheus_helix::error::HelixError::InvalidBlockSize => puts("InvalidBlockSize"),
                    morpheus_helix::error::HelixError::LogFull => puts("LogFull"),
                    morpheus_helix::error::HelixError::LogCrcMismatch => puts("LogCrcMismatch"),
                    morpheus_helix::error::HelixError::LogSegmentCorrupt => {
                        puts("LogSegmentCorrupt")
                    }
                    _ => puts("(other)"),
                }
                puts("\n");
                BLOCK_DEVICE = None;
            }
        }
        break;
    }

    if !found_data_disk {
        puts("[STORAGE] no suitable data disk found — using RAM-disk\n");
        puts("[STORAGE] hint: add a blank virtio-blk disk for persistent storage\n");
    }
}

/// Whether persistent storage is active (vs RAM-disk fallback).
pub fn is_persistent() -> bool {
    unsafe { PERSISTENT_READY }
}

// ═══════════════════════════════════════════════════════════════════════════
// BOOT DISK DETECTION
// ═══════════════════════════════════════════════════════════════════════════

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
        puts("[STORAGE] detected MBR signature at LBA 0\n");
        return true;
    }

    // Check GPT header at LBA 1
    let gpt_offset = sector_size as usize;
    if buf.len() >= gpt_offset + 8 && &buf[gpt_offset..gpt_offset + 8] == b"EFI PART" {
        puts("[STORAGE] detected GPT header at LBA 1\n");
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
            puts("[INITFS] WARNING: no root FS — skipping directory creation\n");
            return;
        }
    };

    for dir in &dirs {
        match morpheus_helix::vfs::vfs_mkdir(&mut fs.mount_table, dir, ts) {
            Ok(_) => {}
            Err(morpheus_helix::error::HelixError::AlreadyExists) => {}
            Err(_) => {
                puts("[INITFS] WARNING: failed to create ");
                puts(dir);
                puts("\n");
            }
        }
    }

    puts("[INITFS] directory structure ready\n");
}

// ═══════════════════════════════════════════════════════════════════════════
// RAW BLOCK DEVICE BRIDGE
// ═══════════════════════════════════════════════════════════════════════════
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

    RawBlockDevice::new(
        core::ptr::null_mut(), // ctx unused — we access statics directly
        info.total_sectors,
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
    bio.read_blocks(GptLba(lba), dst_slice).is_ok()
}

/// Write callback for `RawBlockDevice`.
unsafe fn raw_write(_ctx: *mut u8, lba: u64, src: *const u8, len: usize) -> bool {
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
    bio.write_blocks(GptLba(lba), src_slice).is_ok()
}

/// Flush callback for `RawBlockDevice`.
unsafe fn raw_flush(_ctx: *mut u8) -> bool {
    let dev = match BLOCK_DEVICE.as_mut() {
        Some(d) => d,
        None => return false,
    };

    dev.flush().is_ok()
}
