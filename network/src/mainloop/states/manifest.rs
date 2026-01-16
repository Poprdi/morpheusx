//! Manifest writing state — writes ISO manifest after download.
//!
//! Supports two modes:
//! - FAT32: Write to `/.iso/<name>.manifest` on ESP
//! - Raw sector: Write to a specific disk sector (legacy)
//!
//! Can be used standalone to regenerate a manifest for an existing ISO.

extern crate alloc;
use alloc::boxed::Box;
use alloc::format;

use smoltcp::iface::{Interface, SocketSet};
use smoltcp::time::Instant;

use morpheus_core::iso::{IsoManifest, MAX_MANIFEST_SIZE};

use crate::device::UnifiedBlockDevice;
use crate::driver::traits::NetworkDriver;
use crate::driver::unified_block_io::UnifiedBlockIo;
use crate::mainloop::adapter::SmoltcpAdapter;
use crate::mainloop::context::Context;
use crate::mainloop::serial;
use crate::mainloop::state::{State, StepResult};

use super::{DoneState, FailedState};

/// Max ISO name length we store.
const MAX_ISO_NAME_LEN: usize = 128;

/// DMA buffer size for FAT32 operations.
const FAT32_DMA_BUFFER_SIZE: usize = 64 * 1024;

/// Static DMA buffer for FAT32 manifest operations.
/// Separate from disk_writer's buffer to avoid conflicts.
static mut FAT32_DMA_BUFFER: [u8; FAT32_DMA_BUFFER_SIZE] = [0u8; FAT32_DMA_BUFFER_SIZE];

/// Manifest write mode.
#[derive(Debug, Clone, Copy)]
pub enum ManifestMode {
    /// Write to FAT32 ESP filesystem
    Fat32 { esp_start_lba: u64 },
    /// Write to raw disk sector
    RawSector { sector: u64 },
    /// Skip manifest writing
    Skip,
}

/// Configuration for manifest writing.
#[derive(Debug, Clone)]
pub struct ManifestConfig {
    /// ISO name stored inline (avoids lifetime issues)
    pub iso_name_buf: [u8; MAX_ISO_NAME_LEN],
    /// ISO name length
    pub iso_name_len: usize,
    /// Total ISO size in bytes
    pub iso_size: u64,
    /// Start sector where ISO data begins
    pub start_sector: u64,
    /// End sector (exclusive)
    pub end_sector: u64,
    /// Partition UUID (16 bytes)
    pub partition_uuid: [u8; 16],
    /// Write mode
    pub mode: ManifestMode,
}

impl ManifestConfig {
    /// Get ISO name as string slice.
    pub fn iso_name(&self) -> &str {
        core::str::from_utf8(&self.iso_name_buf[..self.iso_name_len]).unwrap_or("unknown")
    }

    /// Create config with name copied into buffer.
    pub fn new(
        iso_name: &str,
        iso_size: u64,
        start_sector: u64,
        end_sector: u64,
        partition_uuid: [u8; 16],
        mode: ManifestMode,
    ) -> Self {
        let mut iso_name_buf = [0u8; MAX_ISO_NAME_LEN];
        let len = iso_name.len().min(MAX_ISO_NAME_LEN);
        iso_name_buf[..len].copy_from_slice(&iso_name.as_bytes()[..len]);

        Self {
            iso_name_buf,
            iso_name_len: len,
            iso_size,
            start_sector,
            end_sector,
            partition_uuid,
            mode,
        }
    }

    /// Create config for FAT32 manifest.
    pub fn fat32(
        iso_name: &str,
        iso_size: u64,
        start_sector: u64,
        end_sector: u64,
        partition_uuid: [u8; 16],
        esp_start_lba: u64,
    ) -> Self {
        Self::new(
            iso_name,
            iso_size,
            start_sector,
            end_sector,
            partition_uuid,
            ManifestMode::Fat32 { esp_start_lba },
        )
    }

    /// Create config for raw sector manifest.
    pub fn raw_sector(
        iso_name: &str,
        iso_size: u64,
        start_sector: u64,
        end_sector: u64,
        partition_uuid: [u8; 16],
        manifest_sector: u64,
    ) -> Self {
        Self::new(
            iso_name,
            iso_size,
            start_sector,
            end_sector,
            partition_uuid,
            ManifestMode::RawSector { sector: manifest_sector },
        )
    }

    /// Create config to skip manifest writing.
    pub fn skip() -> Self {
        Self {
            iso_name_buf: [0u8; MAX_ISO_NAME_LEN],
            iso_name_len: 0,
            iso_size: 0,
            start_sector: 0,
            end_sector: 0,
            partition_uuid: [0u8; 16],
            mode: ManifestMode::Skip,
        }
    }
}

/// Manifest writing state.
pub struct ManifestState {
    config: ManifestConfig,
    started: bool,
    completed: bool,
}

impl ManifestState {
    /// Create manifest state with configuration.
    pub fn new(config: ManifestConfig) -> Self {
        Self {
            config,
            started: false,
            completed: false,
        }
    }

    /// Create from context after download.
    pub fn from_context(ctx: &Context<'_>) -> Self {
        let iso_size = ctx.bytes_downloaded;
        // Use actual_start_sector (set by GPT prep) rather than config
        let start_sector = ctx.actual_start_sector;
        let num_sectors = (iso_size + 511) / 512;
        let end_sector = start_sector + num_sectors;

        let mode = if ctx.config.esp_start_lba > 0 {
            ManifestMode::Fat32 { esp_start_lba: ctx.config.esp_start_lba }
        } else if ctx.config.manifest_sector > 0 {
            ManifestMode::RawSector { sector: ctx.config.manifest_sector }
        } else {
            ManifestMode::Skip
        };

        Self::new(ManifestConfig::new(
            ctx.config.iso_name,
            iso_size,
            start_sector,
            end_sector,
            ctx.config.partition_uuid,
            mode,
        ))
    }

    /// Build manifest structure.
    fn build_manifest(&self) -> Option<IsoManifest> {
        let mut manifest = IsoManifest::new(self.config.iso_name(), self.config.iso_size);

        if manifest.add_chunk(
            self.config.partition_uuid,
            self.config.start_sector,
            self.config.end_sector,
        ).is_err() {
            serial::println("[MANIFEST] ERROR: Failed to add chunk");
            return None;
        }

        if let Some(chunk) = manifest.chunks.chunks.get_mut(0) {
            chunk.data_size = self.config.iso_size;
            chunk.written = true;
        }

        manifest.mark_complete();
        Some(manifest)
    }

    /// Write manifest to FAT32 ESP filesystem.
    fn write_fat32(&self, blk: &mut UnifiedBlockDevice, esp_start_lba: u64) -> bool {
        serial::println("[MANIFEST] Writing to FAT32 ESP...");
        serial::print("[MANIFEST] ESP start LBA: ");
        serial::print_hex(esp_start_lba);
        serial::println("");
        serial::print("[MANIFEST] ISO start sector: ");
        serial::print_hex(self.config.start_sector);
        serial::println("");

        let manifest = match self.build_manifest() {
            Some(m) => m,
            None => return false,
        };

        // Serialize manifest
        let mut manifest_buffer = [0u8; MAX_MANIFEST_SIZE];
        let manifest_len = match manifest.serialize(&mut manifest_buffer) {
            Ok(len) => {
                serial::print("[MANIFEST] Serialized ");
                serial::print_u32(len as u32);
                serial::println(" bytes");
                len
            }
            Err(_) => {
                serial::println("[MANIFEST] ERROR: Failed to serialize manifest");
                return false;
            }
        };

        // Create BlockIo adapter for FAT32 operations
        serial::println("[MANIFEST] Creating BlockIo adapter for FAT32...");
        let (dma_buffer, dma_buffer_phys) = unsafe {
            let buf = core::slice::from_raw_parts_mut(
                (&raw mut FAT32_DMA_BUFFER).cast::<u8>(),
                FAT32_DMA_BUFFER_SIZE,
            );
            let phys = (&raw const FAT32_DMA_BUFFER).cast::<u8>() as u64;
            (buf, phys)
        };
        let timeout_ticks = 500_000_000u64; // ~500ms

        let mut adapter = match UnifiedBlockIo::new(blk, dma_buffer, dma_buffer_phys, timeout_ticks) {
            Ok(a) => {
                serial::println("[MANIFEST] BlockIo adapter created");
                a
            }
            Err(_) => {
                serial::println("[MANIFEST] ERROR: Failed to create BlockIo adapter");
                return false;
            }
        };

        // Generate 8.3 compatible manifest filename
        let manifest_filename = morpheus_core::fs::generate_8_3_manifest_name(self.config.iso_name());
        let manifest_path = format!("/.iso/{}", manifest_filename);

        serial::print("[MANIFEST] Writing to: ");
        serial::println(&manifest_path);

        // Ensure .iso directory exists
        let _ = morpheus_core::fs::create_directory(&mut adapter, esp_start_lba, "/.iso");

        // Write manifest file
        match morpheus_core::fs::write_file(
            &mut adapter,
            esp_start_lba,
            &manifest_path,
            &manifest_buffer[..manifest_len],
        ) {
            Ok(()) => {
                serial::println("[MANIFEST] OK: Written to ESP");
                true
            }
            Err(e) => {
                serial::print("[MANIFEST] ERROR: FAT32 write failed: ");
                serial::println(match e {
                    morpheus_core::fs::Fat32Error::IoError => "IO error",
                    morpheus_core::fs::Fat32Error::PartitionTooSmall => "Partition too small",
                    morpheus_core::fs::Fat32Error::PartitionTooLarge => "Partition too large",
                    morpheus_core::fs::Fat32Error::InvalidBlockSize => "Invalid block size",
                    morpheus_core::fs::Fat32Error::NotImplemented => "Not implemented",
                });
                false
            }
        }
    }

    /// Write manifest using raw sector method.
    fn write_raw_sector(&self, blk: &mut UnifiedBlockDevice, sector: u64) -> bool {
        serial::println("[MANIFEST] Writing to raw sector...");
        serial::print("[MANIFEST] Sector: ");
        serial::print_hex(sector);
        serial::println("");

        let manifest = match self.build_manifest() {
            Some(m) => m,
            None => return false,
        };

        // Serialize
        let mut buffer = [0u8; MAX_MANIFEST_SIZE];
        let len = match manifest.serialize(&mut buffer) {
            Ok(l) => l,
            Err(_) => {
                serial::println("[MANIFEST] ERROR: Serialize failed");
                return false;
            }
        };

        serial::print("[MANIFEST] Serialized ");
        serial::print_u32(len as u32);
        serial::println(" bytes");

        // Write to disk
        unsafe { write_sector(blk, sector, &buffer) }
    }
}

impl<D: NetworkDriver> State<D> for ManifestState {
    fn step(
        mut self: Box<Self>,
        ctx: &mut Context<'_>,
        _iface: &mut Interface,
        _sockets: &mut SocketSet<'_>,
        _adapter: &mut SmoltcpAdapter<'_, D>,
        _now: Instant,
        _tsc: u64,
    ) -> (Box<dyn State<D>>, StepResult) {
        if self.completed {
            return (Box::new(DoneState::new()), StepResult::Transition);
        }

        if !self.started {
            self.started = true;
            
            match self.config.mode {
                ManifestMode::Skip => {
                    serial::println("[MANIFEST] Skipping (not configured)");
                    self.completed = true;
                    return (self, StepResult::Continue);
                }
                ManifestMode::Fat32 { esp_start_lba } => {
                    serial::println("=================================");
                    serial::println("     WRITING ISO MANIFEST        ");
                    serial::println("=================================");
                    serial::print("[MANIFEST] ISO: ");
                    serial::println(self.config.iso_name());
                    serial::print("[MANIFEST] Size: ");
                    serial::print_u32((self.config.iso_size / 1024 / 1024) as u32);
                    serial::println(" MB");
                    serial::print("[MANIFEST] Sectors: ");
                    serial::print_hex(self.config.start_sector);
                    serial::print(" - ");
                    serial::print_hex(self.config.end_sector);
                    serial::println("");
                    serial::print("[MANIFEST] Mode: FAT32 (ESP LBA ");
                    serial::print_u32(esp_start_lba as u32);
                    serial::println(")");

                    let blk = match &mut ctx.blk_device {
                        Some(b) => b,
                        None => {
                            serial::println("[MANIFEST] ERROR: No block device");
                            return (Box::new(FailedState::new("no block device")), StepResult::Failed("no blk"));
                        }
                    };

                    if self.write_fat32(blk, esp_start_lba) {
                        serial::println("[MANIFEST] Write successful");
                        self.completed = true;
                    } else {
                        return (Box::new(FailedState::new("manifest write failed")), StepResult::Failed("write"));
                    }
                }
                ManifestMode::RawSector { sector } => {
                    serial::println("=================================");
                    serial::println("     WRITING ISO MANIFEST        ");
                    serial::println("=================================");
                    serial::print("[MANIFEST] ISO: ");
                    serial::println(self.config.iso_name());
                    serial::print("[MANIFEST] Size: ");
                    serial::print_u32((self.config.iso_size / 1024 / 1024) as u32);
                    serial::println(" MB");

                    let blk = match &mut ctx.blk_device {
                        Some(b) => b,
                        None => {
                            serial::println("[MANIFEST] ERROR: No block device");
                            return (Box::new(FailedState::new("no block device")), StepResult::Failed("no blk"));
                        }
                    };

                    if self.write_raw_sector(blk, sector) {
                        serial::println("[MANIFEST] Write successful");
                        self.completed = true;
                    } else {
                        return (Box::new(FailedState::new("manifest write failed")), StepResult::Failed("write"));
                    }
                }
            }
        }

        (self, StepResult::Continue)
    }

    fn name(&self) -> &'static str {
        "Manifest"
    }
}

/// Write a buffer to a disk sector.
unsafe fn write_sector(blk: &mut UnifiedBlockDevice, sector: u64, data: &[u8]) -> bool {
    use crate::driver::block_traits::BlockDriver;

    // Pad to sector size
    static mut SECTOR_BUF: [u8; 512] = [0u8; 512];
    let copy_len = data.len().min(512);
    SECTOR_BUF[..copy_len].copy_from_slice(&data[..copy_len]);
    for i in copy_len..512 {
        SECTOR_BUF[i] = 0;
    }

    let buffer_phys = (&raw const SECTOR_BUF).cast::<u8>() as u64;

    // Drain pending
    while blk.poll_completion().is_some() {}

    if !blk.can_submit() {
        serial::println("[MANIFEST] ERROR: Queue full");
        return false;
    }

    let request_id = 0xFFFF_0001u32;
    if blk.submit_write(sector, buffer_phys, 1, request_id).is_err() {
        serial::println("[MANIFEST] ERROR: Submit failed");
        return false;
    }

    blk.notify();

    // Poll for completion
    let start = read_tsc();
    let timeout: u64 = 2_000_000_000; // ~500ms

    loop {
        if let Some(completion) = blk.poll_completion() {
            if completion.request_id == request_id {
                return completion.status == 0;
            }
        }
        if read_tsc().wrapping_sub(start) > timeout {
            serial::println("[MANIFEST] ERROR: Timeout");
            return false;
        }
        core::hint::spin_loop();
    }
}

#[cfg(target_arch = "x86_64")]
#[inline]
fn read_tsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nostack, nomem));
    }
    ((hi as u64) << 32) | (lo as u64)
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
fn read_tsc() -> u64 {
    0
}

// ============================================================================
// Standalone API for manifest regeneration
// ============================================================================

/// Write a manifest for an existing ISO without using the state machine.
///
/// Use case: Recreate a manifest for an ISO that was previously downloaded
/// but whose manifest was lost or corrupted.
///
/// # Arguments
/// * `blk` - Block device to write to
/// * `config` - Manifest configuration describing the ISO location
///
/// # Returns
/// `true` if manifest was written successfully
pub fn write_manifest_standalone(
    blk: &mut UnifiedBlockDevice,
    config: &ManifestConfig,
) -> bool {
    let state = ManifestState::new(config.clone());

    match config.mode {
        ManifestMode::Skip => {
            serial::println("[MANIFEST] Skipping (not configured)");
            true
        }
        ManifestMode::Fat32 { esp_start_lba } => {
            state.write_fat32(blk, esp_start_lba)
        }
        ManifestMode::RawSector { sector } => {
            state.write_raw_sector(blk, sector)
        }
    }
}

/// Regenerate manifest for an existing ISO on disk.
///
/// Convenience wrapper that creates the config and writes the manifest.
///
/// # Arguments
/// * `blk` - Block device
/// * `iso_name` - Name of the ISO (e.g., "tails-6.10.iso")
/// * `iso_size` - Total size in bytes
/// * `start_sector` - First sector of ISO data
/// * `end_sector` - End sector (exclusive)
/// * `partition_uuid` - UUID of the partition containing the ISO
/// * `esp_start_lba` - ESP start LBA (for FAT32 mode, 0 to skip)
/// * `manifest_sector` - Raw sector for manifest (for raw mode, 0 to skip)
pub fn regenerate_manifest(
    blk: &mut UnifiedBlockDevice,
    iso_name: &str,
    iso_size: u64,
    start_sector: u64,
    end_sector: u64,
    partition_uuid: [u8; 16],
    esp_start_lba: u64,
    manifest_sector: u64,
) -> bool {
    serial::println("=================================");
    serial::println("  REGENERATING ISO MANIFEST      ");
    serial::println("=================================");
    serial::print("[MANIFEST] ISO: ");
    serial::println(iso_name);
    serial::print("[MANIFEST] Size: ");
    serial::print_u32((iso_size / 1024 / 1024) as u32);
    serial::println(" MB");
    serial::print("[MANIFEST] Sectors: ");
    serial::print_hex(start_sector);
    serial::print(" - ");
    serial::print_hex(end_sector);
    serial::println("");

    let mode = if esp_start_lba > 0 {
        serial::print("[MANIFEST] Mode: FAT32 (ESP LBA ");
        serial::print_u32(esp_start_lba as u32);
        serial::println(")");
        ManifestMode::Fat32 { esp_start_lba }
    } else if manifest_sector > 0 {
        serial::print("[MANIFEST] Mode: Raw sector ");
        serial::print_hex(manifest_sector);
        serial::println("");
        ManifestMode::RawSector { sector: manifest_sector }
    } else {
        serial::println("[MANIFEST] ERROR: No write mode specified");
        return false;
    };

    let config = ManifestConfig::new(
        iso_name,
        iso_size,
        start_sector,
        end_sector,
        partition_uuid,
        mode,
    );

    write_manifest_standalone(blk, &config)
}
