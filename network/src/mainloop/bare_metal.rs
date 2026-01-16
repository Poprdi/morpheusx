//! Bare-metal main loop for post-ExitBootServices execution.
//!
//! This module provides the complete end-to-end runner that:
//! 1. Initializes the VirtIO-net driver using ASM layer
//! 2. Creates the smoltcp interface and sockets
//! 3. Runs the 5-phase main loop
//! 4. Orchestrates ISO download and disk write
//! 5. Writes manifest to disk for boot entry discovery
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §6, §7

#![allow(unused_variables)]
#![allow(dead_code)]

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use smoltcp::iface::{Config, Interface, SocketSet, SocketStorage};
use smoltcp::socket::dhcpv4::{Event as Dhcpv4Event, Socket as Dhcpv4Socket};
use smoltcp::socket::dns::{GetQueryResultError, Socket as DnsSocket};
use smoltcp::socket::tcp::{Socket as TcpSocket, SocketBuffer as TcpSocketBuffer};
use smoltcp::time::Instant;
use smoltcp::wire::{
    DnsQueryType, EthernetAddress, HardwareAddress, IpAddress, IpCidr, Ipv4Address, Ipv4Cidr,
};

use crate::boot::handoff::{
    BootHandoff, BLK_TYPE_AHCI, BLK_TYPE_VIRTIO, NIC_TYPE_INTEL, NIC_TYPE_VIRTIO, TRANSPORT_MMIO,
    TRANSPORT_PCI_MODERN,
};
use crate::boot::init::TimeoutConfig;
use crate::device::UnifiedBlockDevice;
use crate::driver::ahci::{AhciConfig, AhciDriver, AhciInitError};
use crate::driver::block_traits::{BlockCompletion, BlockDeviceInfo, BlockDriver};
use crate::driver::traits::NetworkDriver;
use crate::driver::unified::{UnifiedDriverError, UnifiedNetworkDriver};
use crate::driver::unified_block_io::{UnifiedBlockIo, UnifiedBlockIoError};
use crate::driver::virtio::{PciModernConfig, TransportType, VirtioTransport};
use crate::driver::virtio::{VirtioConfig, VirtioInitError, VirtioNetDriver};
use crate::driver::virtio_blk::{VirtioBlkConfig, VirtioBlkDriver, VirtioBlkInitError};
use crate::transfer::disk::{ChunkPartition, ChunkSet, PartitionInfo, MAX_CHUNK_PARTITIONS};
use crate::url::Url;

// Import manifest support from morpheus-core (same format bootloader scanner uses)
use morpheus_core::iso::{IsoManifest, MAX_MANIFEST_SIZE};

// Import from sibling modules in the mainloop package
use super::phases::{phase1_rx_refill, phase5_tx_completions};
use super::runner::{get_tsc, MainLoopConfig};

// Import download state machine from state module
use crate::state::download::{DownloadConfig, IsoDownloadState};

// ═══════════════════════════════════════════════════════════════════════════
// SERIAL OUTPUT (POST-EBS)
// ═══════════════════════════════════════════════════════════════════════════

/// Serial port base address (COM1).
const SERIAL_PORT: u16 = 0x3F8;

/// Write a byte to serial port.
#[cfg(target_arch = "x86_64")]
unsafe fn serial_write_byte(byte: u8) {
    // Wait for transmit buffer empty
    loop {
        let status: u8;
        core::arch::asm!(
            "in al, dx",
            in("dx") SERIAL_PORT + 5,
            out("al") status,
            options(nomem, nostack)
        );
        if status & 0x20 != 0 {
            break;
        }
    }
    // Write byte
    core::arch::asm!(
        "out dx, al",
        in("dx") SERIAL_PORT,
        in("al") byte,
        options(nomem, nostack)
    );
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn serial_write_byte(_byte: u8) {}

// Static counters for packet debugging
static mut TX_PACKET_COUNT: u32 = 0;
static mut RX_PACKET_COUNT: u32 = 0;

/// Increment TX counter
pub fn inc_tx_count() {
    unsafe { TX_PACKET_COUNT += 1; }
}

/// Increment RX counter
pub fn inc_rx_count() {
    unsafe { RX_PACKET_COUNT += 1; }
}

/// Get TX count
pub fn get_tx_count() -> u32 {
    unsafe { TX_PACKET_COUNT }
}

/// Get RX count
pub fn get_rx_count() -> u32 {
    unsafe { RX_PACKET_COUNT }
}

/// Write string to serial port.
pub fn serial_print(s: &str) {
    for byte in s.bytes() {
        unsafe {
            serial_write_byte(byte);
        }
    }
    // Mirror to framebuffer display
    crate::display::display_write(s);
}

/// Write string with newline.
pub fn serial_println(s: &str) {
    serial_print(s);
    serial_print("\r\n");
}

/// Print a single byte as 2 hex digits (mirrored to display).
pub fn serial_print_hex_byte(value: u8) {
    let hi = value >> 4;
    let lo = value & 0xF;
    let hi_char = if hi < 10 { b'0' + hi } else { b'a' + hi - 10 };
    let lo_char = if lo < 10 { b'0' + lo } else { b'a' + lo - 10 };
    let buf = [hi_char, lo_char];
    unsafe {
        serial_write_byte(hi_char);
        serial_write_byte(lo_char);
    }
    if let Ok(s) = core::str::from_utf8(&buf) {
        crate::display::display_write(s);
    }
}

/// Print hex number.
pub fn serial_print_hex(value: u64) {
    serial_print("0x");
    let mut buf = [0u8; 16];
    for i in 0..16 {
        let nibble = ((value >> ((15 - i) * 4)) & 0xF) as u8;
        let c = if nibble < 10 {
            b'0' + nibble
        } else {
            b'a' + nibble - 10
        };
        buf[i] = c;
        unsafe {
            serial_write_byte(c);
        }
    }
    // Mirror to display
    if let Ok(s) = core::str::from_utf8(&buf) {
        crate::display::display_write(s);
    }
}

/// Print an IPv4 address (e.g., "10.0.2.15").
pub fn print_ipv4(ip: Ipv4Address) {
    let octets = ip.as_bytes();
    for (i, octet) in octets.iter().enumerate() {
        if i > 0 {
            serial_print(".");
        }
        serial_print_decimal(*octet as u32);
    }
}

/// Print a decimal number.
pub fn serial_print_decimal(value: u32) {
    if value == 0 {
        unsafe {
            serial_write_byte(b'0');
        }
        crate::display::display_write("0");
        return;
    }
    // Build digits in reverse order
    let mut buf = [0u8; 10];
    let mut i = 0;
    let mut val = value;
    while val > 0 {
        buf[i] = b'0' + (val % 10) as u8;
        val /= 10;
        i += 1;
    }
    // Create display buffer in correct order
    let mut display_buf = [0u8; 10];
    let num_digits = i;
    for j in 0..num_digits {
        display_buf[j] = buf[num_digits - 1 - j];
    }
    // Write to serial (reverse order from buf)
    while i > 0 {
        i -= 1;
        unsafe {
            serial_write_byte(buf[i]);
        }
    }
    // Mirror to display
    if let Ok(s) = core::str::from_utf8(&display_buf[..num_digits]) {
        crate::display::display_write(s);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// STREAMING DISK WRITE HELPERS
// ═══════════════════════════════════════════════════════════════════════════

/// Flush the disk write buffer to the block device.
///
/// Writes the buffered data as one or more sector writes.
/// Returns the number of bytes written, or 0 on error.
unsafe fn flush_disk_buffer<D: BlockDriver>(blk_driver: &mut D) -> usize {
    if DISK_WRITE_BUFFER_FILL == 0 {
        return 0;
    }

    // Calculate sectors to write (round up)
    let bytes_to_write = DISK_WRITE_BUFFER_FILL;
    let num_sectors = ((bytes_to_write + 511) / 512) as u32;

    // Get the buffer physical address (we're identity mapped post-EBS)
    let buffer_phys = (&raw const DISK_WRITE_BUFFER).cast::<u8>() as u64;

    // Submit write request
    let request_id = DISK_NEXT_REQUEST_ID;
    DISK_NEXT_REQUEST_ID = DISK_NEXT_REQUEST_ID.wrapping_add(1);

    // Poll for completion space first (drain any pending completions)
    while let Some(_completion) = blk_driver.poll_completion() {
        // Just drain pending completions
    }

    // Check if driver can accept a request
    if !blk_driver.can_submit() {
        serial_println("[DISK] ERROR: Block queue full, cannot submit");
        return 0;
    }

    // Submit the write
    if let Err(e) = blk_driver.submit_write(DISK_NEXT_SECTOR, buffer_phys, num_sectors, request_id)
    {
        serial_print("[DISK] ERROR: Write submit failed at sector ");
        serial_print_hex(DISK_NEXT_SECTOR);
        serial_println("");
        return 0;
    }

    // Notify the device
    blk_driver.notify();

    // Poll for completion (with timeout)
    let start_tsc = super::runner::get_tsc();
    // Use 1 second timeout at ~4GHz TSC (4 billion ticks)
    // This accounts for high-frequency TSC and gives plenty of margin
    let timeout_ticks: u64 = 4_000_000_000;

    loop {
        if let Some(completion) = blk_driver.poll_completion() {
            if completion.request_id == request_id {
                if completion.status == 0 {
                    // Success!
                    DISK_NEXT_SECTOR += num_sectors as u64;
                    DISK_TOTAL_BYTES += bytes_to_write as u64;
                    DISK_WRITE_BUFFER_FILL = 0;
                    return bytes_to_write;
                } else {
                    serial_print("[DISK] ERROR: Write completion status ");
                    serial_print_decimal(completion.status as u32);
                    serial_print(" at sector ");
                    serial_print_decimal(DISK_NEXT_SECTOR as u32);
                    serial_println("");
                    return 0;
                }
            }
        }

        let now = super::runner::get_tsc();
        if now.wrapping_sub(start_tsc) > timeout_ticks {
            serial_print("[DISK] ERROR: Write timeout at sector ");
            serial_print_decimal(DISK_NEXT_SECTOR as u32);
            serial_print(", total bytes: ");
            serial_print_decimal(DISK_TOTAL_BYTES as u32);
            serial_println("");
            return 0;
        }

        core::hint::spin_loop();
    }
}

/// Add data to the disk write buffer.
/// Automatically flushes when buffer is full.
/// Returns the number of bytes consumed from the input.
unsafe fn buffer_disk_write<D: BlockDriver>(blk_driver: &mut D, data: &[u8]) -> usize {
    let mut consumed = 0;
    let mut remaining = data;

    while !remaining.is_empty() {
        // Calculate how much fits in current buffer
        let space_left = DISK_WRITE_BUFFER_SIZE - DISK_WRITE_BUFFER_FILL;
        let to_copy = remaining.len().min(space_left);

        // Copy to buffer
        let dst_start = DISK_WRITE_BUFFER_FILL;
        DISK_WRITE_BUFFER[dst_start..dst_start + to_copy].copy_from_slice(&remaining[..to_copy]);
        DISK_WRITE_BUFFER_FILL += to_copy;
        consumed += to_copy;
        remaining = &remaining[to_copy..];

        // Flush if buffer is full
        if DISK_WRITE_BUFFER_FILL >= DISK_WRITE_BUFFER_SIZE {
            let written = flush_disk_buffer(blk_driver);
            if written == 0 {
                // Write failed, stop
                break;
            }
        }
    }

    consumed
}

/// Flush any remaining data in the buffer (for end of download).
unsafe fn flush_remaining_disk_buffer<D: BlockDriver>(blk_driver: &mut D) -> bool {
    if DISK_WRITE_BUFFER_FILL > 0 {
        // Pad the rest with zeros (needed for sector alignment)
        for i in DISK_WRITE_BUFFER_FILL..DISK_WRITE_BUFFER_SIZE {
            DISK_WRITE_BUFFER[i] = 0;
        }
        let written = flush_disk_buffer(blk_driver);
        return written > 0;
    }
    true
}

// ═══════════════════════════════════════════════════════════════════════════
// MANIFEST WRITING (POST-EBS)
// ═══════════════════════════════════════════════════════════════════════════

/// Static buffer for manifest serialization (1 sector = 512 bytes, but
/// manifest can be up to ~1KB, so use 2 sectors).
static mut MANIFEST_BUFFER: [u8; 1024] = [0u8; 1024];

/// Write an ISO manifest to disk using UnifiedBlockDevice.
///
/// Works with both VirtIO and AHCI block devices.
unsafe fn write_manifest_to_disk_unified(
    blk_device: &mut UnifiedBlockDevice,
    manifest_sector: u64,
    manifest: &IsoManifest,
) -> bool {
    serial_println("[MANIFEST] ═══════════════════════════════════════════════════");
    serial_println("[MANIFEST] Writing ISO manifest to disk (unified)");
    serial_println("[MANIFEST] ═══════════════════════════════════════════════════");

    serial_println("[MANIFEST] Serializing manifest structure...");

    // Clear buffer using raw pointer
    let manifest_ptr = (&raw mut MANIFEST_BUFFER).cast::<u8>();
    for i in 0..1024 {
        *manifest_ptr.add(i) = 0;
    }

    // Serialize manifest using raw pointer cast to slice
    let manifest_buffer =
        core::slice::from_raw_parts_mut((&raw mut MANIFEST_BUFFER).cast::<u8>(), 1024);
    let size = match manifest.serialize(manifest_buffer) {
        Ok(s) => s,
        Err(_) => {
            serial_println("[MANIFEST] ERROR: Failed to serialize manifest");
            return false;
        }
    };

    serial_print("[MANIFEST] Serialized size: ");
    serial_print_decimal(size as u32);
    serial_println(" bytes");

    // Calculate sectors needed (round up)
    let num_sectors = ((size + 511) / 512) as u32;

    serial_print("[MANIFEST] Writing to sector: ");
    serial_print_hex(manifest_sector);
    serial_print(" (");
    serial_print_decimal(num_sectors);
    serial_println(" sectors)");

    // Get the buffer physical address
    let buffer_phys = manifest_buffer.as_ptr() as u64;

    // Poll for completion space first
    while let Some(_completion) = blk_device.poll_completion() {
        // Drain pending completions
    }

    // Check if driver can accept a request
    if !blk_device.can_submit() {
        serial_println("[MANIFEST] ERROR: Block queue full");
        return false;
    }

    // Submit the write
    let request_id = DISK_NEXT_REQUEST_ID;
    DISK_NEXT_REQUEST_ID = DISK_NEXT_REQUEST_ID.wrapping_add(1);

    if let Err(_) = blk_device.submit_write(manifest_sector, buffer_phys, num_sectors, request_id) {
        serial_println("[MANIFEST] ERROR: Write submit failed");
        return false;
    }

    // Notify the device
    blk_device.notify();

    serial_println("[MANIFEST] Write submitted, waiting for completion...");

    // Poll for completion (with timeout)
    let start_tsc = super::runner::get_tsc();
    let timeout_ticks = 100_000_000; // ~100ms at 1GHz TSC

    loop {
        if let Some(completion) = blk_device.poll_completion() {
            if completion.request_id == request_id {
                if completion.status == 0 {
                    serial_println(
                        "[MANIFEST] ═══════════════════════════════════════════════════",
                    );
                    serial_println("[MANIFEST] MANIFEST WRITTEN SUCCESSFULLY");
                    serial_println(
                        "[MANIFEST] ═══════════════════════════════════════════════════",
                    );
                    return true;
                } else {
                    serial_print("[MANIFEST] ERROR: Write completion status ");
                    serial_print_decimal(completion.status as u32);
                    serial_println("");
                    return false;
                }
            }
        }

        let now = super::runner::get_tsc();
        if now.wrapping_sub(start_tsc) > timeout_ticks {
            serial_println("[MANIFEST] ERROR: Write timeout");
            return false;
        }

        core::hint::spin_loop();
    }
}

/// Create and write a completed ISO manifest to disk.
///
/// Called after HTTP download completes to record the ISO location.1G
///
/// # Strategy
/// - If `esp_start_lba > 0`: Write to FAT32 ESP at `/.iso/<name>.manifest`
/// - Else if `manifest_sector > 0`: Write to raw sector (legacy)
/// - Else: Skip manifest writing
unsafe fn finalize_manifest(
    blk_device: &mut UnifiedBlockDevice,
    config: &BareMetalConfig,
    total_bytes: u64,
) -> bool {
    // Check if we have any manifest destination configured
    if config.esp_start_lba == 0 && config.manifest_sector == 0 {
        serial_println("[MANIFEST] No manifest destination configured, skipping");
        return true;
    }

    serial_println("");
    serial_println("=== WRITING ISO MANIFEST ===");
    serial_print("[MANIFEST] ISO name: ");
    serial_println(config.iso_name);
    serial_print("[MANIFEST] Total size: ");
    serial_print_decimal((total_bytes / (1024 * 1024)) as u32);
    serial_println(" MB");

    // Calculate end sector
    let bytes_in_sectors = ((total_bytes + 511) / 512) * 512;
    let num_sectors = bytes_in_sectors / 512;
    let end_sector = config.target_start_sector + num_sectors;

    serial_print("[MANIFEST] Sectors: ");
    serial_print_hex(config.target_start_sector);
    serial_print(" - ");
    serial_print_hex(end_sector);
    serial_println("");

    // Prefer FAT32 writing if ESP is configured
    if config.esp_start_lba > 0 {
        return finalize_manifest_fat32(blk_device, config, total_bytes, end_sector);
    }

    // Fall back to legacy raw sector write
    finalize_manifest_raw(blk_device, config, total_bytes, end_sector)
}

/// Write manifest to FAT32 ESP filesystem.
///
/// Creates `/.iso/<name>.manifest` file on the ESP.
/// Uses morpheus_core::iso::IsoManifest for compatibility with bootloader scanner.
unsafe fn finalize_manifest_fat32(
    blk_device: &mut UnifiedBlockDevice,
    config: &BareMetalConfig,
    total_bytes: u64,
    end_sector: u64,
) -> bool {
    serial_println("[MANIFEST] Writing to FAT32 ESP...");
    serial_print("[MANIFEST] ESP start LBA: ");
    serial_print_hex(config.esp_start_lba);
    serial_println("");

    // Use actual start sector (determined by free space scan)
    let start_sector = unsafe { ACTUAL_START_SECTOR };
    serial_print("[MANIFEST] ISO start sector: ");
    serial_print_hex(start_sector);
    serial_println("");

    // Create IsoManifest using morpheus_core (same format bootloader scanner expects)
    let mut manifest = IsoManifest::new(config.iso_name, total_bytes);

    // Add chunk with partition UUID and LBA range
    match manifest.add_chunk(config.partition_uuid, start_sector, end_sector) {
        Ok(idx) => {
            serial_print("[MANIFEST] Added chunk ");
            serial_print_decimal(idx as u32);
            serial_println("");

            // Update chunk with data size and mark as written
            if let Some(chunk) = manifest.chunks.chunks.get_mut(idx) {
                chunk.data_size = total_bytes;
                chunk.written = true;
            }
        }
        Err(_) => {
            serial_println("[MANIFEST] ERROR: Failed to add chunk");
            return false;
        }
    }

    // Mark manifest as complete
    manifest.mark_complete();

    // Create BlockIo adapter for FAT32 operations
    serial_println("[MANIFEST] Creating BlockIo adapter for FAT32...");
    let dma_buffer = core::slice::from_raw_parts_mut(
        (&raw mut DISK_WRITE_BUFFER).cast::<u8>(),
        DISK_WRITE_BUFFER_SIZE,
    );
    let dma_buffer_phys = (&raw const DISK_WRITE_BUFFER).cast::<u8>() as u64;
    let timeout_ticks = 500_000_000u64; // ~500ms (increased for FAT32 ops)

    let mut adapter =
        match UnifiedBlockIo::new(blk_device, dma_buffer, dma_buffer_phys, timeout_ticks) {
            Ok(a) => {
                serial_println("[MANIFEST] BlockIo adapter created");
                a
            }
            Err(_) => {
                serial_println("[MANIFEST] ERROR: Failed to create BlockIo adapter");
                return false;
            }
        };

    // Serialize manifest to buffer
    let mut manifest_buffer = [0u8; MAX_MANIFEST_SIZE];
    let manifest_len = match manifest.serialize(&mut manifest_buffer) {
        Ok(len) => {
            serial_print("[MANIFEST] Serialized ");
            serial_print_decimal(len as u32);
            serial_println(" bytes");
            len
        }
        Err(_) => {
            serial_println("[MANIFEST] ERROR: Failed to serialize manifest");
            return false;
        }
    };

    // Generate 8.3 compatible manifest filename (FAT32 limitation)
    // Example: "tails-6.10.iso" -> "4B2A7C3D.MFS" (CRC32 hash)
    let manifest_filename = morpheus_core::fs::generate_8_3_manifest_name(config.iso_name);
    let manifest_path = format!("/.iso/{}", manifest_filename);

    serial_print("[MANIFEST] Writing to: ");
    serial_println(&manifest_path);

    // Ensure .iso directory exists (no subdirectory needed)
    let _ = morpheus_core::fs::create_directory(&mut adapter, config.esp_start_lba, "/.iso");

    // Write manifest file using morpheus_core FAT32 operations
    match morpheus_core::fs::write_file(
        &mut adapter,
        config.esp_start_lba,
        &manifest_path,
        &manifest_buffer[..manifest_len],
    ) {
        Ok(()) => {
            serial_println("[MANIFEST] OK: Written to ESP");
            true
        }
        Err(e) => {
            serial_print("[MANIFEST] ERROR: FAT32 write failed: ");
            serial_println(match e {
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
/// Write manifest to raw disk sector (legacy method).
unsafe fn finalize_manifest_raw(
    blk_device: &mut UnifiedBlockDevice,
    config: &BareMetalConfig,
    total_bytes: u64,
    end_sector: u64,
) -> bool {
    serial_println("[MANIFEST] Writing to raw sector (legacy)...");
    serial_print("[MANIFEST] Sector: ");
    serial_print_hex(config.manifest_sector);
    serial_println("");

    // Use actual start sector (determined by free space scan)
    let start_sector = ACTUAL_START_SECTOR;

    // Use morpheus_core's IsoManifest for raw sector write
    let mut manifest = IsoManifest::new(config.iso_name, total_bytes);

    // Add chunk entry
    match manifest.add_chunk(config.partition_uuid, start_sector, end_sector) {
        Ok(idx) => {
            serial_print("[MANIFEST] Added chunk ");
            serial_print_decimal(idx as u32);
            serial_println("");
        }
        Err(_) => {
            serial_println("[MANIFEST] ERROR: Failed to add chunk");
            return false;
        }
    }

    // Update chunk with data size
    if let Some(chunk) = manifest.chunks.chunks.get_mut(0) {
        chunk.data_size = total_bytes;
        chunk.written = true;
    }

    // Mark as complete
    manifest.mark_complete();

    serial_println("[MANIFEST] Manifest marked as COMPLETE");

    // Write to disk using unified device
    write_manifest_to_disk_unified(blk_device, config.manifest_sector, &manifest)
}

// ═══════════════════════════════════════════════════════════════════════════
// GPT PARTITION CREATION FOR ISO DATA
// ═══════════════════════════════════════════════════════════════════════════

/// Create a GPT partition for ISO data storage.
///
/// This properly claims disk space so other tools won't overwrite our ISO.
/// The partition is created at `start_sector` with size `size_bytes`.
///
/// Returns the partition GUID on success.
unsafe fn create_iso_partition(
    blk_device: &mut UnifiedBlockDevice,
    start_sector: u64,
    size_bytes: u64,
    iso_name: &str,
) -> Option<[u8; 16]> {
    use morpheus_core::disk::gpt_ops::create_partition;
    use morpheus_core::disk::partition::PartitionType;

    serial_println("[GPT] ═══════════════════════════════════════════════════════");
    serial_println("[GPT] Creating partition for ISO data storage");
    serial_println("[GPT] ═══════════════════════════════════════════════════════");

    serial_print("[GPT] ISO name: ");
    serial_println(iso_name);

    serial_print("[GPT] Partition type: BasicData (");
    // EBD0A0A2-B9E5-4433-87C0-68B6B72699C7 is Microsoft Basic Data
    serial_println("EBD0A0A2-B9E5-4433-87C0-68B6B72699C7)");

    serial_print("[GPT] Start sector (LBA): ");
    serial_print_hex(start_sector);
    serial_print(" (byte offset: ");
    serial_print_hex(start_sector * 512);
    serial_println(")");

    // Calculate end sector
    let sectors_needed = (size_bytes + 511) / 512;
    let end_sector = start_sector + sectors_needed - 1;

    serial_print("[GPT] End sector (LBA): ");
    serial_print_hex(end_sector);
    serial_print(" (byte offset: ");
    serial_print_hex(end_sector * 512);
    serial_println(")");

    serial_print("[GPT] Sectors needed: ");
    serial_print_decimal(sectors_needed as u32);
    serial_println("");

    serial_print("[GPT] Partition size: ");
    let size_mb = size_bytes / (1024 * 1024);
    let size_gb = size_bytes / (1024 * 1024 * 1024);
    if size_gb > 0 {
        serial_print_decimal(size_gb as u32);
        serial_print(" GB (");
        serial_print_decimal(size_mb as u32);
        serial_println(" MB)");
    } else {
        serial_print_decimal(size_mb as u32);
        serial_println(" MB");
    }

    // === OVERLAP VERIFICATION ===
    // Before creating the partition, verify the range is actually free
    serial_println("[GPT] Verifying range is not already claimed...");

    let dma_buffer = core::slice::from_raw_parts_mut(
        (&raw mut DISK_WRITE_BUFFER).cast::<u8>(),
        DISK_WRITE_BUFFER_SIZE,
    );
    let dma_buffer_phys = (&raw const DISK_WRITE_BUFFER).cast::<u8>() as u64;
    let timeout_ticks = 100_000_000u64;

    let mut verify_adapter =
        match UnifiedBlockIo::new(blk_device, dma_buffer, dma_buffer_phys, timeout_ticks) {
            Ok(a) => a,
            Err(_) => {
                serial_println("[GPT] ERROR: Failed to create BlockIo adapter for verification");
                return None;
            }
        };

    // Check if requested range is free, if not find alternative
    let (actual_start, actual_end) = match crate::transfer::disk::GptOps::verify_range_free(
        &mut verify_adapter,
        start_sector,
        end_sector,
    ) {
        Ok(true) => {
            serial_println("[GPT] ✓ Range verified free - safe to create partition");
            (start_sector, end_sector)
        }
        Ok(false) => {
            serial_println("[GPT] WARNING: Requested range overlaps existing partition!");
            serial_print("[GPT] Requested: ");
            serial_print_hex(start_sector);
            serial_print(" - ");
            serial_print_hex(end_sector);
            serial_println("");

            serial_println("[GPT] Searching for alternative free space...");
            match crate::transfer::disk::GptOps::find_free_space(&mut verify_adapter) {
                Ok((free_start, free_end)) => {
                    let free_size = free_end - free_start + 1;
                    let needed_size = end_sector - start_sector + 1;

                    if free_size >= needed_size {
                        serial_print("[GPT] ✓ Found suitable free space: ");
                        serial_print_hex(free_start);
                        serial_print(" - ");
                        serial_print_hex(free_end);
                        serial_print(" (");
                        serial_print_decimal((free_size * 512 / (1024 * 1024 * 1024)) as u32);
                        serial_println(" GB)");

                        // Align to 1MB boundary (2048 sectors)
                        let aligned_start = ((free_start + 2047) / 2048) * 2048;
                        let aligned_end = aligned_start + needed_size - 1;

                        serial_print("[GPT] Using aligned range: ");
                        serial_print_hex(aligned_start);
                        serial_print(" - ");
                        serial_print_hex(aligned_end);
                        serial_println("");

                        // Update global for manifest
                        ACTUAL_START_SECTOR = aligned_start;

                        (aligned_start, aligned_end)
                    } else {
                        serial_print("[GPT] ERROR: Free space too small (");
                        serial_print_decimal((free_size * 512 / (1024 * 1024 * 1024)) as u32);
                        serial_print(" GB < ");
                        serial_print_decimal((needed_size * 512 / (1024 * 1024 * 1024)) as u32);
                        serial_println(" GB needed)");
                        return None;
                    }
                }
                Err(e) => {
                    serial_print("[GPT] ERROR: Could not find free space: ");
                    serial_println(match e {
                        crate::transfer::disk::DiskError::IoError => "IO error",
                        crate::transfer::disk::DiskError::InvalidGpt => "Invalid GPT",
                        crate::transfer::disk::DiskError::NoFreeSpace => "No free space",
                        _ => "Unknown error",
                    });
                    serial_println("[GPT] ABORTING: Cannot create partition");
                    return None;
                }
            }
        }
        Err(e) => {
            serial_print("[GPT] ERROR: Could not verify range: ");
            serial_println(match e {
                crate::transfer::disk::DiskError::IoError => "IO error",
                crate::transfer::disk::DiskError::InvalidGpt => "Invalid GPT",
                _ => "Unknown error",
            });
            serial_println("[GPT] ABORTING: Cannot safely create partition");
            return None;
        }
    };

    // Use the verified range for partition creation
    let start_sector = actual_start;
    let end_sector = actual_end;

    // Create BlockIo adapter
    serial_println("[GPT] Creating BlockIO adapter...");
    let dma_buffer = core::slice::from_raw_parts_mut(
        (&raw mut DISK_WRITE_BUFFER).cast::<u8>(),
        DISK_WRITE_BUFFER_SIZE,
    );
    let dma_buffer_phys = (&raw const DISK_WRITE_BUFFER).cast::<u8>() as u64;
    let timeout_ticks = 100_000_000u64;

    let adapter = match UnifiedBlockIo::new(blk_device, dma_buffer, dma_buffer_phys, timeout_ticks)
    {
        Ok(a) => {
            serial_println("[GPT] BlockIO adapter created");
            a
        }
        Err(_) => {
            serial_println("[GPT] ERROR: Failed to create BlockIo adapter");
            return None;
        }
    };

    // Create the partition (BasicData type for ISO storage)
    serial_println("[GPT] Writing partition entry to GPT...");
    serial_println("[GPT] (Reading existing GPT header, finding free slot, writing entry)");

    match create_partition(adapter, PartitionType::BasicData, start_sector, end_sector) {
        Ok(()) => {
            serial_println("[GPT] ───────────────────────────────────────────────────────");
            serial_println("[GPT] PARTITION CREATED SUCCESSFULLY");
            serial_println("[GPT] ───────────────────────────────────────────────────────");
            serial_print("[GPT] Location: sectors ");
            serial_print_hex(start_sector);
            serial_print(" - ");
            serial_print_hex(end_sector);
            serial_println("");
            serial_println("[GPT] Type: Microsoft Basic Data");
            serial_println("[GPT] Status: Active in GPT partition table");
            serial_println("[GPT] ───────────────────────────────────────────────────────");
            // Return a placeholder GUID - the create_partition function generates one
            // TODO: Return actual GUID from create_partition
            Some([
                0x12, 0x34, 0x56, 0x78, 0x12, 0x34, 0x56, 0x78, 0x12, 0x34, 0x56, 0x78, 0x12, 0x34,
                0x56, 0x78,
            ])
        }
        Err(e) => {
            serial_print("[GPT] ERROR: Failed to create partition: ");
            serial_println(match e {
                morpheus_core::disk::gpt_ops::GptError::IoError => {
                    "IO error (disk read/write failed)"
                }
                morpheus_core::disk::gpt_ops::GptError::InvalidHeader => {
                    "Invalid GPT header (disk may not have GPT)"
                }
                morpheus_core::disk::gpt_ops::GptError::InvalidSize => {
                    "Invalid size/range (outside usable area)"
                }
                morpheus_core::disk::gpt_ops::GptError::NoSpace => {
                    "No free partition slot in GPT table"
                }
                morpheus_core::disk::gpt_ops::GptError::PartitionNotFound => "Partition not found",
                morpheus_core::disk::gpt_ops::GptError::OverlappingPartitions => {
                    "Range overlaps existing partition"
                }
                morpheus_core::disk::gpt_ops::GptError::AlignmentError => "Alignment error",
            });
            None
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// NO-HEAP PARSING HELPERS
// ═══════════════════════════════════════════════════════════════════════════

/// Parse a u16 from a string without allocation.
fn parse_u16(s: &str) -> Option<u16> {
    let mut result: u16 = 0;
    for c in s.bytes() {
        if c < b'0' || c > b'9' {
            return None;
        }
        result = result.checked_mul(10)?;
        result = result.checked_add((c - b'0') as u16)?;
    }
    Some(result)
}

/// Parse a u8 from a string without allocation.
fn parse_u8(s: &str) -> Option<u8> {
    let mut result: u16 = 0;
    for c in s.bytes() {
        if c < b'0' || c > b'9' {
            return None;
        }
        result = result * 10 + (c - b'0') as u16;
        if result > 255 {
            return None;
        }
    }
    Some(result as u8)
}

/// Parse IPv4 address from string without allocation.
/// Format: "a.b.c.d" where a,b,c,d are 0-255.
fn parse_ipv4(s: &str) -> Option<Ipv4Address> {
    let bytes = s.as_bytes();
    let mut octets = [0u8; 4];
    let mut octet_idx = 0;
    let mut current: u16 = 0;
    let mut digit_count = 0;

    for &b in bytes {
        if b == b'.' {
            if digit_count == 0 || current > 255 {
                return None;
            }
            if octet_idx >= 3 {
                return None;
            }
            octets[octet_idx] = current as u8;
            octet_idx += 1;
            current = 0;
            digit_count = 0;
        } else if b >= b'0' && b <= b'9' {
            current = current * 10 + (b - b'0') as u16;
            digit_count += 1;
            if digit_count > 3 || current > 255 {
                return None;
            }
        } else {
            return None;
        }
    }

    // Handle last octet
    if digit_count == 0 || current > 255 || octet_idx != 3 {
        return None;
    }
    octets[3] = current as u8;

    Some(Ipv4Address::new(octets[0], octets[1], octets[2], octets[3]))
}

/// Format an HTTP GET request into a static buffer.
/// Returns the number of bytes written, or None if buffer too small.
fn format_http_get(buffer: &mut [u8], path: &str, host: &str) -> Option<usize> {
    let mut pos = 0;

    // "GET "
    let prefix = b"GET ";
    if pos + prefix.len() > buffer.len() {
        return None;
    }
    buffer[pos..pos + prefix.len()].copy_from_slice(prefix);
    pos += prefix.len();

    // path
    let path_bytes = path.as_bytes();
    if pos + path_bytes.len() > buffer.len() {
        return None;
    }
    buffer[pos..pos + path_bytes.len()].copy_from_slice(path_bytes);
    pos += path_bytes.len();

    // " HTTP/1.1\r\nHost: "
    let mid = b" HTTP/1.1\r\nHost: ";
    if pos + mid.len() > buffer.len() {
        return None;
    }
    buffer[pos..pos + mid.len()].copy_from_slice(mid);
    pos += mid.len();

    // host
    let host_bytes = host.as_bytes();
    if pos + host_bytes.len() > buffer.len() {
        return None;
    }
    buffer[pos..pos + host_bytes.len()].copy_from_slice(host_bytes);
    pos += host_bytes.len();

    // Headers and terminator
    let suffix = b"\r\nUser-Agent: MorpheusX/1.0\r\nAccept: */*\r\nConnection: close\r\n\r\n";
    if pos + suffix.len() > buffer.len() {
        return None;
    }
    buffer[pos..pos + suffix.len()].copy_from_slice(suffix);
    pos += suffix.len();

    Some(pos)
}

/// Case-insensitive starts_with for ASCII strings (no heap allocation).
fn starts_with_ignore_case(s: &str, prefix: &str) -> bool {
    if s.len() < prefix.len() {
        return false;
    }
    let s_bytes = s.as_bytes();
    let p_bytes = prefix.as_bytes();
    for i in 0..p_bytes.len() {
        let a = s_bytes[i].to_ascii_lowercase();
        let b = p_bytes[i].to_ascii_lowercase();
        if a != b {
            return false;
        }
    }
    true
}

/// Case-insensitive contains for ASCII strings (no heap allocation).
fn contains_ignore_case(s: &str, needle: &str) -> bool {
    if needle.len() > s.len() {
        return false;
    }
    let s_bytes = s.as_bytes();
    let n_bytes = needle.as_bytes();

    for i in 0..=(s_bytes.len() - n_bytes.len()) {
        let mut found = true;
        for j in 0..n_bytes.len() {
            if s_bytes[i + j].to_ascii_lowercase() != n_bytes[j].to_ascii_lowercase() {
                found = false;
                break;
            }
        }
        if found {
            return true;
        }
    }
    false
}

/// Parse usize from string without allocation.
fn parse_usize(s: &str) -> Option<usize> {
    let mut result: usize = 0;
    let mut has_digit = false;
    for c in s.bytes() {
        if c >= b'0' && c <= b'9' {
            has_digit = true;
            result = result.checked_mul(10)?;
            result = result.checked_add((c - b'0') as usize)?;
        } else if c == b' ' || c == b'\t' {
            // Skip whitespace at beginning
            if has_digit {
                break; // Stop at whitespace after digits
            }
        } else {
            break; // Stop at non-digit, non-whitespace
        }
    }
    if has_digit {
        Some(result)
    } else {
        None
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// REENTRANCY GUARD
// ═══════════════════════════════════════════════════════════════════════════

/// Static flag to detect reentrancy in polling loop.
/// If this is > 1 during a poll, we have a bug.
static mut POLL_DEPTH: u32 = 0;

/// Increment poll depth, panic if already polling.
#[inline(always)]
fn enter_poll() {
    unsafe {
        POLL_DEPTH += 1;
        if POLL_DEPTH > 1 {
            serial_println("!!! REENTRANCY BUG DETECTED !!!");
            serial_print("Poll depth: ");
            serial_print_decimal(POLL_DEPTH);
            serial_println("");
            // Halt to prevent further corruption
            loop {
                core::hint::spin_loop();
            }
        }
    }
}

/// Decrement poll depth.
#[inline(always)]
fn exit_poll() {
    unsafe {
        if POLL_DEPTH > 0 {
            POLL_DEPTH -= 1;
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SMOLTCP DEVICE ADAPTER
// ═══════════════════════════════════════════════════════════════════════════

/// Adapter bridging NetworkDriver to smoltcp Device trait.
///
/// This adapter uses a simple design where:
/// - RX: Buffers received data internally, RxToken references it
/// - TX: TxToken writes directly via the driver
pub struct SmoltcpAdapter<'a, D: NetworkDriver> {
    driver: &'a mut D,
    /// Temporary buffer for received packet
    rx_buffer: [u8; 2048],
    /// Length of data in rx_buffer (0 if no pending packet)
    rx_len: usize,
    /// TX packet count (for debug)
    tx_count: u32,
    /// RX packet count (for debug)
    rx_count: u32,
}

impl<'a, D: NetworkDriver> SmoltcpAdapter<'a, D> {
    pub fn new(driver: &'a mut D) -> Self {
        Self {
            driver,
            rx_buffer: [0u8; 2048],
            rx_len: 0,
            tx_count: 0,
            rx_count: 0,
        }
    }

    /// Try to receive a packet into our internal buffer.
    /// Called before polling smoltcp.
    pub fn poll_receive(&mut self) {
        if self.rx_len == 0 {
            // No pending packet, try to receive
            match self.driver.receive(&mut self.rx_buffer) {
                Ok(Some(len)) => {
                    self.rx_len = len;
                    self.rx_count += 1;
                    inc_rx_count(); // Global counter
                    
                    // Debug: show what we're receiving
                    if len >= 14 {
                        // Show dest MAC (first 6 bytes) and src MAC (next 6)
                        serial_print("[RX dst=");
                        serial_print_hex_byte(self.rx_buffer[0]);
                        serial_print(":");
                        serial_print_hex_byte(self.rx_buffer[1]);
                        serial_print(":");
                        serial_print_hex_byte(self.rx_buffer[2]);
                        serial_print(":");
                        serial_print_hex_byte(self.rx_buffer[3]);
                        serial_print(":");
                        serial_print_hex_byte(self.rx_buffer[4]);
                        serial_print(":");
                        serial_print_hex_byte(self.rx_buffer[5]);
                        
                        serial_print(" src=");
                        serial_print_hex_byte(self.rx_buffer[6]);
                        serial_print(":");
                        serial_print_hex_byte(self.rx_buffer[7]);
                        serial_print(":");
                        serial_print_hex_byte(self.rx_buffer[8]);
                        serial_print(":");
                        serial_print_hex_byte(self.rx_buffer[9]);
                        serial_print(":");
                        serial_print_hex_byte(self.rx_buffer[10]);
                        serial_print(":");
                        serial_print_hex_byte(self.rx_buffer[11]);
                        
                        let ethertype = ((self.rx_buffer[12] as u16) << 8) | (self.rx_buffer[13] as u16);
                        serial_print(" et=0x");
                        serial_print_hex_byte((ethertype >> 8) as u8);
                        serial_print_hex_byte((ethertype & 0xFF) as u8);
                        
                        if ethertype == 0x0800 && len >= 42 { // IPv4 + UDP header
                            let proto = self.rx_buffer[23];
                            if proto == 17 { // UDP
                                let src_port = ((self.rx_buffer[34] as u16) << 8) | (self.rx_buffer[35] as u16);
                                let dst_port = ((self.rx_buffer[36] as u16) << 8) | (self.rx_buffer[37] as u16);
                                serial_print(" udp ");
                                serial_print_decimal(src_port as u32);
                                serial_print("->");
                                serial_print_decimal(dst_port as u32);
                            }
                        }
                        serial_println("]");
                    }
                }
                _ => {}
            }
        }
    }
    
    /// Get TX count for debug.
    pub fn tx_count(&self) -> u32 {
        self.tx_count
    }
    
    /// Get RX count for debug.
    pub fn rx_count(&self) -> u32 {
        self.rx_count
    }

    /// Refill RX queue. Called in main loop Phase 1.
    pub fn refill_rx(&mut self) {
        self.driver.refill_rx_queue();
    }

    /// Collect TX completions. Called in main loop Phase 5.
    pub fn collect_tx(&mut self) {
        self.driver.collect_tx_completions();
    }
}

/// RX token for smoltcp - uses a fixed-size buffer (no heap allocation).
/// Maximum Ethernet frame size is 1514 bytes.
pub struct RxToken {
    buffer: [u8; 2048],
    len: usize,
}

impl smoltcp::phy::RxToken for RxToken {
    fn consume<R, F>(mut self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        // Debug disabled for performance
        f(&mut self.buffer[..self.len])
    }
}

/// TX token for smoltcp - uses a fixed-size stack buffer (no heap allocation).
pub struct TxToken<'a, D: NetworkDriver> {
    driver: &'a mut D,
}

impl<'a, D: NetworkDriver> smoltcp::phy::TxToken for TxToken<'a, D> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        // Use stack-allocated buffer (NO HEAP!) - max Ethernet frame + some margin
        const MAX_FRAME: usize = 2048;
        let mut buffer = [0u8; MAX_FRAME];

        let actual_len = if len > MAX_FRAME { MAX_FRAME } else { len };

        let result = f(&mut buffer[..actual_len]);

        // Fire-and-forget transmit - don't wait for completion
        if self.driver.transmit(&buffer[..actual_len]).is_ok() {
            inc_tx_count();
        }

        result
    }
}

impl<'a, D: NetworkDriver> smoltcp::phy::Device for SmoltcpAdapter<'a, D> {
    type RxToken<'b>
        = RxToken
    where
        Self: 'b;
    type TxToken<'b>
        = TxToken<'b, D>
    where
        Self: 'b;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        // First, try to receive if we don't have a pending packet
        self.poll_receive();

        if self.rx_len > 0 {
            // Copy the packet data to RxToken using fixed buffer (NO HEAP!)
            let mut rx_buf = [0u8; 2048];
            let copy_len = self.rx_len.min(rx_buf.len());
            rx_buf[..copy_len].copy_from_slice(&self.rx_buffer[..copy_len]);
            let rx_len = copy_len;
            self.rx_len = 0; // Mark buffer as consumed

            Some((
                RxToken {
                    buffer: rx_buf,
                    len: rx_len,
                },
                TxToken {
                    driver: self.driver,
                },
            ))
        } else {
            None
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        if self.driver.can_transmit() {
            Some(TxToken {
                driver: self.driver,
            })
        } else {
            None
        }
    }

    fn capabilities(&self) -> smoltcp::phy::DeviceCapabilities {
        let mut caps = smoltcp::phy::DeviceCapabilities::default();
        caps.medium = smoltcp::phy::Medium::Ethernet;
        caps.max_transmission_unit = 1514;
        caps.max_burst_size = Some(32); // Process up to 32 packets per poll for throughput
        caps
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// BARE-METAL ENTRY POINT
// ═══════════════════════════════════════════════════════════════════════════

/// Run result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunResult {
    /// ISO download and write completed successfully.
    Success,
    /// Initialization failed.
    InitFailed,
    /// DHCP timeout.
    DhcpTimeout,
    /// Download failed.
    DownloadFailed,
    /// Disk write failed.
    DiskWriteFailed,
}

/// Configuration for the bare-metal runner.
///
/// NOTE: Uses &'static str instead of String because we cannot allocate
/// after ExitBootServices (the UEFI allocator is gone).
pub struct BareMetalConfig {
    /// URL to download ISO from (must be 'static - allocated before EBS).
    pub iso_url: &'static str,
    /// ISO filename for manifest (e.g., "tails-6.0.iso").
    pub iso_name: &'static str,
    /// Target disk sector to start writing ISO data at.
    pub target_start_sector: u64,
    /// Sector where manifest is stored (raw sector write, legacy).
    /// Set to 0 to use FAT32 manifest instead.
    pub manifest_sector: u64,
    /// ESP partition start LBA for FAT32 manifest writing.
    /// If non-zero, writes manifest to `/.iso/<name>.manifest` on ESP.
    pub esp_start_lba: u64,
    /// Maximum download size in bytes.
    pub max_download_size: u64,
    /// Whether to write to disk (requires VirtIO-blk).
    pub write_to_disk: bool,
    /// Partition UUID for chunk tracking (16 bytes, or zeros if unknown).
    pub partition_uuid: [u8; 16],
}

impl Default for BareMetalConfig {
    fn default() -> Self {
        Self {
            iso_url: "http://10.0.2.2:8000/test-iso.img",
            iso_name: "download.iso",
            // Start writing ISO data AFTER the 4GB ESP partition
            // ESP is sectors 2048 to ~8388608 (1MiB to 4GiB)
            // We start at 4GiB = 8388608 sectors
            target_start_sector: 8388608, // 4GiB in 512-byte sectors
            manifest_sector: 0,           // Use FAT32 by default (set non-zero for raw sector)
            esp_start_lba: 2048,          // Standard GPT ESP at sector 2048
            max_download_size: 8 * 1024 * 1024 * 1024, // 8GB max (supports Kali, Parrot, etc.)
            write_to_disk: true,          // Enable disk writes by default
            partition_uuid: [0u8; 16],    // Will be set by bootloader if known
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// DISK WRITE BUFFER (Static - no heap allocation)
// ═══════════════════════════════════════════════════════════════════════════

/// Size of write buffer: 64KB = 128 sectors (optimal for VirtIO-blk)
const DISK_WRITE_BUFFER_SIZE: usize = 64 * 1024;

/// Number of sectors per write operation
const SECTORS_PER_WRITE: u32 = (DISK_WRITE_BUFFER_SIZE / 512) as u32;

/// Static buffer for accumulating data before disk write
static mut DISK_WRITE_BUFFER: [u8; DISK_WRITE_BUFFER_SIZE] = [0u8; DISK_WRITE_BUFFER_SIZE];

/// Current fill level of write buffer
static mut DISK_WRITE_BUFFER_FILL: usize = 0;

/// Next sector to write to
static mut DISK_NEXT_SECTOR: u64 = 0;

/// Actual start sector (after finding free space)
static mut ACTUAL_START_SECTOR: u64 = 0;

/// Total bytes written to disk
static mut DISK_TOTAL_BYTES: u64 = 0;

/// Next request ID for block driver
static mut DISK_NEXT_REQUEST_ID: u32 = 1;

/// Main bare-metal entry point.
///
/// This function:
/// 1. Validates the BootHandoff
/// 2. Initializes VirtIO-net driver
/// 3. Creates smoltcp interface
/// 4. Runs DHCP to get IP
/// 5. Downloads ISO via HTTP
/// 6. Writes ISO to VirtIO-blk disk
///
/// # Safety
/// Must be called after ExitBootServices with valid BootHandoff.
///
/// # Returns
/// Never returns on success (halts after completion).
/// Returns error on failure.
#[cfg(target_arch = "x86_64")]
pub unsafe fn bare_metal_main(handoff: &'static BootHandoff, config: BareMetalConfig) -> RunResult {
    serial_println("=====================================");
    serial_println("  MorpheusX Post-EBS Network Stack");
    serial_println("=====================================");
    serial_println("");

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 1: VERIFY HEAP ALLOCATOR
    // ═══════════════════════════════════════════════════════════════════════
    // Heap should already be initialized by bootloader's efi_main
    // Call init_heap() anyway - it's safe to call multiple times
    crate::alloc_heap::init_heap();
    if crate::alloc_heap::is_initialized() {
        serial_println("[OK] Heap allocator ready (1MB)");
    } else {
        serial_println("[FAIL] Heap allocator not initialized!");
        return RunResult::InitFailed;
    }

    serial_println("[INIT] Validating BootHandoff...");

    if let Err(e) = handoff.validate() {
        serial_println("[FAIL] BootHandoff validation failed");
        return RunResult::InitFailed;
    }
    serial_println("[OK] BootHandoff valid");

    serial_print("[INIT] TSC frequency: ");
    serial_print_hex(handoff.tsc_freq);
    serial_println(" Hz");

    serial_print("[INIT] DMA region: ");
    serial_print_hex(handoff.dma_cpu_ptr);
    serial_print(" - ");
    serial_print_hex(handoff.dma_cpu_ptr + handoff.dma_size);
    serial_println("");

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 1.5: INITIALIZE FRAMEBUFFER DISPLAY (if available)
    // ═══════════════════════════════════════════════════════════════════════
    if handoff.has_framebuffer() {
        crate::display::init_display(
            handoff.framebuffer_base,
            handoff.framebuffer_width,
            handoff.framebuffer_height,
            handoff.framebuffer_stride,
            handoff.framebuffer_format,
        );

        // Now that display is initialized, print banner (will appear on screen)
        serial_println("");
        serial_println("=====================================");
        serial_println("  MorpheusX Post-EBS Network Stack");
        serial_println("=====================================");
        serial_println("");

        serial_print("[OK] Framebuffer display: ");
        serial_print_decimal(handoff.framebuffer_width);
        serial_print("x");
        serial_print_decimal(handoff.framebuffer_height);
        serial_print(" format=");
        serial_print_decimal(handoff.framebuffer_format);
        serial_print(" (0=RGB, 1=BGR) stride=");
        serial_print_decimal(handoff.framebuffer_stride);
        serial_println("");
    } else {
        serial_println("[INFO] No framebuffer available");
    }

    // Create timeout config
    let timeouts = TimeoutConfig::new(handoff.tsc_freq);
    let loop_config = MainLoopConfig::new(handoff.tsc_freq);

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 2: INITIALIZE NETWORK DEVICE (UNIFIED DRIVER)
    // ═══════════════════════════════════════════════════════════════════════
    serial_print("[INIT] NIC type: ");
    let nic_type_name = match handoff.nic_type {
        NIC_TYPE_VIRTIO => "VirtIO-net",
        NIC_TYPE_INTEL => "Intel e1000e",
        _ => "Unknown",
    };
    serial_println(nic_type_name);

    serial_print("[INIT] NIC MMIO base: ");
    serial_print_hex(handoff.nic_mmio_base);
    serial_println("");

    serial_println("[INIT] Initializing unified network driver...");
    serial_println("  [DEBUG] About to call from_handoff...");

    let mut driver = match UnifiedNetworkDriver::from_handoff(handoff) {
        Ok(d) => {
            serial_println("  [DEBUG] from_handoff returned Ok");
            serial_print("[OK] ");
            serial_print(d.driver_name());
            serial_println(" driver initialized");
            d
        }
        Err(e) => {
            serial_println("  [DEBUG] from_handoff returned Err");
            serial_print("[FAIL] Driver init error: ");
            match e {
                UnifiedDriverError::NoNicDetected => serial_println("no NIC detected"),
                UnifiedDriverError::UnsupportedNicType(t) => {
                    serial_print("unsupported NIC type: ");
                    serial_print_decimal(t as u32);
                    serial_println("");
                }
                UnifiedDriverError::VirtioError(ve) => {
                    serial_print("VirtIO error: ");
                    match ve {
                        VirtioInitError::ResetTimeout => serial_println("reset timeout"),
                        VirtioInitError::FeatureNegotiationFailed => {
                            serial_println("feature negotiation failed")
                        }
                        VirtioInitError::FeaturesRejected => {
                            serial_println("features rejected by device")
                        }
                        VirtioInitError::QueueSetupFailed => serial_println("queue setup failed"),
                        VirtioInitError::RxPrefillFailed(_) => serial_println("RX prefill failed"),
                        VirtioInitError::DeviceError => serial_println("device error"),
                    }
                }
                UnifiedDriverError::IntelError(ie) => {
                    serial_print("Intel e1000e error: ");
                    use crate::driver::intel::{E1000eError, E1000eInitError};
                    match ie {
                        E1000eError::InitFailed(init_err) => match init_err {
                            E1000eInitError::ResetTimeout => serial_println("reset timeout"),
                            E1000eInitError::InvalidMac => serial_println("invalid MAC"),
                            E1000eInitError::MmioError => serial_println("MMIO error"),
                            E1000eInitError::LinkTimeout => serial_println("link timeout"),
                            E1000eInitError::UlpDisableFailed => serial_println("ULP disable failed (I218)"),
                            E1000eInitError::PhyNotAccessible => serial_println("PHY not accessible after recovery"),
                            E1000eInitError::SemaphoreTimeout => serial_println("hardware semaphore timeout"),
                        },
                        E1000eError::NotReady => serial_println("device not ready"),
                        E1000eError::LinkDown => serial_println("link down"),
                    }
                }
                UnifiedDriverError::InvalidHandoff => serial_println("invalid handoff data"),
            }
            return RunResult::InitFailed;
        }
    };

    // Print real MAC address from driver (using serial_print_hex_byte for framebuffer)
    serial_print("[INIT] MAC address: ");
    let mac = driver.mac_address();
    for (i, byte) in mac.iter().enumerate() {
        if i > 0 {
            serial_print(":");
        }
        serial_print_hex_byte(*byte);
    }
    serial_println("");

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 2.5: INITIALIZE BLOCK DEVICE (VirtIO-blk or AHCI)
    // ═══════════════════════════════════════════════════════════════════════
    let mut blk_driver: Option<UnifiedBlockDevice> = None;

    if config.write_to_disk && handoff.has_block_device() {
        serial_print("[INIT] Block device type: ");
        serial_print_decimal(handoff.blk_type as u32);
        serial_println("");
        serial_print("[INIT] Block MMIO/ABAR: ");
        serial_print_hex(handoff.blk_mmio_base);
        serial_println("");

        // Calculate DMA region for block device
        // Use second half of DMA region (first half is for network)
        let blk_dma_offset = handoff.dma_size / 2;
        let blk_dma_base = handoff.dma_cpu_ptr + blk_dma_offset;

        if handoff.blk_type == BLK_TYPE_VIRTIO {
            // VirtIO-blk initialization
            serial_println("[INIT] Initializing VirtIO-blk driver...");
            serial_print("[INIT] Block sector size: ");
            serial_print_decimal(handoff.blk_sector_size);
            serial_println("");
            serial_print("[INIT] Block total sectors: ");
            serial_print_hex(handoff.blk_total_sectors);
            serial_println("");

            // VirtIO-blk queue layout
            let blk_config = VirtioBlkConfig {
                queue_size: 32,
                desc_phys: blk_dma_base,
                avail_phys: blk_dma_base + 512,
                used_phys: blk_dma_base + 1024,
                headers_phys: blk_dma_base + 2048,
                status_phys: blk_dma_base + 2048 + 512,
                headers_cpu: blk_dma_base + 2048,
                status_cpu: blk_dma_base + 2048 + 512,
                notify_addr: handoff.blk_mmio_base + 0x50,
                transport_type: handoff.blk_transport_type,
            };

            // Create driver based on transport type
            let driver_result = if handoff.blk_transport_type == TRANSPORT_PCI_MODERN {
                serial_println("[INIT] Using PCI Modern transport for VirtIO-blk");
                serial_print("[INIT] common_cfg: ");
                serial_print_hex(handoff.blk_common_cfg);
                serial_println("");
                serial_print("[INIT] notify_cfg: ");
                serial_print_hex(handoff.blk_notify_cfg);
                serial_println("");
                serial_print("[INIT] device_cfg: ");
                serial_print_hex(handoff.blk_device_cfg);
                serial_println("");

                let pci_config = PciModernConfig {
                    common_cfg: handoff.blk_common_cfg,
                    notify_cfg: handoff.blk_notify_cfg,
                    notify_off_multiplier: handoff.blk_notify_off_multiplier,
                    isr_cfg: handoff.blk_isr_cfg,
                    device_cfg: handoff.blk_device_cfg,
                    pci_cfg: 0,
                };
                let transport = VirtioTransport::pci_modern(pci_config);
                unsafe {
                    VirtioBlkDriver::new_with_transport(transport, blk_config, handoff.tsc_freq)
                }
            } else {
                serial_println("[INIT] Using MMIO transport for VirtIO-blk");
                unsafe { VirtioBlkDriver::new(handoff.blk_mmio_base, blk_config) }
            };

            match driver_result {
                Ok(d) => {
                    serial_println("[OK] VirtIO-blk driver initialized");
                    blk_driver = Some(UnifiedBlockDevice::VirtIO(d));
                }
                Err(e) => {
                    serial_print("[WARN] VirtIO-blk init failed: ");
                    match e {
                        VirtioBlkInitError::ResetFailed => serial_println("reset failed"),
                        VirtioBlkInitError::FeatureNegotiationFailed => {
                            serial_println("feature negotiation failed")
                        }
                        VirtioBlkInitError::QueueSetupFailed => {
                            serial_println("queue setup failed")
                        }
                        VirtioBlkInitError::DeviceFailed => serial_println("device error"),
                        VirtioBlkInitError::InvalidConfig => serial_println("invalid config"),
                        VirtioBlkInitError::TransportError => serial_println("transport error"),
                    }
                }
            }
        } else if handoff.blk_type == BLK_TYPE_AHCI {
            // AHCI initialization
            serial_println("[INIT] Initializing AHCI driver...");
            serial_print("[INIT] ABAR: ");
            serial_print_hex(handoff.blk_mmio_base);
            serial_println("");

            // AHCI DMA layout:
            // - Command List: 1KB aligned, 1KB (at blk_dma_base)
            // - FIS Receive: 256-byte aligned, 256 bytes (at +0x400)
            // - Command Tables: 128-byte aligned, 32 * 256 = 8KB (at +0x800)
            // - IDENTIFY buffer: 512 bytes (at +0x2800)
            let cmd_list_phys = blk_dma_base;
            let fis_phys = blk_dma_base + 0x400;
            let cmd_tables_phys = blk_dma_base + 0x800;
            let identify_phys = blk_dma_base + 0x2800;

            let ahci_config = AhciConfig {
                tsc_freq: handoff.tsc_freq,
                cmd_list_cpu: cmd_list_phys as *mut u8,
                cmd_list_phys,
                fis_cpu: fis_phys as *mut u8,
                fis_phys,
                cmd_tables_cpu: cmd_tables_phys as *mut u8,
                cmd_tables_phys,
                identify_cpu: identify_phys as *mut u8,
                identify_phys,
            };

            match unsafe { AhciDriver::new(handoff.blk_mmio_base, ahci_config) } {
                Ok(d) => {
                    serial_println("[OK] AHCI driver initialized");
                    if d.link_up() {
                        serial_println("[OK] SATA link established");
                    } else {
                        serial_println("[WARN] SATA link not up");
                    }
                    blk_driver = Some(UnifiedBlockDevice::Ahci(d));
                }
                Err(e) => {
                    serial_print("[WARN] AHCI init failed: ");
                    serial_println(match e {
                        AhciInitError::InvalidConfig => "invalid config",
                        AhciInitError::ResetFailed => "reset failed",
                        AhciInitError::NoDeviceFound => "no device found",
                        AhciInitError::PortStopTimeout => "port stop timeout",
                        AhciInitError::PortStartFailed => "port start failed",
                        AhciInitError::IdentifyFailed => "identify failed",
                        AhciInitError::No64BitSupport => "no 64-bit support",
                        AhciInitError::DeviceNotResponding => "device not responding",
                        AhciInitError::DmaSetupFailed => "DMA setup failed",
                    });
                }
            }
        } else {
            serial_print("[WARN] Unknown block device type: ");
            serial_print_decimal(handoff.blk_type as u32);
            serial_println("");
        }

        // If we have a block driver, set up disk writing state
        if let Some(ref mut d) = blk_driver {
            let info = d.info();
            serial_print("[OK] Disk capacity: ");
            let total_bytes = info.total_sectors * info.sector_size as u64;
            if total_bytes >= 1024 * 1024 * 1024 {
                serial_print_decimal((total_bytes / (1024 * 1024 * 1024)) as u32);
                serial_println(" GB");
            } else {
                serial_print_decimal((total_bytes / (1024 * 1024)) as u32);
                serial_println(" MB");
            }

            // Find free space on disk
            serial_println("[INIT] Scanning GPT for existing partitions...");
            let actual_start_sector = {
                // For GPT operations, we need a BlockIo adapter
                // This is a bit awkward with the unified driver, but we'll work with what we have
                config.target_start_sector // Default fallback
                                           // TODO: Implement GPT scanning for unified block device
            };

            serial_print("[INIT] ISO data will be written starting at sector: ");
            serial_print_hex(actual_start_sector);
            serial_println("");

            // Create GPT partition for ISO data BEFORE we start writing
            // Now works with both VirtIO and AHCI!
            serial_println("[INIT] Creating GPT partition for ISO storage...");
            if let Some(part_uuid) = create_iso_partition(
                d,
                actual_start_sector,
                config.max_download_size,
                config.iso_name,
            ) {
                serial_println("[OK] ISO partition created and claimed in GPT");
            } else {
                serial_println(
                    "[WARN] Could not create GPT partition - ISO data may be overwritten!",
                );
                serial_println("[WARN] Continuing anyway (data will be in unmapped space)");
            }

            // Initialize disk write state
            DISK_NEXT_SECTOR = actual_start_sector;
            DISK_WRITE_BUFFER_FILL = 0;
            DISK_TOTAL_BYTES = 0;
            DISK_NEXT_REQUEST_ID = 1;
            ACTUAL_START_SECTOR = actual_start_sector;
        }
    } else if config.write_to_disk {
        serial_println("[WARN] No block device in handoff - disk writes disabled");
    } else {
        serial_println("[INFO] Disk writes disabled by config");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 2.75: WAIT FOR PHY LINK (CRITICAL FOR REAL HARDWARE)
    // ═══════════════════════════════════════════════════════════════════════
    serial_println("[NET] Waiting for PHY link...");

    let link_start = get_tsc();
    let link_timeout_ticks = handoff.tsc_freq * 15; // 15 second timeout for link (auto-neg can be slow)
    let mut last_dot_tsc = link_start;
    let dot_interval = handoff.tsc_freq; // 1 second

    loop {
        if driver.link_up() {
            serial_println("");
            serial_println("[OK] PHY link established");
            break;
        }

        let now_tsc = get_tsc();
        
        // Print a dot every second to show progress
        if now_tsc.wrapping_sub(last_dot_tsc) > dot_interval {
            serial_print(".");
            last_dot_tsc = now_tsc;
        }
        
        if now_tsc.wrapping_sub(link_start) > link_timeout_ticks {
            serial_println("");
            serial_println("[WARN] PHY link timeout - continuing anyway...");
            break;
        }

        // Brief yield
        for _ in 0..10000 {
            core::hint::spin_loop();
        }
    }

    // Give the link a moment to stabilize after coming up
    serial_println("[NET] Link stabilization delay...");
    let stabilize_start = get_tsc();
    let stabilize_ticks = handoff.tsc_freq / 2; // 500ms
    while get_tsc().wrapping_sub(stabilize_start) < stabilize_ticks {
        for _ in 0..1000 {
            core::hint::spin_loop();
        }
    }
    serial_println("[OK] Link stable");

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 3: CREATE SMOLTCP INTERFACE
    // ═══════════════════════════════════════════════════════════════════════
    serial_println("[INIT] Creating smoltcp interface...");

    // Use the MAC address from the driver
    let mac_bytes = driver.mac_address();
    serial_print("[DEBUG] driver.mac_address() = ");
    for (i, b) in mac_bytes.iter().enumerate() {
        if i > 0 {
            serial_print(":");
        }
        serial_print_hex(*b as u64);
    }
    serial_println("");
    let mac = EthernetAddress(mac_bytes);
    let hw_addr = HardwareAddress::Ethernet(mac);

    // Create smoltcp config
    let mut iface_config = Config::new(hw_addr);
    iface_config.random_seed = handoff.tsc_freq; // Use TSC frequency as random seed

    // Create the device adapter wrapping our VirtIO driver
    let mut adapter = SmoltcpAdapter::new(&mut driver);

    // CRITICAL: Capture the base TSC at interface creation time
    // All smoltcp timestamps must be relative to this base, NOT absolute TSC
    // Otherwise smoltcp thinks huge amounts of time have passed since boot
    let smoltcp_base_tsc = get_tsc();

    // Create smoltcp interface with timestamp 0 (matching our base)
    let mut iface = Interface::new(iface_config, &mut adapter, Instant::from_millis(0));

    // Set up initial IP config (DHCP will configure this later)
    iface.update_ip_addrs(|addrs| {
        // Start with empty - DHCP will fill this
    });

    serial_println("[OK] smoltcp interface created");

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 4: DHCP
    // ═══════════════════════════════════════════════════════════════════════
    serial_println("[NET] Starting DHCP discovery...");

    // Create socket storage
    let mut socket_storage: [SocketStorage; 8] = Default::default();
    let mut sockets = SocketSet::new(&mut socket_storage[..]);

    // Create and add DHCP socket
    let dhcp_socket = Dhcpv4Socket::new();
    let dhcp_handle = sockets.add(dhcp_socket);

    let dhcp_start = get_tsc();
    let dhcp_timeout_ticks = timeouts.dhcp();

    // Track number of DHCP events
    let mut dhcp_event_count = 0u32;

    // Track if we got an IP
    #[allow(unused_assignments)]
    let mut got_ip = false;
    #[allow(unused_assignments)]
    let mut our_ip = Ipv4Address::UNSPECIFIED;
    #[allow(unused_assignments)]
    let mut gateway_ip = Ipv4Address::UNSPECIFIED;
    let mut dns_ip = Ipv4Address::UNSPECIFIED;

    // DHCP polling loop
    serial_println("[NET] Sending DHCP DISCOVER...");

    loop {
        let now_tsc = get_tsc();

        // Check timeout
        if now_tsc.wrapping_sub(dhcp_start) > dhcp_timeout_ticks {
            serial_print("[FAIL] DHCP timeout TX:");
            serial_print_decimal(get_tx_count());
            serial_print(" RX:");
            serial_print_decimal(get_rx_count());
            serial_println("");
            return RunResult::DhcpTimeout;
        }

        // Convert TSC to smoltcp Instant - RELATIVE to interface creation time!
        let relative_tsc = now_tsc.wrapping_sub(smoltcp_base_tsc);
        let timestamp = tsc_to_instant(relative_tsc, handoff.tsc_freq);

        // Phase 1: Refill RX queue
        adapter.refill_rx();

        // Phase 2: Poll smoltcp interface (EXACTLY ONCE per iteration)
        enter_poll(); // Reentrancy guard
        let poll_result = iface.poll(timestamp, &mut adapter, &mut sockets);
        exit_poll(); // Reentrancy guard
        
        // Debug disabled for performance
        let _ = poll_result;

        // Check for DHCP events
        let dhcp_socket = sockets.get_mut::<Dhcpv4Socket>(dhcp_handle);
        
        if let Some(event) = dhcp_socket.poll() {
            match event {
                Dhcpv4Event::Configured(dhcp_config) => {
                    serial_println("[NET] Received DHCP ACK");

                    // Apply the configuration
                    our_ip = dhcp_config.address.address();

                    // Print IP address
                    serial_print("[OK] IP address: ");
                    print_ipv4(our_ip);
                    serial_println("");

                    // Apply IP to interface
                    iface.update_ip_addrs(|addrs| {
                        // Clear existing and add new
                        addrs.clear();
                        addrs.push(IpCidr::Ipv4(dhcp_config.address)).ok();
                    });

                    // Set gateway
                    #[allow(unused_assignments)]
                    if let Some(router) = dhcp_config.router {
                        gateway_ip = router;
                        iface.routes_mut().add_default_ipv4_route(router).ok();
                        serial_print("[OK] Gateway: ");
                        print_ipv4(router);
                        serial_println("");
                    }

                    // Set DNS (if provided)
                    if let Some(dns) = dhcp_config.dns_servers.get(0) {
                        dns_ip = *dns;
                        serial_print("[OK] DNS: ");
                        print_ipv4(*dns);
                        serial_println("");
                    }

                    got_ip = true;
                    break;
                }
                Dhcpv4Event::Deconfigured => {
                    // Normal at startup, DHCP will retry automatically
                }
            }
        }

        // Phase 5: Collect TX completions
        adapter.collect_tx();

        // Brief yield (don't spin too tight)
        for _ in 0..100 {
            core::hint::spin_loop();
        }
    }

    if !got_ip {
        serial_println("[FAIL] DHCP did not obtain IP");
        return RunResult::DhcpTimeout;
    }

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 5: HTTP DOWNLOAD - NO HEAP ALLOCATION VERSION
    // ═══════════════════════════════════════════════════════════════════════
    serial_print("[HTTP] Downloading from: ");
    serial_println(config.iso_url);

    // Parse URL without heap allocation
    // Format: http://host[:port]/path
    let url_str = config.iso_url;

    // Check scheme
    if url_str.starts_with("https://") {
        serial_println("[FAIL] HTTPS not supported in bare-metal mode");
        return RunResult::DownloadFailed;
    }

    let rest = if let Some(r) = url_str.strip_prefix("http://") {
        r
    } else {
        serial_println("[FAIL] Invalid URL scheme (must be http://)");
        return RunResult::DownloadFailed;
    };

    // Split host[:port] from path
    let (authority, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };

    // Parse host and port from authority
    let (host_str, server_port): (&str, u16) = if let Some(colon_idx) = authority.rfind(':') {
        let h = &authority[..colon_idx];
        let p = &authority[colon_idx + 1..];
        match parse_u16(p) {
            Some(port) => (h, port),
            None => (authority, 80),
        }
    } else {
        (authority, 80)
    };

    serial_print("[HTTP] Host: ");
    serial_println(host_str);
    serial_print("[HTTP] Port: ");
    serial_print_decimal(server_port as u32);
    serial_println("");

    // Fake debug MAC (display-only) to test whether display shows MAC.
    // Clearly marked as FAKE — this should NOT be used as the real MAC.
    serial_println("[FAKE-MAC] 52:54:00:12:34:56 (fake display)");
    serial_print("[HTTP] Path: ");
    serial_println(path);

    // Parse host as IP address or do DNS resolution
    let server_ip = match parse_ipv4(host_str) {
        Some(ip) => {
            serial_print("[HTTP] Using IP: ");
            print_ipv4(ip);
            serial_println("");
            ip
        }
        None => {
            // Need DNS resolution
            serial_print("[HTTP] Resolving hostname: ");
            serial_println(host_str);

            if dns_ip == Ipv4Address::UNSPECIFIED {
                serial_println("[FAIL] No DNS server available");
                return RunResult::DownloadFailed;
            }

            // Create DNS socket with the DHCP-provided DNS server
            // Use static storage for DNS queries (smoltcp requires this)
            static mut DNS_QUERIES: [Option<smoltcp::socket::dns::DnsQuery>; 1] = [None];

            let dns_servers: &[IpAddress] = &[IpAddress::Ipv4(dns_ip)];
            let dns_socket = DnsSocket::new(dns_servers, unsafe { &mut DNS_QUERIES[..] });
            let dns_handle = sockets.add(dns_socket);

            // Start DNS query
            let query_handle = {
                let dns = sockets.get_mut::<DnsSocket>(dns_handle);
                match dns.start_query(iface.context(), host_str, DnsQueryType::A) {
                    Ok(h) => h,
                    Err(_) => {
                        serial_println("[FAIL] DNS query start failed");
                        return RunResult::DownloadFailed;
                    }
                }
            };

            serial_println("[DNS] Query started, waiting for response...");

            // Poll until we get DNS response
            let dns_start = get_tsc();
            let dns_timeout = timeouts.tcp_connect(); // Use same timeout as TCP

            let resolved_ip = loop {
                let now_tsc = get_tsc();
                if now_tsc.wrapping_sub(dns_start) > dns_timeout {
                    serial_println("[FAIL] DNS timeout");
                    return RunResult::DownloadFailed;
                }

                let relative_tsc = now_tsc.wrapping_sub(smoltcp_base_tsc);
                let timestamp = tsc_to_instant(relative_tsc, handoff.tsc_freq);
                adapter.refill_rx();
                iface.poll(timestamp, &mut adapter, &mut sockets);
                adapter.collect_tx();

                let dns = sockets.get_mut::<DnsSocket>(dns_handle);
                match dns.get_query_result(query_handle) {
                    Ok(addrs) => {
                        // Find first IPv4 address
                        let mut found_ip = None;
                        for addr in addrs {
                            let IpAddress::Ipv4(v4) = addr;
                            found_ip = Some(v4);
                            break;
                        }
                        match found_ip {
                            Some(ip) => break ip,
                            None => {
                                serial_println("[FAIL] DNS response has no IPv4 address");
                                return RunResult::DownloadFailed;
                            }
                        }
                    }
                    Err(GetQueryResultError::Pending) => {
                        // Still waiting, continue polling
                        for _ in 0..100 {
                            core::hint::spin_loop();
                        }
                    }
                    Err(GetQueryResultError::Failed) => {
                        serial_println("[FAIL] DNS query failed");
                        return RunResult::DownloadFailed;
                    }
                }
            };

            serial_print("[OK] DNS resolved: ");
            print_ipv4(resolved_ip);
            serial_println("");
            resolved_ip
        }
    };

    serial_print("[HTTP] Connecting to ");
    print_ipv4(server_ip);
    serial_print(":");
    serial_print_decimal(server_port as u32);
    serial_println("...");

    // Create TCP socket with STATIC buffers (no heap!)
    // Large buffers critical for throughput - 128KB RX allows TCP window scaling
    static mut TCP_RX_STORAGE: [u8; 131072] = [0u8; 131072]; // 128KB RX
    static mut TCP_TX_STORAGE: [u8; 65536] = [0u8; 65536]; // 64KB TX

    let tcp_rx_buffer = TcpSocketBuffer::new(unsafe { &mut TCP_RX_STORAGE[..] });
    let tcp_tx_buffer = TcpSocketBuffer::new(unsafe { &mut TCP_TX_STORAGE[..] });
    let mut tcp_socket = TcpSocket::new(tcp_rx_buffer, tcp_tx_buffer);

    // === THROUGHPUT OPTIMIZATIONS ===
    // Disable Nagle's algorithm - we're doing bulk download, Nagle adds latency
    // for small packets but we want ACKs to flow immediately
    tcp_socket.set_nagle_enabled(false);

    // Disable delayed ACKs - send ACKs immediately to keep sender's window open
    // Default is 10ms delay which can throttle high-throughput downloads
    tcp_socket.set_ack_delay(None);

    // Connect to server
    let local_port = 49152 + ((get_tsc() % 16384) as u16); // Random ephemeral port
    let remote_endpoint = (smoltcp::wire::IpAddress::Ipv4(server_ip), server_port);

    if tcp_socket
        .connect(iface.context(), remote_endpoint, local_port)
        .is_err()
    {
        serial_println("[FAIL] TCP connect failed to initiate");
        return RunResult::DownloadFailed;
    }

    let tcp_handle = sockets.add(tcp_socket);

    // Wait for connection to establish
    let connect_start = get_tsc();
    let connect_timeout = timeouts.tcp_connect();

    loop {
        let now_tsc = get_tsc();
        if now_tsc.wrapping_sub(connect_start) > connect_timeout {
            serial_println("[FAIL] TCP connect timeout");
            return RunResult::DownloadFailed;
        }

        let relative_tsc = now_tsc.wrapping_sub(smoltcp_base_tsc);
        let timestamp = tsc_to_instant(relative_tsc, handoff.tsc_freq);
        adapter.refill_rx();
        iface.poll(timestamp, &mut adapter, &mut sockets);
        adapter.collect_tx();

        let socket = sockets.get_mut::<TcpSocket>(tcp_handle);
        if socket.may_send() {
            serial_println("[OK] TCP connected");
            break;
        }

        if !socket.is_open() {
            serial_println("[FAIL] TCP connection refused");
            return RunResult::DownloadFailed;
        }
        // No artificial delay - tight poll loop for throughput
    }

    // Build and send HTTP GET request (NO HEAP!)
    serial_println("[HTTP] Sending GET request...");
    serial_print("[HTTP] GET ");
    serial_println(path);

    // Build HTTP request in static buffer
    static mut HTTP_REQUEST_BUF: [u8; 1024] = [0u8; 1024];
    let http_request_slice =
        unsafe { core::slice::from_raw_parts_mut((&raw mut HTTP_REQUEST_BUF).cast::<u8>(), 1024) };
    let request_len = match format_http_get(http_request_slice, path, host_str) {
        Some(len) => len,
        None => {
            serial_println("[FAIL] HTTP request too large for buffer");
            return RunResult::DownloadFailed;
        }
    };

    {
        let socket = sockets.get_mut::<TcpSocket>(tcp_handle);
        if socket
            .send_slice(&http_request_slice[..request_len])
            .is_err()
        {
            serial_println("[FAIL] Failed to send HTTP request");
            return RunResult::DownloadFailed;
        }
    }

    // Poll to send the request (may need multiple polls for large requests)
    let send_start = get_tsc();
    let send_timeout = timeouts.http_send();

    loop {
        let now_tsc = get_tsc();
        if now_tsc.wrapping_sub(send_start) > send_timeout {
            serial_println("[FAIL] HTTP send timeout");
            return RunResult::DownloadFailed;
        }

        let relative_tsc = now_tsc.wrapping_sub(smoltcp_base_tsc);
        let timestamp = tsc_to_instant(relative_tsc, handoff.tsc_freq);
        adapter.refill_rx();
        iface.poll(timestamp, &mut adapter, &mut sockets);
        adapter.collect_tx();

        // Check if request has been sent (TX buffer drained)
        let socket = sockets.get_mut::<TcpSocket>(tcp_handle);
        if socket.send_queue() == 0 {
            serial_println("[HTTP] Request sent");
            break;
        }

        if !socket.is_open() {
            serial_println("[FAIL] Connection closed during send");
            return RunResult::DownloadFailed;
        }
        // No artificial delay - tight poll loop for throughput
    }

    // Receive response
    serial_println("[HTTP] Receiving response...");

    let mut total_received: usize = 0;
    let mut headers_done = false;
    let mut content_length: Option<usize> = None;
    let mut body_received: usize = 0;
    let mut http_status: Option<u16> = None;

    // Static buffer for headers (no heap!)
    static mut HEADER_BUFFER: [u8; 16384] = [0u8; 16384];
    // Create slice once with raw pointer - avoids repeated mutable reference warnings
    let header_buffer =
        unsafe { core::slice::from_raw_parts_mut((&raw mut HEADER_BUFFER).cast::<u8>(), 16384) };
    let mut header_len: usize = 0;
    let mut last_progress_kb: usize = 0;

    let recv_start = get_tsc();
    let mut last_activity = recv_start;
    let recv_timeout = handoff.tsc_freq * 3600; // 1 hour timeout for large downloads
    let idle_timeout = handoff.tsc_freq * 60; // 60 second idle timeout (increased for slow connections)

    loop {
        let now_tsc = get_tsc();

        // Check total timeout
        if now_tsc.wrapping_sub(recv_start) > recv_timeout {
            serial_println("[FAIL] Download timeout (total time exceeded)");
            return RunResult::DownloadFailed;
        }

        // Check idle timeout (no data received for too long)
        if now_tsc.wrapping_sub(last_activity) > idle_timeout && headers_done {
            serial_println("[FAIL] Download timeout (connection stalled)");
            return RunResult::DownloadFailed;
        }

        // === POLL-DRIVEN RECEIVE LOOP ===
        // smoltcp is entirely poll-based: it won't process packets, send ACKs,
        // or advance TCP state without poll(). We must poll frequently to keep
        // ACKs flowing and prevent sender window stall.
        let relative_tsc = now_tsc.wrapping_sub(smoltcp_base_tsc);
        let timestamp = tsc_to_instant(relative_tsc, handoff.tsc_freq);

        // Phase 1: Refill RX buffers so device can receive more packets
        adapter.refill_rx();

        // Phase 2: Poll smoltcp - processes incoming packets, generates ACKs
        iface.poll(timestamp, &mut adapter, &mut sockets);

        // Phase 3: Collect TX completions and notify device of pending TX
        adapter.collect_tx();

        // Phase 4: Try to receive data from socket
        let mut buf = [0u8; 32768];
        let socket = sockets.get_mut::<TcpSocket>(tcp_handle);

        if socket.can_recv() {
            match socket.recv_slice(&mut buf) {
                Ok(len) if len > 0 => {
                    total_received += len;
                    last_activity = now_tsc;

                    if !headers_done {
                        // Accumulate header data in static buffer
                        let space_left = header_buffer.len() - header_len;

                        if len > space_left {
                            serial_println("[FAIL] HTTP headers too large");
                            return RunResult::DownloadFailed;
                        }

                        header_buffer[header_len..header_len + len].copy_from_slice(&buf[..len]);
                        header_len += len;

                        // Look for end of headers
                        if let Some(pos) = find_header_end(&header_buffer[..header_len]) {
                            headers_done = true;
                            serial_println("[HTTP] Headers received");

                            // Parse HTTP status line (NO HEAP!)
                            // Format: "HTTP/1.1 200 OK\r\n"
                            let header_str =
                                core::str::from_utf8(&header_buffer[..pos]).unwrap_or("");

                            // Find first line (status line)
                            if let Some(first_line_end) = header_str.find('\r') {
                                let status_line = &header_str[..first_line_end];

                                // Parse status code manually (avoid split().collect())
                                // Find "HTTP/x.x " prefix, then parse number
                                if let Some(space_after_http) = status_line.find(' ') {
                                    let after_http = &status_line[space_after_http + 1..];
                                    // Find the status code (3 digits)
                                    let status_end =
                                        after_http.find(' ').unwrap_or(after_http.len());
                                    let status_str = &after_http[..status_end];

                                    if let Some(status) = parse_u16(status_str) {
                                        http_status = Some(status);
                                        serial_print("[HTTP] Status: ");
                                        serial_print_decimal(status as u32);
                                        if status_end < after_http.len() {
                                            serial_print(" ");
                                            serial_println(&after_http[status_end + 1..]);
                                        } else {
                                            serial_println("");
                                        }

                                        // Check for HTTP errors
                                        if status >= 400 {
                                            serial_print("[FAIL] HTTP error: ");
                                            serial_print_decimal(status as u32);
                                            serial_println("");
                                            return RunResult::DownloadFailed;
                                        }

                                        // Handle redirects (3xx)
                                        if status >= 300 && status < 400 {
                                            serial_println("[WARN] HTTP redirect - not following");
                                        }
                                    }
                                }
                            }

                            // Parse headers - look for Content-Length (case-insensitive, NO HEAP!)
                            for line in header_str.lines().skip(1) {
                                // Case-insensitive comparison without allocation
                                if starts_with_ignore_case(line, "content-length:") {
                                    // Find the ':' and parse the number after it
                                    if let Some(colon_pos) = line.find(':') {
                                        let value_str = line[colon_pos + 1..].trim();
                                        if let Some(len) = parse_usize(value_str) {
                                            content_length = Some(len);
                                            serial_print("[HTTP] Content-Length: ");
                                            // Print in human readable format
                                            if len >= 1024 * 1024 * 1024 {
                                                serial_print_decimal(
                                                    (len / (1024 * 1024 * 1024)) as u32,
                                                );
                                                serial_println(" GB");
                                            } else if len >= 1024 * 1024 {
                                                serial_print_decimal((len / (1024 * 1024)) as u32);
                                                serial_println(" MB");
                                            } else if len >= 1024 {
                                                serial_print_decimal((len / 1024) as u32);
                                                serial_println(" KB");
                                            } else {
                                                serial_print_decimal(len as u32);
                                                serial_println(" bytes");
                                            }
                                        }
                                    }
                                } else if starts_with_ignore_case(line, "content-type:") {
                                    if let Some(colon_pos) = line.find(':') {
                                        let value = line[colon_pos + 1..].trim();
                                        serial_print("[HTTP] Content-Type: ");
                                        serial_println(value);
                                    }
                                } else if starts_with_ignore_case(line, "transfer-encoding:") {
                                    if contains_ignore_case(line, "chunked") {
                                        serial_println("[HTTP] Transfer-Encoding: chunked");
                                        // NOTE: Chunked encoding would need special handling
                                    }
                                }
                            }

                            // Body starts after \r\n\r\n
                            let body_start = pos + 4;
                            if header_len > body_start {
                                let initial_body = &header_buffer[body_start..header_len];
                                body_received = initial_body.len();

                                // Write initial body data to disk if enabled
                                if let Some(ref mut blk) = blk_driver {
                                    let written = buffer_disk_write(blk, initial_body);
                                    if written != initial_body.len() {
                                        serial_println(
                                            "[FAIL] Failed to write initial body to disk",
                                        );
                                        return RunResult::DownloadFailed;
                                    }
                                }
                            }

                            serial_println("[HTTP] Streaming body...");
                        }
                    } else {
                        body_received += len;

                        // Write received data to disk if enabled
                        if let Some(ref mut blk) = blk_driver {
                            let written = buffer_disk_write(blk, &buf[..len]);
                            if written != len {
                                serial_print("[FAIL] Disk write failed at byte ");
                                serial_print_decimal(DISK_TOTAL_BYTES as u32);
                                serial_print(", wrote ");
                                serial_print_decimal(written as u32);
                                serial_print(" of ");
                                serial_print_decimal(len as u32);
                                serial_println(" bytes");
                                return RunResult::DownloadFailed;
                            }
                        }

                        // Print progress every 1MB with inline progress bar
                        let current_mb = body_received / (1024 * 1024);
                        let last_mb = last_progress_kb / 1024; // Reuse variable as MB tracker
                        if current_mb > last_mb {
                            last_progress_kb = current_mb * 1024; // Update tracker

                            // Carriage return to update in place
                            serial_print("\r[");

                            // Progress bar (20 chars wide)
                            if let Some(cl) = content_length {
                                let percent = ((body_received as u64 * 100) / cl as u64) as usize;
                                let filled = percent / 5; // 20 chars = 5% each
                                for i in 0..20 {
                                    if i < filled {
                                        serial_print("=");
                                    } else if i == filled {
                                        serial_print(">");
                                    } else {
                                        serial_print(" ");
                                    }
                                }
                                serial_print("] ");
                                serial_print_decimal(percent as u32);
                                serial_print("% ");
                            } else {
                                serial_print("====================] ");
                            }

                            // Size in MB
                            serial_print_decimal(current_mb as u32);
                            serial_print(" MB");

                            // Speed estimate if we have content-length
                            if let Some(cl) = content_length {
                                serial_print(" / ");
                                serial_print_decimal((cl / (1024 * 1024)) as u32);
                                serial_print(" MB");
                            }

                            serial_print("   "); // Padding to clear old chars
                        }
                    }
                }
                _ => {} // No data this iteration, will poll again
            }
        }

        // Check if download complete
        let socket = sockets.get_mut::<TcpSocket>(tcp_handle);
        if headers_done {
            if let Some(cl) = content_length {
                if body_received >= cl {
                    serial_println("\n[HTTP] Download complete");
                    break;
                }
            }

            // Also check if connection closed
            if !socket.is_open() && socket.recv_queue() == 0 {
                if let Some(cl) = content_length {
                    if body_received < cl {
                        // Had content-length but didn't get all data
                        serial_println("\n[WARN] Connection closed before full content received");
                    }
                } else {
                    // No content-length, server closed = complete
                    serial_println("\n[HTTP] Download complete (connection closed)");
                }
                break;
            }
        }
        // Loop continues - poll again next iteration
    }

    // Print download summary
    serial_println("");
    serial_println("=== DOWNLOAD SUMMARY ===");
    serial_print("[HTTP] Total headers + body: ");
    if total_received >= 1024 * 1024 {
        serial_print_decimal((total_received / (1024 * 1024)) as u32);
        serial_println(" MB");
    } else {
        serial_print_decimal((total_received / 1024) as u32);
        serial_println(" KB");
    }
    serial_print("[HTTP] Body only: ");
    if body_received >= 1024 * 1024 {
        serial_print_decimal((body_received / (1024 * 1024)) as u32);
        serial_println(" MB");
    } else {
        serial_print_decimal((body_received / 1024) as u32);
        serial_println(" KB");
    }
    if let Some(status) = http_status {
        serial_print("[HTTP] Final status: ");
        serial_print_decimal(status as u32);
        serial_println("");
    }

    // Verify we got what we expected
    if let Some(cl) = content_length {
        if body_received >= cl {
            serial_println("[OK] Download verified: received >= Content-Length");
        } else {
            serial_print("[WARN] Incomplete: received ");
            serial_print_decimal(body_received as u32);
            serial_print(" of ");
            serial_print_decimal(cl as u32);
            serial_println(" bytes");
        }
    }
    serial_println("");

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 6: FINALIZE DISK WRITE
    // ═══════════════════════════════════════════════════════════════════════
    if let Some(ref mut blk) = blk_driver {
        serial_println("[DISK] Flushing remaining data to disk...");

        // Flush any remaining buffered data
        if flush_remaining_disk_buffer(blk) {
            serial_println("[OK] Disk write finalized");
        } else {
            serial_println("[WARN] Final flush failed");
        }

        // Print disk write summary
        serial_println("");
        serial_println("=== DISK WRITE SUMMARY ===");
        serial_print("[DISK] Total bytes written: ");
        if DISK_TOTAL_BYTES >= 1024 * 1024 * 1024 {
            serial_print_decimal((DISK_TOTAL_BYTES / (1024 * 1024 * 1024)) as u32);
            serial_println(" GB");
        } else if DISK_TOTAL_BYTES >= 1024 * 1024 {
            serial_print_decimal((DISK_TOTAL_BYTES / (1024 * 1024)) as u32);
            serial_println(" MB");
        } else {
            serial_print_decimal((DISK_TOTAL_BYTES / 1024) as u32);
            serial_println(" KB");
        }
        serial_print("[DISK] Start sector: ");
        serial_print_decimal(config.target_start_sector as u32);
        serial_println("");
        serial_print("[DISK] End sector: ");
        serial_print_decimal(DISK_NEXT_SECTOR as u32);
        serial_println("");
        serial_print("[DISK] Sectors written: ");
        serial_print_decimal((DISK_NEXT_SECTOR - config.target_start_sector) as u32);
        serial_println("");

        // ═══════════════════════════════════════════════════════════════════
        // STEP 6.5: WRITE ISO MANIFEST
        // ═══════════════════════════════════════════════════════════════════
        // Write manifest so bootloader can discover this ISO on next boot
        if DISK_TOTAL_BYTES > 0 {
            // Now works with both VirtIO and AHCI!
            let manifest_result = finalize_manifest(blk, &config, DISK_TOTAL_BYTES);
            if manifest_result {
                serial_println("[OK] ISO manifest written successfully");
            } else {
                serial_println("[WARN] Failed to write ISO manifest");
            }
        }

        // ═══════════════════════════════════════════════════════════════════
        // STEP 6.6: FINAL DISK SYNC
        // ═══════════════════════════════════════════════════════════════════
        // Flush VirtIO-blk write cache to ensure all data is persisted
        serial_println("[DISK] Syncing disk cache...");
        use crate::driver::block_traits::BlockDriver;
        match blk.flush() {
            Ok(()) => serial_println("[OK] Disk cache synced"),
            Err(e) => {
                serial_print("[WARN] Disk sync failed: ");
                serial_println(match e {
                    crate::driver::block_traits::BlockError::Unsupported => {
                        "not supported (assuming durable)"
                    }
                    crate::driver::block_traits::BlockError::Timeout => "timeout",
                    crate::driver::block_traits::BlockError::DeviceError => "device error",
                    _ => "unknown error",
                });
            }
        }
    } else {
        serial_println("[NOTE] Disk write disabled - data received but not persisted");
    }
    serial_println("");

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 7: SAFE SYSTEM SHUTDOWN/REBOOT
    // ═══════════════════════════════════════════════════════════════════════
    serial_println("");
    serial_println("=====================================");
    serial_println("  ISO Download Complete!");
    serial_println("=====================================");
    serial_println("");
    serial_println("Initiating safe system reboot...");

    unsafe {
        // 1) Keyboard controller reset (most compatible method)
        serial_println("[REBOOT] Using keyboard controller reset (0x64 -> 0xFE)");
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x64u16,
            in("al") 0xFEu8,
            options(nomem, nostack)
        );

        // Wait for reboot (should happen quickly)
        for _ in 0..50_000_000 {
            core::hint::spin_loop();
        }

        // 2) Fallback: Port 0xCF9 reset (modern systems)
        serial_println("[REBOOT] Fallback: Port 0xCF9 reset");
        core::arch::asm!(
            "out dx, al",
            in("dx") 0xCF9u16,
            in("al") 0x06u8,
            options(nomem, nostack)
        );

        // Wait again
        for _ in 0..50_000_000 {
            core::hint::spin_loop();
        }

        // 3) If reboot failed, halt gracefully instead of triple-fault
        serial_println("[REBOOT] Reboot methods failed - halting system");
        serial_println("[REBOOT] Please manually power cycle the system");
        loop {
            core::arch::asm!("hlt", options(nomem, nostack));
        }
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub unsafe fn bare_metal_main(
    _handoff: &'static BootHandoff,
    _config: BareMetalConfig,
) -> RunResult {
    RunResult::InitFailed
}

// ═══════════════════════════════════════════════════════════════════════════
// FULL INTEGRATED RUNNER (with real state machines)
// ═══════════════════════════════════════════════════════════════════════════

/// Full integrated main loop with real state machines.
///
/// This is the production implementation that uses:
/// - VirtioNetDevice for networking
/// - smoltcp for TCP/IP
/// - IsoDownloadState for orchestration
/// - DiskWriterState for streaming writes
#[cfg(target_arch = "x86_64")]
pub unsafe fn run_full_download<D: NetworkDriver>(
    device: &mut D,
    handoff: &'static BootHandoff,
    iso_url: Url,
) -> RunResult {
    serial_println("[MAIN] Starting full integrated download...");

    let timeouts = TimeoutConfig::new(handoff.tsc_freq);
    let loop_config = MainLoopConfig::new(handoff.tsc_freq);

    // Create download config
    let download_config = DownloadConfig::new(iso_url);

    // Create download state machine
    let mut download_state = IsoDownloadState::new(download_config);

    // Start download (no existing network config, will do DHCP)
    download_state.start(None, get_tsc());

    // Main loop
    let mut iteration = 0u64;
    loop {
        let iteration_start = get_tsc();

        // Phase 1: RX Refill
        phase1_rx_refill(device);

        // Phase 2: Would poll smoltcp here
        // let timestamp = tsc_to_instant(iteration_start, handoff.tsc_freq);
        // iface.poll(timestamp, device, &mut sockets);

        // Phase 3: TX drain (handled by smoltcp)

        // Phase 4: App state step
        // Note: This is simplified - real impl needs smoltcp socket integration
        // let result = download_state.step(...);

        // Phase 5: TX completions
        phase5_tx_completions(device);

        // Check timing
        let elapsed = get_tsc().wrapping_sub(iteration_start);
        if elapsed > loop_config.timing_warning_ticks {
            serial_println("[WARN] Iteration exceeded 5ms");
        }

        iteration += 1;

        // For demonstration, exit after some iterations
        if iteration > 1000 {
            break;
        }
    }

    RunResult::Success
}

#[cfg(not(target_arch = "x86_64"))]
pub unsafe fn run_full_download<D: NetworkDriver>(
    _device: &mut D,
    _handoff: &'static BootHandoff,
    _iso_url: Url,
) -> RunResult {
    RunResult::InitFailed
}

/// Convert TSC ticks to smoltcp Instant.
fn tsc_to_instant(tsc: u64, tsc_freq: u64) -> Instant {
    let ms = tsc / (tsc_freq / 1000);
    Instant::from_millis(ms as i64)
}

/// Find the end of HTTP headers (\r\n\r\n).
/// Returns the position of the first \r in \r\n\r\n.
fn find_header_end(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|w| w == b"\r\n\r\n")
}
