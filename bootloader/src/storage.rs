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
use morpheus_hwinit::serial::puts;
use morpheus_network::{
    BlockDmaConfig, BlockDriver, UnifiedBlockDevice, UnifiedBlockIo,
    create_unified_from_detected, scan_all_block_devices,
};

// ═══════════════════════════════════════════════════════════════════════════
// DMA LAYOUT CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

const VIRTIO_QUEUE_SIZE: u16 = 32;

// VirtIO virtqueue structures
const OFF_VIRTIO_DESC:    usize = 0x0_0000;
const OFF_VIRTIO_AVAIL:   usize = 0x0_0200;
const OFF_VIRTIO_USED:    usize = 0x0_0400;
const OFF_VIRTIO_HEADERS: usize = 0x0_1000; // page-aligned
const OFF_VIRTIO_STATUS:  usize = 0x0_1200;

// AHCI structures
const OFF_AHCI_CMD_LIST:  usize = 0x0_2000; // 1 KB-aligned
const OFF_AHCI_FIS:       usize = 0x0_2400; // 256-byte aligned
const OFF_AHCI_CMD_TABLES:usize = 0x0_2800; // 128-byte aligned
const OFF_AHCI_IDENTIFY:  usize = 0x0_4800;

// I/O transfer buffer — used by UnifiedBlockIo for synchronous read/write
const OFF_IO_BUFFER:      usize = 0x1_0000;
const IO_BUFFER_SIZE:     usize = 64 * 1024; // 64 KB = UnifiedBlockIo::MAX_TRANSFER_SIZE

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

    STORAGE_DMA = Some(*dma);
    STORAGE_TSC_FREQ = tsc_freq;

    // ── Build BlockDmaConfig from the DMA region ────────────────────────
    let base_cpu = dma.cpu_base();
    let base_bus = dma.bus_base();

    let config = BlockDmaConfig {
        tsc_freq,

        // VirtIO-blk
        virtio_desc_cpu:     base_cpu.add(OFF_VIRTIO_DESC),
        virtio_desc_phys:    base_bus + OFF_VIRTIO_DESC as u64,
        virtio_avail_cpu:    base_cpu.add(OFF_VIRTIO_AVAIL),
        virtio_avail_phys:   base_bus + OFF_VIRTIO_AVAIL as u64,
        virtio_used_cpu:     base_cpu.add(OFF_VIRTIO_USED),
        virtio_used_phys:    base_bus + OFF_VIRTIO_USED as u64,
        virtio_headers_cpu:  base_cpu.add(OFF_VIRTIO_HEADERS),
        virtio_headers_phys: base_bus + OFF_VIRTIO_HEADERS as u64,
        virtio_status_cpu:   base_cpu.add(OFF_VIRTIO_STATUS),
        virtio_status_phys:  base_bus + OFF_VIRTIO_STATUS as u64,
        virtio_notify_addr:  0, // Filled by driver from PCI caps
        queue_size:          VIRTIO_QUEUE_SIZE,

        // AHCI
        ahci_cmd_list_cpu:   base_cpu.add(OFF_AHCI_CMD_LIST),
        ahci_cmd_list_phys:  base_bus + OFF_AHCI_CMD_LIST as u64,
        ahci_fis_cpu:        base_cpu.add(OFF_AHCI_FIS),
        ahci_fis_phys:       base_bus + OFF_AHCI_FIS as u64,
        ahci_cmd_tables_cpu: base_cpu.add(OFF_AHCI_CMD_TABLES),
        ahci_cmd_tables_phys:base_bus + OFF_AHCI_CMD_TABLES as u64,
        ahci_identify_cpu:   base_cpu.add(OFF_AHCI_IDENTIFY),
        ahci_identify_phys:  base_bus + OFF_AHCI_IDENTIFY as u64,
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
    for i in 0..dev_count {
        let detected = match &devices[i] {
            Some(d) => d,
            None => continue,
        };

        puts("[STORAGE] trying device ");
        morpheus_hwinit::serial::put_hex32(i as u32);
        puts("...\n");

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
        let raw_dev = make_raw_block_device();

        let needs_format = {
            let mut probe_dev = make_raw_block_device();
            morpheus_helix::log::recovery::recover_superblock(
                &mut probe_dev, 0, info.sector_size,
            ).is_err()
        };

        if needs_format {
            puts("[STORAGE] no valid HelixFS — formatting\n");
        } else {
            puts("[STORAGE] valid HelixFS found — mounting\n");
        }

        match morpheus_helix::vfs::global::replace_root_device(raw_dev, needs_format) {
            Ok(()) => {
                PERSISTENT_READY = true;
                puts("[STORAGE] persistent root FS mounted at /\n");
            }
            Err(_) => {
                puts("[STORAGE] ERROR: failed to mount persistent FS — keeping RAM-disk\n");
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
    if buf.len() >= gpt_offset + 8 {
        if &buf[gpt_offset..gpt_offset + 8] == b"EFI PART" {
            puts("[STORAGE] detected GPT header at LBA 1\n");
            return true;
        }
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
        match morpheus_helix::vfs::vfs_mkdir(
            &mut fs.mount_table,
            dir,
            ts,
        ) {
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

    let mut dst_slice = core::slice::from_raw_parts_mut(dst, len);

    use morpheus_network::GptBlockIo;
    use morpheus_network::GptLba;
    bio.read_blocks(GptLba(lba), &mut dst_slice).is_ok()
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
