//! Boot-time storage bring-up: probe every block device, register them (drivers
//! kept ALIVE in these bootloader statics — same address space, so Direct mounts
//! work at runtime), enumerate volumes per device, and mount root through the
//! kernel storage subsystem's privileged `storage::mount(.., MNT_STAGED)` path
//! (spec §7 boot population). Replaces the old "probe one device, mount via
//! helix::vfs, drop the rest" policy.
//!
//! DMA layout within hwinit's 2 MB region:
//!   0x00000  VirtIO desc/avail/used/headers/status  (≤ 0x01400)
//!   0x02000  AHCI cmd_list/FIS/cmd_tables/IDENTIFY  (≤ 0x05000)
//!   0x10000  64 KB I/O buffer for UnifiedBlockIo

use morpheus_block::ahci::AhciInitError;
use morpheus_block::boot_probe::{
    create_unified_from_detected, scan_all_block_devices, BlockDmaConfig, DetectedBlockDevice,
};
use morpheus_block::device::{UnifiedBlockDevice, UnifiedBlockError};
use morpheus_block::gpt::{enumerate_partitions, PartitionEntry};
use morpheus_block::sdhci::SdhciInitError;
use morpheus_block::unified_block_io::UnifiedBlockIo;
use morpheus_block::usb_msd::UsbMsdInitError;
use morpheus_block::virtio_blk::VirtioBlkInitError;
use morpheus_block::{BlockDriver, DeviceKind, MemBlockDevice, RawBlockDevice};
use morpheus_foundation::storage::{FS_AUTO, FS_HELIX, MNT_STAGED};
use morpheus_hal_x86_64::dma::DmaRegion;
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

/// Fresh RAM-root size when no disk/pre-EBS root is found (matches the old
/// `init_root_fs` 16 MiB allocation; now routed through staging admission).
const RAM_ROOT_BYTES: u64 = 16 * 1024 * 1024;

/// Largest number of live block devices we keep alive across the boot→runtime
/// handoff. A DoS bound, not a capacity claim; matches the probe's scan width.
const MAX_LIVE_DEVICES: usize = 32;

/// A probed driver kept alive for the whole runtime. The `RawBlockDevice` handed
/// to the kernel registry bridges back here via `ctx == &LIVE[i]`; whole-disk
/// addressing (no LBA base) so the storage subsystem's per-volume backend owns
/// the partition offset. These statics live in the kernel address space forever.
struct LiveDevice {
    /// `None` until a driver is parked here; never cleared (lives for the runtime).
    dev: Option<UnifiedBlockDevice>,
    sector_size: u32,
    total_sectors: u64,
}

impl LiveDevice {
    const fn empty() -> Self {
        Self {
            dev: None,
            sector_size: 0,
            total_sectors: 0,
        }
    }
}

static mut LIVE: [LiveDevice; MAX_LIVE_DEVICES] = [const { LiveDevice::empty() }; MAX_LIVE_DEVICES];
static mut LIVE_COUNT: usize = 0;

/// Geometry of a Helix-detected volume, kept so the root mount can size its RAM
/// staging to the live FS footprint (superblock-driven) rather than the whole
/// partition — preserving today's footprint-only staging on multi-GB disks.
#[derive(Clone, Copy)]
struct HelixCandidate {
    volume_id: u64,
    slot: usize,
    lba_start: u64,
    sector_size: u32,
}

const MAX_HELIX_CANDIDATES: usize = 64;
static mut HELIX_CANDS: [Option<HelixCandidate>; MAX_HELIX_CANDIDATES] =
    [const { None }; MAX_HELIX_CANDIDATES];
static mut HELIX_CAND_COUNT: usize = 0;

static mut STORAGE_DMA: Option<DmaRegion> = None;
static mut STORAGE_TSC_FREQ: u64 = 0;
static mut PERSISTENT_READY: bool = false;

/// Backing for the pre-EBS staged Helix image, kept alive for the device it
/// backs (the kernel registry holds a `RawBlockDevice` whose ctx points here).
static mut RAM_HELIX_DEVICE: Option<MemBlockDevice> = None;

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

/// Map device-kind for the volume layer + a registry label.
fn detected_kind(d: &DetectedBlockDevice) -> DeviceKind {
    match d {
        DetectedBlockDevice::VirtIO { .. } => DeviceKind::Virtio,
        DetectedBlockDevice::Ahci(_) => DeviceKind::Ahci,
        DetectedBlockDevice::Sdhci(_) => DeviceKind::Sdhci,
        DetectedBlockDevice::UsbMsd(_) => DeviceKind::UsbMsd,
    }
}

/// Log a driver-init failure with its specific cause; returns true if the failure
/// was a scaffold-only backend (used to surface a louder error when that backend
/// would have been the only root candidate).
fn log_init_error(err: &UnifiedBlockError, is_ahci: bool) -> bool {
    let mut scaffold = false;
    match err {
        UnifiedBlockError::AhciError(e) => {
            let msg = match e {
                AhciInitError::InvalidConfig => "AHCI init failed: invalid config",
                AhciInitError::ResetFailed => "AHCI init failed: HBA reset timeout",
                AhciInitError::NoDeviceFound => "AHCI init failed: no SATA device found",
                AhciInitError::PortStopTimeout => "AHCI init failed: port stop timeout",
                AhciInitError::PortStartFailed => "AHCI init failed: port start failed",
                AhciInitError::IdentifyFailed => "AHCI init failed: IDENTIFY failed",
                AhciInitError::No64BitSupport => "AHCI init failed: no 64-bit DMA support",
                AhciInitError::DeviceNotResponding => "AHCI init failed: device not responding",
                AhciInitError::DmaSetupFailed => "AHCI init failed: DMA setup failed",
            };
            log_warn("STORAGE", 825, msg);
        },
        UnifiedBlockError::VirtioError(e) => {
            let msg = match e {
                VirtioBlkInitError::ResetFailed => "VirtIO init failed: reset failed",
                VirtioBlkInitError::FeatureNegotiationFailed => {
                    "VirtIO init failed: feature negotiation failed"
                },
                VirtioBlkInitError::QueueSetupFailed => "VirtIO init failed: queue setup failed",
                VirtioBlkInitError::DeviceFailed => "VirtIO init failed: device failed status",
                VirtioBlkInitError::InvalidConfig => "VirtIO init failed: invalid config",
                VirtioBlkInitError::TransportError => "VirtIO init failed: transport error",
            };
            log_warn("STORAGE", 825, msg);
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
            let msg = match e {
                SdhciInitError::InvalidConfig => "SDHCI init failed: invalid config",
                SdhciInitError::ControllerResetFailed => {
                    "SDHCI init failed: controller reset failed"
                },
                SdhciInitError::NoCardPresent => "SDHCI init failed: no card present",
                SdhciInitError::VoltageSwitchFailed => "SDHCI init failed: voltage switch failed",
                SdhciInitError::ClockSetupFailed => "SDHCI init failed: clock setup failed",
                SdhciInitError::CommandTimeout => "SDHCI init failed: command timeout",
                SdhciInitError::DataTimeout => "SDHCI init failed: data timeout",
                SdhciInitError::IoError => "SDHCI init failed: I/O error",
                SdhciInitError::NotImplemented => {
                    scaffold = true;
                    "SDHCI init failed: not implemented"
                },
            };
            log_warn("STORAGE", 825, msg);
        },
        UnifiedBlockError::UsbMsdError(e) => {
            let msg = match e {
                UsbMsdInitError::InvalidConfig => "USB-MSD init failed: invalid config",
                UsbMsdInitError::ControllerInitFailed => "USB-MSD init failed: controller init failed",
                UsbMsdInitError::ControllerProbeFailed => "USB-MSD init failed: controller probe failed",
                UsbMsdInitError::ControllerResetFailed => "USB-MSD init failed: controller reset failed",
                UsbMsdInitError::ControllerScratchpadUnsupported => {
                    "USB-MSD init failed: scratchpad requirement unsupported"
                },
                UsbMsdInitError::ControllerStartFailed => {
                    "USB-MSD init failed: controller start failed (HCH stuck)"
                },
                UsbMsdInitError::HubUnsupported => {
                    "USB-MSD init failed: USB hub detected; downstream hub traversal not implemented"
                },
                UsbMsdInitError::PortResetFailed => "USB-MSD init failed: port reset failed",
                UsbMsdInitError::PortResetTimeout => "USB-MSD init failed: port reset timeout",
                UsbMsdInitError::PortResetHotCmdTimeout => "USB-MSD init failed: hot reset command timeout",
                UsbMsdInitError::PortResetHotSettleTimeout => "USB-MSD init failed: hot reset settle timeout",
                UsbMsdInitError::PortResetWarmTimeout => "USB-MSD init failed: warm reset timeout",
                UsbMsdInitError::PortResetNoLink => "USB-MSD init failed: USB link not usable",
                UsbMsdInitError::EnableSlotFailed => "USB-MSD init failed: enable-slot command failed",
                UsbMsdInitError::AddressDeviceFailed => "USB-MSD init failed: address-device command failed",
                UsbMsdInitError::DeviceDescriptorFailed => "USB-MSD init failed: GET_DESCRIPTOR(device) failed",
                UsbMsdInitError::ConfigDescriptorFailed => "USB-MSD init failed: GET_DESCRIPTOR(config) failed",
                UsbMsdInitError::MassStorageProtocolUnsupported => {
                    "USB-MSD init failed: mass-storage protocol unsupported (need BOT)"
                },
                UsbMsdInitError::NoBotMassStorageInterface => {
                    "USB-MSD init failed: no BOT mass-storage interface found"
                },
                UsbMsdInitError::ActivePortsNoConnectedDevice => {
                    "USB-MSD init failed: root ports active but no connected device detected"
                },
                UsbMsdInitError::SetConfigurationFailed => "USB-MSD init failed: SET_CONFIGURATION failed",
                UsbMsdInitError::ConfigureEndpointsFailed => "USB-MSD init failed: configure endpoint command failed",
                UsbMsdInitError::DeviceEnumerationFailed => "USB-MSD init failed: device enumeration failed",
                UsbMsdInitError::TransportInitFailed => "USB-MSD init failed: transport init failed",
                UsbMsdInitError::NoMedia => "USB-MSD init failed: no media",
                UsbMsdInitError::CommandTimeout => "USB-MSD init failed: command timeout",
                UsbMsdInitError::IoError => "USB-MSD init failed: I/O error",
                UsbMsdInitError::NotImplemented => {
                    scaffold = true;
                    "USB-MSD init failed: not implemented"
                },
            };
            log_warn("STORAGE", 825, msg);
        },
    }
    scaffold
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
    STORAGE_DMA = Some(*dma);
    STORAGE_TSC_FREQ = tsc_freq;

    // Fast path: a Helix image staged in RAM before ExitBootServices. Register it
    // as a RAM device + volume and Direct-mount in place (already resident; no
    // re-copy). Only commit it as root if it actually carries /bin/init.
    if let Some(img) = pre_ebs_helix {
        if try_mount_pre_ebs_root(img) {
            PERSISTENT_READY = true;
            return;
        }
        // A pre-EBS mount that lacked /bin/init may still be mounted at /; tear it
        // down so the device-probe fallback can claim / without EEXIST.
        morpheus_kernel::storage::unmount_root_privileged();
        log_warn(
            "STORAGE",
            827,
            "pre-EBS staged root unusable; falling back to device probe",
        );
    }

    // dump_pci_devices(); // enable when debugging device discovery

    if probe_and_register_devices(dma, tsc_freq) {
        PERSISTENT_READY = true;
        return;
    }

    // Last resort: a fresh empty RAM helix at / (generalizes the old RAM-disk
    // root), allocated through the staging admission control as a privileged mount.
    if mount_fresh_ram_root() {
        PERSISTENT_READY = true;
        log_warn("STORAGE", 836, "no data disk; mounted fresh RAM root");
    } else {
        log_error("STORAGE", 835, "failed to mount any root filesystem");
    }
}

/// Register the pre-EBS RAM image and Direct-mount it at `/`. Returns true iff the
/// mount succeeded and `/bin/init` is present.
unsafe fn try_mount_pre_ebs_root(img: crate::boot::PreEbsHelixImage) -> bool {
    RAM_HELIX_DEVICE = Some(MemBlockDevice::new(
        img.base as *mut u8,
        img.size,
        img.sector_size,
    ));
    let mem = match RAM_HELIX_DEVICE.as_mut() {
        Some(m) => m,
        None => return false,
    };
    let raw = MemBlockDevice::into_raw(mem);
    let total_sectors = img.size as u64 / img.sector_size as u64;

    let device_id = match morpheus_kernel::storage::register_boot_device(
        raw,
        DeviceKind::Ram,
        img.sector_size,
        total_sectors,
    ) {
        Some(id) => id,
        None => return false,
    };

    let mut label = [0u8; 64];
    let l = b"pre-ebs-root";
    label[..l.len()].copy_from_slice(l);
    let volume_id = match morpheus_kernel::storage::register_volume(
        device_id,
        0,
        total_sectors,
        img.sector_size,
        [0u8; 16],
        FS_HELIX,
        label,
        false,
        false,
    ) {
        Some(id) => id,
        None => return false,
    };

    // Direct mount: the image is already in RAM, drive it in place (not staged).
    spinner_start();
    let ok = mount_root_volume(volume_id, 0, 0);
    spinner_done();
    if !ok {
        return false;
    }

    if morpheus_kernel::storage::path_exists("/bin/init") {
        true
    } else {
        log_warn("STORAGE", 844, "pre-EBS root missing /bin/init");
        false
    }
}

/// Probe every block device, register each (driver kept alive) + its volumes, then
/// mount the first Helix volume that yields `/bin/init` at `/` (staged). Returns
/// true once a root carrying `/bin/init` is mounted.
unsafe fn probe_and_register_devices(dma: &DmaRegion, tsc_freq: u64) -> bool {
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
        log_warn("STORAGE", 823, "no block device found");
        return false;
    }

    let mut saw_scaffold = false;

    #[allow(clippy::needless_range_loop)]
    for i in 0..dev_count {
        let detected = match &devices[i] {
            Some(d) => d,
            None => continue,
        };

        // Map BARs UC before the driver touches them.
        match detected {
            DetectedBlockDevice::VirtIO { pci_addr, .. } => {
                map_virtio_bars(pci_addr.bus, pci_addr.device, pci_addr.function);
            },
            DetectedBlockDevice::Ahci(info) => {
                if is_paging_initialized() {
                    // ABAR: HBA regs + 32 ports × 0x80 = 0x1100, round to 2 pages.
                    let _ = kmap_mmio(info.abar, 0x2000);
                }
            },
            DetectedBlockDevice::Sdhci(info) => {
                if is_paging_initialized() {
                    let _ = kmap_mmio(info.mmio_base, 0x1000);
                }
            },
            DetectedBlockDevice::UsbMsd(info) => {
                if is_paging_initialized() {
                    let _ = kmap_mmio(info.mmio_base, 0x4000);
                }
            },
        }

        let kind = detected_kind(detected);
        let is_ahci = matches!(detected, DetectedBlockDevice::Ahci(_));

        // Mask BSP interrupts across driver init: 100 Hz LAPIC timer ISRs
        // mid-PCH-MMIO extend bus cycles on real Intel silicon (same root cause
        // as the AHCI BIOS/OS handoff stall). Init polls on TSC.
        morpheus_hal_x86_64::cpu::idt::disable_interrupts();
        let device_result = create_unified_from_detected(detected, &config);
        morpheus_hal_x86_64::cpu::idt::enable_interrupts();

        let device = match device_result {
            Ok(dev) => dev,
            Err(err) => {
                if log_init_error(&err, is_ahci) {
                    saw_scaffold = true;
                }
                continue;
            },
        };

        // Park the live driver in a permanent slot; its RawBlockDevice ctx points
        // back here, so it must never move or drop for the runtime's lifetime.
        let slot = match park_live_device(device) {
            Some(s) => s,
            None => {
                log_warn("STORAGE", 837, "live-device table full; skipping disk");
                continue;
            },
        };

        let _ = register_device_and_volumes(slot, kind);
    }

    // All devices + volumes are now registered. Mount the first Helix volume that
    // yields /bin/init at / (spec §7 /bin/init selection policy).
    let root_mounted = mount_helix_root();

    if !root_mounted && saw_scaffold {
        log_error(
            "STORAGE",
            852,
            "boot medium backend is scaffold-only (SDHCI/USB-MSD not implemented); /bin/init unavailable",
        );
    }

    root_mounted
}

/// Move `device` into a permanent `LIVE` slot; returns its index or `None` if the
/// table is full.
unsafe fn park_live_device(device: UnifiedBlockDevice) -> Option<usize> {
    let count = LIVE_COUNT;
    if count >= MAX_LIVE_DEVICES {
        return None;
    }
    let info = device.info();
    let slot = &mut LIVE[count];
    slot.sector_size = info.sector_size;
    slot.total_sectors = info.total_sectors;
    slot.dev = Some(device);
    LIVE_COUNT = count + 1;
    Some(count)
}

/// Register slot `i` as a whole-disk device in the kernel registry and enumerate
/// its partitions into volumes (sniffing each volume's FS). Returns true on a
/// successful device registration.
unsafe fn register_device_and_volumes(slot: usize, kind: DeviceKind) -> bool {
    let (sector_size, total_sectors) = {
        let s = &LIVE[slot];
        (s.sector_size, s.total_sectors)
    };
    if sector_size == 0 {
        return false;
    }

    let raw = make_raw_block_device(slot, total_sectors, sector_size);
    let device_id =
        match morpheus_kernel::storage::register_boot_device(raw, kind, sector_size, total_sectors)
        {
            Some(id) => id,
            None => return false,
        };

    // Enumerate partitions over a fresh whole-disk handle.
    let mut probe = make_raw_block_device(slot, total_sectors, sector_size);
    let parts = enumerate_partitions(&mut probe);

    if parts.is_empty() {
        // Unpartitioned/whole-disk: one volume spanning the device.
        register_one_volume(
            device_id,
            slot,
            sector_size,
            0,
            total_sectors,
            &[0u8; 16],
            &[],
        );
    } else {
        for p in parts.iter() {
            register_partition_volume(device_id, slot, sector_size, p);
        }
    }
    true
}

unsafe fn register_partition_volume(
    device_id: u64,
    slot: usize,
    sector_size: u32,
    p: &PartitionEntry,
) {
    register_one_volume(
        device_id,
        slot,
        sector_size,
        p.lba_start,
        p.lba_count,
        &p.type_guid,
        &p.name,
    );
}

/// Sniff the FS at `lba_start` and register the volume. `name` is the raw on-disk
/// label (truncated to fit). Best-effort: a failed registration just drops the
/// volume (the device stays registered).
unsafe fn register_one_volume(
    device_id: u64,
    slot: usize,
    sector_size: u32,
    lba_start: u64,
    lba_count: u64,
    type_guid: &[u8; 16],
    name: &[u8],
) {
    let mut probe = make_raw_block_device(slot, LIVE[slot].total_sectors, sector_size);
    let detected = morpheus_kernel::storage::detect_fs(&mut probe, lba_start);

    let mut label = [0u8; 64];
    let n = name.len().min(label.len());
    label[..n].copy_from_slice(&name[..n]);

    let volume_id = morpheus_kernel::storage::register_volume(
        device_id,
        lba_start,
        lba_count,
        sector_size,
        *type_guid,
        detected,
        label,
        false,
        false,
    );

    if detected == FS_HELIX {
        if let Some(id) = volume_id {
            record_helix_candidate(HelixCandidate {
                volume_id: id,
                slot,
                lba_start,
                sector_size,
            });
        }
    }
}

unsafe fn record_helix_candidate(c: HelixCandidate) {
    let n = HELIX_CAND_COUNT;
    if n >= MAX_HELIX_CANDIDATES {
        return;
    }
    HELIX_CANDS[n] = Some(c);
    HELIX_CAND_COUNT = n + 1;
}

/// Mount the first Helix-detected volume at `/` (staged, footprint-capped) and
/// commit it only if `/bin/init` is present. Returns true once root is committed.
unsafe fn mount_helix_root() -> bool {
    let count = HELIX_CAND_COUNT;
    for i in 0..count {
        let c = match HELIX_CANDS[i] {
            Some(c) => c,
            None => continue,
        };
        // Cap staging to the live FS footprint (superblock-driven), not the whole
        // partition; aux=0 (full source) would copy multi-GB disks into RAM.
        let aux = helix_footprint_bytes(&c);

        spinner_start();
        let ok = mount_root_volume(c.volume_id, MNT_STAGED, aux);
        spinner_done();
        if !ok {
            continue;
        }
        if morpheus_kernel::storage::path_exists("/bin/init") {
            log_root_program_checks();
            return true;
        }
        // Wrong root: tear it down (boot-only, no fds open yet), free its staged
        // RAM, and keep scanning. umount("/") is EBUSY by design, so use the
        // privileged boot teardown.
        morpheus_kernel::storage::unmount_root_privileged();
        log_warn("STORAGE", 851, "candidate root rejected: /bin/init missing");
    }
    false
}

/// Live Helix footprint in bytes (log tail + used data), capped to the partition,
/// for use as the staged-mount `aux` size. Returns 0 (→ full source) if the
/// superblock can't be read — the staging admission still bounds it.
unsafe fn helix_footprint_bytes(c: &HelixCandidate) -> u64 {
    let total = LIVE[c.slot].total_sectors;
    let mut probe = make_raw_block_device(c.slot, total, c.sector_size);
    let sb = match morpheus_helix::log::recovery::recover_superblock(
        &mut probe,
        c.lba_start,
        c.sector_size,
    ) {
        Ok(sb) => sb,
        Err(_) => return 0,
    };

    let mut blocks = 2u64;
    let log_hi = sb.log_end_block.saturating_add(1);
    if log_hi > blocks {
        blocks = log_hi;
    }
    let data_hi = sb.data_start_block.saturating_add(sb.blocks_used);
    if data_hi > blocks {
        blocks = data_hi;
    }
    if blocks > sb.total_blocks {
        blocks = sb.total_blocks;
    }
    blocks.saturating_mul(sb.block_size as u64)
}

/// Build a privileged root `MountReq` for `volume_id` and mount it at `/`.
/// `extra_flags` carries `MNT_STAGED` (real disks) or 0 (already-RAM devices);
/// `aux` caps the staged copy (0 = full source).
unsafe fn mount_root_volume(volume_id: u64, extra_flags: u32, aux: u64) -> bool {
    let mut mp = [0u8; 256];
    mp[0] = b'/';
    let req = morpheus_kernel::storage::MountReq {
        source_volume_id: volume_id,
        mount_point: mp,
        mount_point_len: 1,
        fs_type: FS_AUTO,
        flags: extra_flags,
        aux,
        pid: 0,
        privileged: true,
    };
    morpheus_kernel::storage::mount(&req).is_ok()
}

/// Fresh empty Helix at `/` via the `VOLUME_NONE` staged-from-nothing path
/// (privileged). The RAM is allocated through the staging admission control.
unsafe fn mount_fresh_ram_root() -> bool {
    let mut mp = [0u8; 256];
    mp[0] = b'/';
    let req = morpheus_kernel::storage::MountReq {
        source_volume_id: morpheus_foundation::storage::VOLUME_NONE,
        mount_point: mp,
        mount_point_len: 1,
        fs_type: FS_HELIX,
        flags: MNT_STAGED,
        aux: RAM_ROOT_BYTES,
        pid: 0,
        privileged: true,
    };
    morpheus_kernel::storage::mount(&req).is_ok()
}

fn log_root_program_checks() {
    if !morpheus_kernel::storage::path_exists("/bin/compd") {
        log_warn("STORAGE", 849, "root check: /bin/compd missing");
    }
    if !morpheus_kernel::storage::path_exists("/bin/shelld") {
        log_warn("STORAGE", 850, "root check: /bin/shelld missing");
    }
}

pub fn is_persistent() -> bool {
    unsafe { PERSISTENT_READY }
}

/// Idempotent; call after the root FS is mounted. Routes through the kernel
/// storage subsystem's privileged mkdir on the mounted root.
pub fn create_init_directories() {
    use morpheus_hal_x86_64::cpu::tsc::read_tsc;

    if !is_persistent() {
        log_warn("INITFS", 840, "no root fs; skipping directory bootstrap");
        return;
    }

    let dirs = ["/bin", "/etc", "/tmp", "/home", "/var", "/dev"];
    let ts = read_tsc();
    for dir in &dirs {
        if morpheus_kernel::storage::mkdir_root(dir, ts).is_err() {
            let _ = dir;
            log_warn("INITFS", 841, "failed to create one startup directory");
        }
    }
}

// RawBlockDevice fn-ptr vtable → UnifiedBlockIo. Whole-disk addressing; the
// storage subsystem's per-volume backend applies the partition LBA offset. A
// single shared DMA I/O buffer is fine: runtime FS ops are serialized by
// STORAGE_LOCK, and boot probing is single-threaded.

/// Build a whole-disk `RawBlockDevice` whose ctx encodes the `LIVE` slot index.
fn make_raw_block_device(slot: usize, total_sectors: u64, sector_size: u32) -> RawBlockDevice {
    // SAFETY: ctx is just the slot index (not a dereferenced pointer); the
    // callbacks recover it and index `LIVE`, whose entries live for the runtime.
    unsafe {
        RawBlockDevice::new(
            slot as *mut u8,
            total_sectors,
            sector_size,
            raw_read,
            raw_write,
            raw_flush,
        )
    }
}

/// Recover the live driver from a callback ctx, or `None` if out of range/unused.
unsafe fn live_dev(ctx: *mut u8) -> Option<&'static mut UnifiedBlockDevice> {
    let idx = ctx as usize;
    if idx >= MAX_LIVE_DEVICES {
        return None;
    }
    LIVE[idx].dev.as_mut()
}

unsafe fn raw_read(ctx: *mut u8, lba: u64, dst: *mut u8, len: usize) -> bool {
    spinner_tick();
    let dev = match live_dev(ctx) {
        Some(s) => s,
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
    bio.read_blocks(GptLba(lba), dst_slice).is_ok()
}

unsafe fn raw_write(ctx: *mut u8, lba: u64, src: *const u8, len: usize) -> bool {
    spinner_tick();
    let dev = match live_dev(ctx) {
        Some(s) => s,
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
    bio.write_blocks(GptLba(lba), src_slice).is_ok()
}

unsafe fn raw_flush(ctx: *mut u8) -> bool {
    let dev = match live_dev(ctx) {
        Some(s) => s,
        None => return false,
    };
    dev.flush().is_ok()
}
