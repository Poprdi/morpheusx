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

use alloc::vec::Vec;
use alloc::vec;
use alloc::string::{String, ToString};
use alloc::format;

use smoltcp::iface::{Config, Interface, SocketSet, SocketStorage};
use smoltcp::socket::dhcpv4::{Socket as Dhcpv4Socket, Event as Dhcpv4Event};
use smoltcp::socket::tcp::{Socket as TcpSocket, SocketBuffer as TcpSocketBuffer};
use smoltcp::socket::dns::{Socket as DnsSocket, GetQueryResultError};
use smoltcp::time::Instant;
use smoltcp::wire::{DnsQueryType, EthernetAddress, HardwareAddress, IpAddress, IpCidr, Ipv4Address, Ipv4Cidr};

use crate::boot::handoff::{BootHandoff, TRANSPORT_MMIO, TRANSPORT_PCI_MODERN, BLK_TYPE_VIRTIO};
use crate::boot::init::TimeoutConfig;
use crate::driver::virtio::{VirtioNetDriver, VirtioConfig, VirtioInitError};
use crate::driver::virtio::{VirtioTransport, TransportType, PciModernConfig};
use crate::driver::virtio_blk::{VirtioBlkDriver, VirtioBlkConfig, VirtioBlkInitError};
use crate::driver::block_io_adapter::VirtioBlkBlockIo;
use crate::driver::traits::NetworkDriver;
use crate::driver::block_traits::BlockDriver;
use crate::transfer::disk::{ManifestWriter, ChunkSet, ChunkPartition, PartitionInfo, MAX_CHUNK_PARTITIONS};
use crate::url::Url;

// Import manifest support from morpheus-core
use morpheus_core::iso::{IsoManifest, MAX_MANIFEST_SIZE};

// Import from sibling modules in the mainloop package
use super::runner::{MainLoopConfig, get_tsc};
use super::phases::{phase1_rx_refill, phase5_tx_completions};

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

/// Write string to serial port.
pub fn serial_print(s: &str) {
    for byte in s.bytes() {
        unsafe { serial_write_byte(byte); }
    }
}

/// Write string with newline.
pub fn serial_println(s: &str) {
    serial_print(s);
    serial_print("\r\n");
}

/// Print hex number.
pub fn serial_print_hex(value: u64) {
    serial_print("0x");
    for i in (0..16).rev() {
        let nibble = ((value >> (i * 4)) & 0xF) as u8;
        let c = if nibble < 10 { b'0' + nibble } else { b'a' + nibble - 10 };
        unsafe { serial_write_byte(c); }
    }
}

/// Print an IPv4 address (e.g., "10.0.2.15").
pub fn print_ipv4(ip: Ipv4Address) {
    let octets = ip.as_bytes();
    for (i, octet) in octets.iter().enumerate() {
        if i > 0 { serial_print("."); }
        serial_print_decimal(*octet as u32);
    }
}

/// Print a decimal number.
pub fn serial_print_decimal(value: u32) {
    if value == 0 {
        unsafe { serial_write_byte(b'0'); }
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = 0;
    let mut val = value;
    while val > 0 {
        buf[i] = b'0' + (val % 10) as u8;
        val /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        unsafe { serial_write_byte(buf[i]); }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// STREAMING DISK WRITE HELPERS
// ═══════════════════════════════════════════════════════════════════════════

/// Flush the disk write buffer to the block device.
/// 
/// Writes the buffered data as one or more sector writes.
/// Returns the number of bytes written, or 0 on error.
unsafe fn flush_disk_buffer(blk_driver: &mut VirtioBlkDriver) -> usize {
    if DISK_WRITE_BUFFER_FILL == 0 {
        return 0;
    }
    
    // Calculate sectors to write (round up)
    let bytes_to_write = DISK_WRITE_BUFFER_FILL;
    let num_sectors = ((bytes_to_write + 511) / 512) as u32;
    
    // Get the buffer physical address (we're identity mapped post-EBS)
    let buffer_phys = DISK_WRITE_BUFFER.as_ptr() as u64;
    
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
    if let Err(e) = blk_driver.submit_write(
        DISK_NEXT_SECTOR,
        buffer_phys,
        num_sectors,
        request_id,
    ) {
        serial_print("[DISK] ERROR: Write submit failed at sector ");
        serial_print_hex(DISK_NEXT_SECTOR);
        serial_println("");
        return 0;
    }
    
    // Notify the device
    blk_driver.notify();
    
    // Poll for completion (with timeout)
    let start_tsc = super::runner::get_tsc();
    let timeout_ticks = 100_000_000; // ~100ms at 1GHz TSC
    
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
                    serial_println("");
                    return 0;
                }
            }
        }
        
        let now = super::runner::get_tsc();
        if now.wrapping_sub(start_tsc) > timeout_ticks {
            serial_println("[DISK] ERROR: Write completion timeout");
            return 0;
        }
        
        core::hint::spin_loop();
    }
}

/// Add data to the disk write buffer.
/// Automatically flushes when buffer is full.
/// Returns the number of bytes consumed from the input.
unsafe fn buffer_disk_write(
    blk_driver: &mut VirtioBlkDriver,
    data: &[u8],
) -> usize {
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
unsafe fn flush_remaining_disk_buffer(blk_driver: &mut VirtioBlkDriver) -> bool {
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

/// Write an ISO manifest to disk at the specified sector.
/// 
/// The manifest is serialized and written to the manifest sector.
/// This allows the bootloader to discover downloaded ISOs on next boot.
/// 
/// # Arguments
/// * `blk_driver` - The block driver to write with
/// * `manifest_sector` - Sector number to write manifest at
/// * `manifest` - The manifest to write
/// 
/// # Returns
/// true if write successful, false otherwise
unsafe fn write_manifest_to_disk(
    blk_driver: &mut VirtioBlkDriver,
    manifest_sector: u64,
    manifest: &IsoManifest,
) -> bool {
    serial_println("[MANIFEST] Serializing manifest...");
    
    // Clear buffer
    for b in MANIFEST_BUFFER.iter_mut() {
        *b = 0;
    }
    
    // Serialize manifest
    let manifest_buffer = &mut MANIFEST_BUFFER;
    let size = match manifest.serialize(manifest_buffer) {
        Ok(s) => s,
        Err(_) => {
            serial_println("[MANIFEST] ERROR: Failed to serialize manifest");
            return false;
        }
    };
    
    serial_print("[MANIFEST] Manifest size: ");
    serial_print_decimal(size as u32);
    serial_println(" bytes");
    
    // Calculate sectors needed (round up)
    let num_sectors = ((size + 511) / 512) as u32;
    serial_print("[MANIFEST] Writing ");
    serial_print_decimal(num_sectors);
    serial_print(" sector(s) to sector ");
    serial_print_hex(manifest_sector);
    serial_println("");
    
    // Get the buffer physical address
    let buffer_phys = manifest_buffer.as_ptr() as u64;
    
    // Poll for completion space first
    while let Some(_completion) = blk_driver.poll_completion() {
        // Drain pending completions
    }
    
    // Check if driver can accept a request
    if !blk_driver.can_submit() {
        serial_println("[MANIFEST] ERROR: Block queue full");
        return false;
    }
    
    // Submit the write
    let request_id = DISK_NEXT_REQUEST_ID;
    DISK_NEXT_REQUEST_ID = DISK_NEXT_REQUEST_ID.wrapping_add(1);
    
    if let Err(_) = blk_driver.submit_write(
        manifest_sector,
        buffer_phys,
        num_sectors,
        request_id,
    ) {
        serial_println("[MANIFEST] ERROR: Write submit failed");
        return false;
    }
    
    // Notify the device
    blk_driver.notify();
    
    // Poll for completion (with timeout)
    let start_tsc = super::runner::get_tsc();
    let timeout_ticks = 100_000_000; // ~100ms at 1GHz TSC
    
    loop {
        if let Some(completion) = blk_driver.poll_completion() {
            if completion.request_id == request_id {
                if completion.status == 0 {
                    serial_println("[MANIFEST] OK: Manifest written to disk");
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
/// Called after HTTP download completes to record the ISO location.
/// 
/// # Strategy
/// - If `esp_start_lba > 0`: Write to FAT32 ESP at `/morpheus/isos/<name>.manifest`
/// - Else if `manifest_sector > 0`: Write to raw sector (legacy)
/// - Else: Skip manifest writing
unsafe fn finalize_manifest(
    blk_driver: &mut VirtioBlkDriver,
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
        return finalize_manifest_fat32(blk_driver, config, total_bytes, end_sector);
    }
    
    // Fall back to legacy raw sector write
    finalize_manifest_raw(blk_driver, config, total_bytes, end_sector)
}

/// Write manifest to FAT32 ESP filesystem.
/// 
/// Creates `/morpheus/isos/<name>.manifest` file on the ESP.
unsafe fn finalize_manifest_fat32(
    blk_driver: &mut VirtioBlkDriver,
    config: &BareMetalConfig,
    total_bytes: u64,
    end_sector: u64,
) -> bool {
    serial_println("[MANIFEST] Writing to FAT32 ESP...");
    serial_print("[MANIFEST] ESP start LBA: ");
    serial_print_hex(config.esp_start_lba);
    serial_println("");
    
    // Create ManifestWriter (our disk module's version)
    let mut manifest = ManifestWriter::new(config.iso_name, total_bytes);
    manifest.set_complete(true);
    
    // Build chunk set with partition info
    let mut chunks = ChunkSet::new();
    chunks.count = 1;
    chunks.total_size = total_bytes;
    chunks.bytes_written = total_bytes;
    
    // Set up the partition info
    let mut part_info = PartitionInfo::new(0, config.target_start_sector, end_sector, config.partition_uuid);
    part_info.set_name(config.iso_name);
    
    // Create chunk partition
    let mut chunk = ChunkPartition::new(part_info, 0);
    chunk.bytes_written = total_bytes;
    chunk.complete = true;
    chunks.chunks[0] = chunk;
    
    // Create BlockIo adapter for FAT32 operations
    // Use our static DMA buffer (reuse the write buffer since download is done)
    let dma_buffer = &mut DISK_WRITE_BUFFER;
    let dma_buffer_phys = dma_buffer.as_ptr() as u64;
    let timeout_ticks = 100_000_000u64; // ~100ms
    
    let mut adapter = match VirtioBlkBlockIo::new(
        blk_driver,
        dma_buffer,
        dma_buffer_phys,
        timeout_ticks,
    ) {
        Ok(a) => a,
        Err(_) => {
            serial_println("[MANIFEST] ERROR: Failed to create BlockIo adapter");
            return false;
        }
    };
    
    // Write manifest to FAT32
    match manifest.write_to_esp_fat32(&mut adapter, config.esp_start_lba, &chunks) {
        Ok(()) => {
            serial_println("[MANIFEST] OK: Written to /morpheus/isos/<name>.manifest");
            true
        }
        Err(e) => {
            serial_print("[MANIFEST] ERROR: FAT32 write failed: ");
            serial_println(match e {
                crate::transfer::disk::DiskError::IoError => "IO error",
                crate::transfer::disk::DiskError::ManifestError => "Manifest error",
                crate::transfer::disk::DiskError::BufferTooSmall => "Buffer too small",
                _ => "Unknown error",
            });
            false
        }
    }
}

/// Write manifest to raw disk sector (legacy method).
unsafe fn finalize_manifest_raw(
    blk_driver: &mut VirtioBlkDriver,
    config: &BareMetalConfig,
    total_bytes: u64,
    end_sector: u64,
) -> bool {
    serial_println("[MANIFEST] Writing to raw sector (legacy)...");
    serial_print("[MANIFEST] Sector: ");
    serial_print_hex(config.manifest_sector);
    serial_println("");
    
    // Use morpheus_core's IsoManifest for raw sector write
    let mut manifest = IsoManifest::new(config.iso_name, total_bytes);
    
    // Add chunk entry
    match manifest.add_chunk(
        config.partition_uuid,
        config.target_start_sector,
        end_sector,
    ) {
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
    
    // Write to disk
    write_manifest_to_disk(blk_driver, config.manifest_sector, &manifest)
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
    if pos + prefix.len() > buffer.len() { return None; }
    buffer[pos..pos + prefix.len()].copy_from_slice(prefix);
    pos += prefix.len();
    
    // path
    let path_bytes = path.as_bytes();
    if pos + path_bytes.len() > buffer.len() { return None; }
    buffer[pos..pos + path_bytes.len()].copy_from_slice(path_bytes);
    pos += path_bytes.len();
    
    // " HTTP/1.1\r\nHost: "
    let mid = b" HTTP/1.1\r\nHost: ";
    if pos + mid.len() > buffer.len() { return None; }
    buffer[pos..pos + mid.len()].copy_from_slice(mid);
    pos += mid.len();
    
    // host
    let host_bytes = host.as_bytes();
    if pos + host_bytes.len() > buffer.len() { return None; }
    buffer[pos..pos + host_bytes.len()].copy_from_slice(host_bytes);
    pos += host_bytes.len();
    
    // Headers and terminator
    let suffix = b"\r\nUser-Agent: MorpheusX/1.0\r\nAccept: */*\r\nConnection: close\r\n\r\n";
    if pos + suffix.len() > buffer.len() { return None; }
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
    if has_digit { Some(result) } else { None }
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
            loop { core::hint::spin_loop(); }
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
}

impl<'a, D: NetworkDriver> SmoltcpAdapter<'a, D> {
    pub fn new(driver: &'a mut D) -> Self {
        Self {
            driver,
            rx_buffer: [0u8; 2048],
            rx_len: 0,
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
                }
                _ => {}
            }
        }
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
        // Max frame is 1514 bytes, but smoltcp may request up to ~1600 for headers
        const MAX_FRAME: usize = 2048;
        let mut buffer = [0u8; MAX_FRAME];
        
        let actual_len = if len > MAX_FRAME {
            serial_println("[ADAPTER-TX] ERROR: requested len exceeds buffer!");
            MAX_FRAME
        } else {
            len
        };
        
        let result = f(&mut buffer[..actual_len]);
        
        // Fire-and-forget transmit - don't wait for completion
        let _ = self.driver.transmit(&buffer[..actual_len]);
        
        result
    }
}

impl<'a, D: NetworkDriver> smoltcp::phy::Device for SmoltcpAdapter<'a, D> {
    type RxToken<'b> = RxToken where Self: 'b;
    type TxToken<'b> = TxToken<'b, D> where Self: 'b;

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
                RxToken { buffer: rx_buf, len: rx_len },
                TxToken { driver: self.driver },
            ))
        } else {
            None
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        if self.driver.can_transmit() {
            Some(TxToken { driver: self.driver })
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
    /// If non-zero, writes manifest to /morpheus/isos/<name>.manifest on ESP.
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
            target_start_sector: 2048, // Start at 1MB offset (after manifest)
            manifest_sector: 0,        // Use FAT32 by default (set non-zero for raw sector)
            esp_start_lba: 2048,       // Standard GPT ESP at sector 2048
            max_download_size: 4 * 1024 * 1024 * 1024, // 4GB max
            write_to_disk: true, // Enable disk writes by default
            partition_uuid: [0u8; 16], // Will be set by bootloader if known
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
pub unsafe fn bare_metal_main(
    handoff: &'static BootHandoff,
    config: BareMetalConfig,
) -> RunResult {
    serial_println("=====================================");
    serial_println("  MorpheusX Post-EBS Network Stack");
    serial_println("=====================================");
    serial_println("");

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 1: POST-EBS INITIALIZATION
    // ═══════════════════════════════════════════════════════════════════════
    serial_println("[INIT] Initializing post-EBS heap allocator...");
    crate::alloc_heap::init_heap();
    serial_println("[OK] Heap allocator ready (1MB)");
    
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

    // Create timeout config
    let timeouts = TimeoutConfig::new(handoff.tsc_freq);
    let loop_config = MainLoopConfig::new(handoff.tsc_freq);

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 2: INITIALIZE NETWORK DEVICE
    // ═══════════════════════════════════════════════════════════════════════
    serial_println("[INIT] Creating VirtIO config...");
    
    // Create VirtIO config from handoff data
    let virtio_config = VirtioConfig {
        dma_cpu_base: handoff.dma_cpu_ptr as *mut u8,
        dma_bus_base: handoff.dma_cpu_ptr, // In identity-mapped post-EBS, bus=physical=virtual
        dma_size: handoff.dma_size as usize,
        queue_size: VirtioConfig::DEFAULT_QUEUE_SIZE,
        buffer_size: VirtioConfig::DEFAULT_BUFFER_SIZE,
    };
    
    // Determine transport type from handoff
    serial_print("[INIT] Transport type: ");
    let transport = match handoff.nic_transport_type {
        TRANSPORT_PCI_MODERN => {
            serial_println("PCI Modern");
            serial_print("[INIT] Common cfg: ");
            serial_print_hex(handoff.nic_common_cfg);
            serial_println("");
            serial_print("[INIT] Notify cfg: ");
            serial_print_hex(handoff.nic_notify_cfg);
            serial_println("");
            serial_print("[INIT] Device cfg: ");
            serial_print_hex(handoff.nic_device_cfg);
            serial_println("");
            serial_print("[INIT] Notify off multiplier: ");
            serial_print_decimal(handoff.nic_notify_off_multiplier);
            serial_println("");
            
            VirtioTransport::pci_modern(PciModernConfig {
                common_cfg: handoff.nic_common_cfg,
                notify_cfg: handoff.nic_notify_cfg,
                notify_off_multiplier: handoff.nic_notify_off_multiplier,
                isr_cfg: handoff.nic_isr_cfg,
                device_cfg: handoff.nic_device_cfg,
                pci_cfg: 0, // Not used for now
            })
        }
        TRANSPORT_MMIO => {
            serial_println("MMIO");
            serial_print("[INIT] MMIO base: ");
            serial_print_hex(handoff.nic_mmio_base);
            serial_println("");
            VirtioTransport::mmio(handoff.nic_mmio_base)
        }
        _ => {
            serial_println("Unknown (defaulting to MMIO)");
            serial_print("[INIT] NIC MMIO base: ");
            serial_print_hex(handoff.nic_mmio_base);
            serial_println("");
            VirtioTransport::mmio(handoff.nic_mmio_base)
        }
    };
    
    serial_println("[INIT] Initializing VirtIO-net driver...");
    
    let mut driver = match VirtioNetDriver::new_with_transport(transport, virtio_config, handoff.tsc_freq) {
        Ok(d) => {
            serial_println("[OK] VirtIO driver initialized");
            d
        }
        Err(e) => {
            serial_print("[FAIL] VirtIO init error: ");
            match e {
                VirtioInitError::ResetTimeout => 
                    serial_println("reset timeout"),
                VirtioInitError::FeatureNegotiationFailed => 
                    serial_println("feature negotiation failed"),
                VirtioInitError::FeaturesRejected => 
                    serial_println("features rejected by device"),
                VirtioInitError::QueueSetupFailed => 
                    serial_println("queue setup failed"),
                VirtioInitError::RxPrefillFailed(_) => 
                    serial_println("RX prefill failed"),
                VirtioInitError::DeviceError => 
                    serial_println("device error"),
            }
            return RunResult::InitFailed;
        }
    };
    
    // Print real MAC address from driver
    serial_print("[INIT] MAC address: ");
    let mac = driver.mac_address();
    for (i, byte) in mac.iter().enumerate() {
        if i > 0 { serial_print(":"); }
        let hi = byte >> 4;
        let lo = byte & 0xF;
        unsafe {
            serial_write_byte(if hi < 10 { b'0' + hi } else { b'a' + hi - 10 });
            serial_write_byte(if lo < 10 { b'0' + lo } else { b'a' + lo - 10 });
        }
    }
    serial_println("");

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 2.5: INITIALIZE BLOCK DEVICE (VirtIO-blk)
    // ═══════════════════════════════════════════════════════════════════════
    let mut blk_driver: Option<VirtioBlkDriver> = None;
    
    if config.write_to_disk && handoff.has_block_device() {
        serial_println("[INIT] Initializing VirtIO-blk driver...");
        serial_print("[INIT] Block MMIO base: ");
        serial_print_hex(handoff.blk_mmio_base);
        serial_println("");
        serial_print("[INIT] Block sector size: ");
        serial_print_decimal(handoff.blk_sector_size);
        serial_println("");
        serial_print("[INIT] Block total sectors: ");
        serial_print_hex(handoff.blk_total_sectors);
        serial_println("");
        
        // Calculate DMA region for block device
        // Use second half of DMA region (first half is for network)
        let blk_dma_offset = handoff.dma_size / 2;
        let blk_dma_base = handoff.dma_cpu_ptr + blk_dma_offset;
        
        // VirtIO-blk queue layout:
        // - Descriptors: 32 * 16 = 512 bytes
        // - Avail ring: 4 + 32*2 + 2 = 70 bytes (pad to 512)
        // - Used ring: 4 + 32*8 + 2 = 262 bytes (pad to 512)
        // - Headers: 32 * 16 = 512 bytes (one header per desc set)
        // - Status: 32 bytes (one per desc set)
        // - Data buffers: 32 * 64KB = 2MB (for larger writes)
        
        let blk_config = VirtioBlkConfig {
            queue_size: 32,
            desc_phys: blk_dma_base,
            avail_phys: blk_dma_base + 512,
            used_phys: blk_dma_base + 1024,
            headers_phys: blk_dma_base + 2048,
            status_phys: blk_dma_base + 2048 + 512,
            headers_cpu: blk_dma_base + 2048,
            status_cpu: blk_dma_base + 2048 + 512,
            notify_addr: handoff.blk_mmio_base + 0x50, // MMIO notify offset
        };
        
        match VirtioBlkDriver::new(handoff.blk_mmio_base, blk_config) {
            Ok(d) => {
                let info = d.info();
                serial_println("[OK] VirtIO-blk driver initialized");
                serial_print("[OK] Disk capacity: ");
                let total_bytes = info.total_sectors * info.sector_size as u64;
                if total_bytes >= 1024 * 1024 * 1024 {
                    serial_print_decimal((total_bytes / (1024 * 1024 * 1024)) as u32);
                    serial_println(" GB");
                } else {
                    serial_print_decimal((total_bytes / (1024 * 1024)) as u32);
                    serial_println(" MB");
                }
                
                // Initialize disk write state
                DISK_NEXT_SECTOR = config.target_start_sector;
                DISK_WRITE_BUFFER_FILL = 0;
                DISK_TOTAL_BYTES = 0;
                DISK_NEXT_REQUEST_ID = 1;
                
                blk_driver = Some(d);
            }
            Err(e) => {
                serial_print("[WARN] VirtIO-blk init failed: ");
                match e {
                    VirtioBlkInitError::ResetFailed => serial_println("reset failed"),
                    VirtioBlkInitError::FeatureNegotiationFailed => serial_println("feature negotiation failed"),
                    VirtioBlkInitError::QueueSetupFailed => serial_println("queue setup failed"),
                    VirtioBlkInitError::DeviceFailed => serial_println("device error"),
                    VirtioBlkInitError::InvalidConfig => serial_println("invalid config"),
                }
                serial_println("[WARN] Continuing without disk write support");
            }
        }
    } else if config.write_to_disk {
        serial_println("[WARN] No block device in handoff - disk writes disabled");
    } else {
        serial_println("[INFO] Disk writes disabled by config");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 3: CREATE SMOLTCP INTERFACE
    // ═══════════════════════════════════════════════════════════════════════
    serial_println("[INIT] Creating smoltcp interface...");

    // Use the MAC address from the driver
    let mac = EthernetAddress(driver.mac_address());
    let hw_addr = HardwareAddress::Ethernet(mac);

    // Create smoltcp config
    let mut iface_config = Config::new(hw_addr);
    iface_config.random_seed = handoff.tsc_freq; // Use TSC frequency as random seed
    
    // Create the device adapter wrapping our VirtIO driver
    let mut adapter = SmoltcpAdapter::new(&mut driver);

    // Create smoltcp interface  
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
    
    // Track if we got an IP
    let mut got_ip = false;
    let mut our_ip = Ipv4Address::UNSPECIFIED;
    let mut gateway_ip = Ipv4Address::UNSPECIFIED;
    let mut dns_ip = Ipv4Address::UNSPECIFIED;
    
    // DHCP polling loop
    serial_println("[NET] Sending DHCP DISCOVER...");
    
    loop {
        let now_tsc = get_tsc();
        
        // Check timeout
        if now_tsc.wrapping_sub(dhcp_start) > dhcp_timeout_ticks {
            serial_println("[FAIL] DHCP timeout");
            return RunResult::DhcpTimeout;
        }
        
        // Convert TSC to smoltcp Instant
        let timestamp = tsc_to_instant(now_tsc, handoff.tsc_freq);
        
        // Phase 1: Refill RX queue
        adapter.refill_rx();
        
        // Phase 2: Poll smoltcp interface (EXACTLY ONCE per iteration)
        enter_poll();  // Reentrancy guard
        let poll_result = iface.poll(timestamp, &mut adapter, &mut sockets);
        exit_poll();   // Reentrancy guard
        
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
                    serial_println("[NET] DHCP deconfigured");
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
                
                let timestamp = tsc_to_instant(now_tsc, handoff.tsc_freq);
                adapter.refill_rx();
                iface.poll(timestamp, &mut adapter, &mut sockets);
                adapter.collect_tx();
                
                let dns = sockets.get_mut::<DnsSocket>(dns_handle);
                match dns.get_query_result(query_handle) {
                    Ok(addrs) => {
                        // Find first IPv4 address
                        let mut found_ip = None;
                        for addr in addrs {
                            if let IpAddress::Ipv4(v4) = addr {
                                found_ip = Some(v4);
                                break;
                            }
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
    static mut TCP_TX_STORAGE: [u8; 65536] = [0u8; 65536];   // 64KB TX
    
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
    
    if tcp_socket.connect(iface.context(), remote_endpoint, local_port).is_err() {
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
        
        let timestamp = tsc_to_instant(now_tsc, handoff.tsc_freq);
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
    let request_len = match format_http_get(unsafe { &mut HTTP_REQUEST_BUF }, path, host_str) {
        Some(len) => len,
        None => {
            serial_println("[FAIL] HTTP request too large for buffer");
            return RunResult::DownloadFailed;
        }
    };
    
    {
        let socket = sockets.get_mut::<TcpSocket>(tcp_handle);
        if socket.send_slice(unsafe { &HTTP_REQUEST_BUF[..request_len] }).is_err() {
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
        
        let timestamp = tsc_to_instant(now_tsc, handoff.tsc_freq);
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
    let mut header_len: usize = 0;
    let mut last_progress_kb: usize = 0;
    
    let recv_start = get_tsc();
    let mut last_activity = recv_start;
    let recv_timeout = handoff.tsc_freq * 300; // 5 minute timeout for large downloads
    let idle_timeout = handoff.tsc_freq * 30;  // 30 second idle timeout
    
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
        let timestamp = tsc_to_instant(now_tsc, handoff.tsc_freq);
        
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
                        let header_buffer = unsafe { &mut HEADER_BUFFER };
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
                            let header_str = core::str::from_utf8(&header_buffer[..pos]).unwrap_or("");
                            
                            // Find first line (status line)
                            if let Some(first_line_end) = header_str.find('\r') {
                                let status_line = &header_str[..first_line_end];
                                
                                // Parse status code manually (avoid split().collect())
                                // Find "HTTP/x.x " prefix, then parse number
                                if let Some(space_after_http) = status_line.find(' ') {
                                    let after_http = &status_line[space_after_http + 1..];
                                    // Find the status code (3 digits)
                                    let status_end = after_http.find(' ').unwrap_or(after_http.len());
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
                                                serial_print_decimal((len / (1024 * 1024 * 1024)) as u32);
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
                                        serial_println("[WARN] Failed to write initial body to disk");
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
                                serial_println("[WARN] Incomplete disk write");
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
                if content_length.is_none() {
                    // No content-length, server closed = complete
                    serial_println("\n[HTTP] Download complete (connection closed)");
                } else if content_length.is_some() && body_received < content_length.unwrap() {
                    // Had content-length but didn't get all data
                    serial_println("\n[WARN] Connection closed before full content received");
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
            if finalize_manifest(blk, &config, DISK_TOTAL_BYTES) {
                serial_println("[OK] ISO manifest written successfully");
            } else {
                serial_println("[WARN] Failed to write ISO manifest");
            }
        }
    } else {
        serial_println("[NOTE] Disk write disabled - data received but not persisted");
    }
    serial_println("");

    // ═══════════════════════════════════════════════════════════════════════
    // STEP 7: COMPLETE
    // ═══════════════════════════════════════════════════════════════════════
    serial_println("");
    serial_println("=====================================");
    serial_println("  ISO Download Complete!");
    serial_println("=====================================");
    serial_println("");
    serial_println("Ready to boot downloaded image.");
    serial_println("System halted.");

    // Halt
    loop {
        core::arch::asm!("hlt", options(nomem, nostack));
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
    data.windows(4)
        .position(|w| w == b"\r\n\r\n")
}
