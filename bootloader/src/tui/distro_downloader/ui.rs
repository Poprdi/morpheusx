//! Distro Downloader UI
//!
//! Main TUI for browsing, downloading, and managing Linux distributions.
//! Integrates ISO storage management for chunked downloads.
//!
//! Follows the same rendering pattern as main_menu and distro_launcher:
//! - Clear screen once at start
//! - Render initial state
//! - Only re-render after handling input (no clear in render loop)

use alloc::vec::Vec;
use alloc::format;
use alloc::string::ToString;

use super::catalog::{DistroEntry, CATEGORIES, get_by_category};
use super::state::{DownloadState, DownloadStatus, UiState, UiMode};
use crate::tui::input::{InputKey, Keyboard};
use crate::tui::renderer::{
    Screen, EFI_BLACK, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN, EFI_RED,
    EFI_YELLOW, EFI_WHITE, EFI_DARKGRAY,
};
use crate::BootServices;
use crate::uefi::block_io::{BlockIoProtocol, EFI_BLOCK_IO_PROTOCOL_GUID};
use crate::uefi::block_io_adapter::UefiBlockIo;
use morpheus_core::disk::gpt_ops::{find_free_space, create_partition, FreeRegion, GptError};
use morpheus_core::disk::partition::PartitionType;
use morpheus_core::iso::{IsoStorageManager, IsoManifest, ChunkWriter, ChunkInfo, ChunkSet, MAX_ISOS, MAX_CHUNKS, IsoError};

// Layout constants
const HEADER_Y: usize = 0;
const CATEGORY_Y: usize = 3;
const LIST_Y: usize = 5;
const DETAILS_Y: usize = 14;
const FOOTER_Y: usize = 19;
const VISIBLE_ITEMS: usize = 8;

/// Action returned from UI
#[derive(Debug, Clone, Copy)]
pub enum ManageAction {
    /// Continue UI loop
    Continue,
    /// Exit UI
    Exit,
}

/// Main distro downloader UI controller
pub struct DistroDownloader {
    /// UI navigation state
    ui_state: UiState,
    /// Download progress state
    download_state: DownloadState,
    /// Cached list of distros for current category
    current_distros: Vec<&'static DistroEntry>,
    /// Boot services reference (for file operations)
    boot_services: *const BootServices,
    /// Image handle
    image_handle: *mut (),
    /// Track if we need full redraw (mode change, category change)
    needs_full_redraw: bool,
    /// ISO storage manager (for downloaded ISOs)
    iso_storage: IsoStorageManager,
    /// Cached ISO names for display
    iso_names: [[u8; 64]; MAX_ISOS],
    /// Cached ISO name lengths
    iso_name_lens: [usize; MAX_ISOS],
    /// Cached ISO sizes (MB)
    iso_sizes_mb: [u64; MAX_ISOS],
    /// Cached ISO completion status
    iso_complete: [bool; MAX_ISOS],
}

impl DistroDownloader {
    /// Create a new distro downloader
    ///
    /// # Arguments
    /// * `boot_services` - UEFI boot services
    /// * `image_handle` - Current image handle
    /// * `esp_start_lba` - Start LBA of ESP partition (for ISO storage)
    /// * `disk_size_lba` - Total disk size in LBAs
    pub fn new(
        boot_services: *const BootServices,
        image_handle: *mut (),
        esp_start_lba: u64,
        disk_size_lba: u64,
    ) -> Self {
        let ui_state = UiState::new();
        let current_category = ui_state.current_category();
        let current_distros: Vec<_> = get_by_category(current_category).collect();
        let iso_storage = IsoStorageManager::new(esp_start_lba, disk_size_lba);

        let mut this = Self {
            ui_state,
            download_state: DownloadState::new(),
            current_distros,
            boot_services,
            image_handle,
            needs_full_redraw: true,
            iso_storage,
            iso_names: [[0u8; 64]; MAX_ISOS],
            iso_name_lens: [0; MAX_ISOS],
            iso_sizes_mb: [0; MAX_ISOS],
            iso_complete: [false; MAX_ISOS],
        };
        this.refresh_iso_cache();
        this
    }

    /// Refresh ISO cache from storage manager
    fn refresh_iso_cache(&mut self) {
        self.ui_state.update_iso_count(self.iso_storage.count());

        for (i, (_, entry)) in self.iso_storage.iter().enumerate() {
            if i >= MAX_ISOS {
                break;
            }
            let manifest = &entry.manifest;

            // Copy name
            let name_len = manifest.name_len.min(64);
            self.iso_names[i][..name_len].copy_from_slice(&manifest.name[..name_len]);
            self.iso_name_lens[i] = name_len;

            // Size in MB
            self.iso_sizes_mb[i] = manifest.total_size / (1024 * 1024);

            // Completion status
            self.iso_complete[i] = manifest.is_complete();
        }
    }

    /// Get ISO storage manager reference
    pub fn storage(&self) -> &IsoStorageManager {
        &self.iso_storage
    }

    /// Get mutable ISO storage manager reference
    pub fn storage_mut(&mut self) -> &mut IsoStorageManager {
        &mut self.iso_storage
    }

    /// Refresh the distro list for current category
    fn refresh_distro_list(&mut self) {
        let category = self.ui_state.current_category();
        self.current_distros = get_by_category(category).collect();
        self.needs_full_redraw = true;
    }

    /// Get currently selected distro
    pub fn selected_distro(&self) -> Option<&'static DistroEntry> {
        self.current_distros.get(self.ui_state.selected_distro).copied()
    }

    /// Handle input and return action
    fn handle_input(&mut self, key: &InputKey, screen: &mut Screen) -> ManageAction {
        match self.ui_state.mode {
            UiMode::Browse => self.handle_browse_input(key, screen),
            UiMode::Confirm => self.handle_confirm_input(key, screen),
            UiMode::Downloading => self.handle_download_input(key, screen),
            UiMode::Result => self.handle_result_input(key, screen),
            UiMode::Manage => self.handle_manage_input(key, screen),
            UiMode::ConfirmDelete => self.handle_confirm_delete_input(key, screen),
        }
    }

    fn handle_browse_input(&mut self, key: &InputKey, screen: &mut Screen) -> ManageAction {
        match key.scan_code {
            // Up arrow
            0x01 => {
                self.ui_state.prev_distro();
                self.render_list_and_details(screen);
            }
            // Down arrow
            0x02 => {
                let count = self.current_distros.len();
                self.ui_state.next_distro(count);
                self.render_list_and_details(screen);
            }
            // Left arrow - previous category
            0x04 => {
                self.ui_state.prev_category();
                self.refresh_distro_list();
                self.render_full(screen);
            }
            // Right arrow - next category
            0x03 => {
                self.ui_state.next_category(CATEGORIES.len());
                self.refresh_distro_list();
                self.render_full(screen);
            }
            // ESC - exit
            0x17 => {
                return ManageAction::Exit;
            }
            _ => {
                // Enter key - show confirm dialog
                if key.unicode_char == 0x0D && self.selected_distro().is_some() {
                    self.ui_state.show_confirm();
                    self.needs_full_redraw = true;
                    self.render_full(screen);
                }
                // 'm' or 'M' - switch to manage view
                else if key.unicode_char == b'm' as u16 || key.unicode_char == b'M' as u16 {
                    self.refresh_iso_cache();
                    self.ui_state.show_manage();
                    self.needs_full_redraw = true;
                    self.render_full(screen);
                }
            }
        }
        ManageAction::Continue
    }

    fn handle_confirm_input(&mut self, key: &InputKey, screen: &mut Screen) -> ManageAction {
        // ESC - cancel
        if key.scan_code == 0x17 {
            self.ui_state.return_to_browse();
            self.needs_full_redraw = true;
            self.render_full(screen);
            return ManageAction::Continue;
        }

        // Y/y - confirm download
        if key.unicode_char == b'y' as u16 || key.unicode_char == b'Y' as u16 {
            if let Some(distro) = self.selected_distro() {
                self.start_download(distro, screen);
            }
            return ManageAction::Continue;
        }

        // N/n - cancel
        if key.unicode_char == b'n' as u16 || key.unicode_char == b'N' as u16 {
            self.ui_state.return_to_browse();
            self.needs_full_redraw = true;
            self.render_full(screen);
        }

        ManageAction::Continue
    }

    fn handle_download_input(&mut self, key: &InputKey, screen: &mut Screen) -> ManageAction {
        // ESC cancels download
        if key.scan_code == 0x17 {
            self.download_state.fail("Cancelled by user");
            self.ui_state.show_result("Download cancelled");
            self.needs_full_redraw = true;
            self.render_full(screen);
        }
        ManageAction::Continue
    }

    fn handle_result_input(&mut self, key: &InputKey, screen: &mut Screen) -> ManageAction {
        // Any key returns to browse
        if key.scan_code != 0 || key.unicode_char != 0 {
            self.ui_state.return_to_browse();
            self.download_state.reset();
            self.refresh_iso_cache(); // Refresh after download
            self.needs_full_redraw = true;
            self.render_full(screen);
        }
        ManageAction::Continue
    }

    fn handle_manage_input(&mut self, key: &InputKey, screen: &mut Screen) -> ManageAction {
        match key.scan_code {
            // Up arrow
            0x01 => {
                self.ui_state.prev_iso();
                self.render_full(screen);
            }
            // Down arrow
            0x02 => {
                self.ui_state.next_iso();
                self.render_full(screen);
            }
            // ESC - back to browse
            0x17 => {
                self.ui_state.return_from_manage();
                self.needs_full_redraw = true;
                self.render_full(screen);
            }
            _ => {
                // 'd' or 'D' - delete
                if (key.unicode_char == b'd' as u16 || key.unicode_char == b'D' as u16)
                    && self.ui_state.iso_count > 0
                {
                    self.ui_state.show_confirm_delete();
                    self.needs_full_redraw = true;
                    self.render_full(screen);
                }
                // 'r' or 'R' - refresh
                else if key.unicode_char == b'r' as u16 || key.unicode_char == b'R' as u16 {
                    self.refresh_iso_cache();
                    self.needs_full_redraw = true;
                    self.render_full(screen);
                }
            }
        }
        ManageAction::Continue
    }

    fn handle_confirm_delete_input(&mut self, key: &InputKey, screen: &mut Screen) -> ManageAction {
        // Y/y - confirm delete
        if key.unicode_char == b'y' as u16 || key.unicode_char == b'Y' as u16 {
            let idx = self.ui_state.selected_iso;
            if self.iso_storage.remove_entry(idx).is_ok() {
                self.refresh_iso_cache();
            }
            self.ui_state.cancel_confirm();
            self.needs_full_redraw = true;
            self.render_full(screen);
            return ManageAction::Continue;
        }

        // N/n/ESC - cancel
        if key.unicode_char == b'n' as u16
            || key.unicode_char == b'N' as u16
            || key.scan_code == 0x17
        {
            self.ui_state.cancel_confirm();
            self.needs_full_redraw = true;
            self.render_full(screen);
        }

        ManageAction::Continue
    }

    /// Start downloading a distribution
    fn start_download(&mut self, distro: &'static DistroEntry, screen: &mut Screen) {
        self.ui_state.start_download();
        self.download_state.start_check(distro.filename);
        self.needs_full_redraw = true;
        self.render_full(screen);

        // Execute the full download flow
        self.execute_download(distro, screen);
    }

    /// Execute the full ISO download flow
    ///
    /// 1. Check disk space and find free regions
    /// 2. Create chunk partitions
    /// 3. Initialize HTTP client
    /// 4. Download with streaming to chunk writer
    /// 5. Finalize and register ISO
    fn execute_download(&mut self, distro: &'static DistroEntry, screen: &mut Screen) {
        let total_size = distro.size_bytes;
        morpheus_core::logger::log(format!("Starting download: {} ({} bytes)", distro.name, total_size).leak());

        // Step 1: Get block I/O protocol for disk operations
        let block_io_protocol = match Self::get_first_disk_block_io(unsafe { &*self.boot_services }) {
            Some(p) => p,
            None => {
                self.show_download_error(screen, "No disk device found");
                return;
            }
        };

        // Create UEFI block I/O adapter
        let uefi_block_io = unsafe { UefiBlockIo::new(block_io_protocol) };

        // Step 2: Find free space on disk
        let block_size = uefi_block_io.block_size_bytes() as usize;
        let free_regions = match find_free_space(uefi_block_io, block_size) {
            Ok(r) => r,
            Err(e) => {
                self.show_download_error(screen, &format!("Failed to scan disk: {:?}", e));
                return;
            }
        };

        // Calculate chunks needed (4GB per chunk)
        const CHUNK_SIZE: u64 = 4 * 1024 * 1024 * 1024; // 4GB
        let chunks_needed = ((total_size + CHUNK_SIZE - 1) / CHUNK_SIZE) as usize;
        
        if chunks_needed > MAX_CHUNKS {
            self.show_download_error(screen, "ISO too large (max 32GB)");
            return;
        }

        morpheus_core::logger::log(format!("Need {} chunks for {} bytes", chunks_needed, total_size).leak());

        // Step 3: Allocate partitions from free space
        let chunk_partitions = match self.allocate_chunk_partitions(
            &free_regions,
            chunks_needed,
            total_size,
            block_size,
        ) {
            Some(p) => p,
            None => {
                self.show_download_error(screen, "Insufficient disk space");
                return;
            }
        };

        // Step 4: Create GPT partitions for each chunk
        // Re-acquire block_io since we need mutable access
        let block_io_protocol = match Self::get_first_disk_block_io(unsafe { &*self.boot_services }) {
            Some(p) => p,
            None => {
                self.show_download_error(screen, "Lost disk device");
                return;
            }
        };

        for (i, (start_lba, end_lba)) in chunk_partitions.iter().enumerate().take(chunks_needed) {
            morpheus_core::logger::log(format!("Creating partition {}: LBA {} - {}", i, start_lba, end_lba).leak());
            
            let uefi_block_io = unsafe { UefiBlockIo::new(block_io_protocol) };
            if let Err(e) = create_partition(
                uefi_block_io,
                PartitionType::BasicData, // FAT32 data partition
                *start_lba,
                *end_lba,
            ) {
                self.show_download_error(screen, &format!("Failed to create partition {}: {:?}", i, e));
                return;
            }
        }

        // Step 5: Prepare manifest and chunk writer
        let mut manifest = IsoManifest::new(distro.filename, total_size);
        
        // Build chunk set with partition info
        let mut chunks = ChunkSet::new();
        chunks.total_size = total_size;
        chunks.count = chunks_needed;
        
        let mut remaining = total_size;
        for i in 0..chunks_needed {
            let chunk_size = remaining.min(CHUNK_SIZE);
            chunks.chunks[i] = ChunkInfo {
                partition_uuid: [0u8; 16],  // Will be set when partition is created
                start_lba: chunk_partitions[i].0,
                end_lba: chunk_partitions[i].1,
                data_size: chunk_size,
                index: i as u8,
                written: false,
            };
            remaining -= chunk_size;
        }
        manifest.chunks = chunks;

        // Create chunk writer
        let partitions: Vec<_> = chunk_partitions.iter()
            .take(chunks_needed)
            .copied()
            .collect();
        
        let mut chunk_writer = match ChunkWriter::new(total_size, CHUNK_SIZE, &partitions) {
            Ok(w) => w,
            Err(e) => {
                self.show_download_error(screen, &format!("Failed to create writer: {:?}", e));
                return;
            }
        };

        // Step 6: Initialize HTTP client and start download
        self.download_state.start_download(Some(total_size as usize));
        self.render_progress_only(screen);

        // Get fresh block_io for write operations
        let block_io_protocol = match Self::get_first_disk_block_io(unsafe { &*self.boot_services }) {
            Some(p) => p,
            None => {
                self.show_download_error(screen, "Lost disk device");
                return;
            }
        };

        // Try native VirtIO download first (works without UEFI HTTP protocol)
        // Falls back to UEFI HTTP if native fails
        morpheus_core::logger::log("Attempting native network download (VirtIO)...");
        let download_result = self.download_native(
            distro.url,
            &mut chunk_writer,
            block_io_protocol,
            screen,
        ).or_else(|native_err| {
            // Native failed, try UEFI HTTP as fallback
            morpheus_core::logger::log(format!("Native download failed: {}, trying UEFI HTTP...", native_err).leak());
            self.download_with_chunk_writer(
                distro.url,
                &mut chunk_writer,
                block_io_protocol,
                screen,
            )
        });

        match download_result {
            Ok(bytes_written) => {
                morpheus_core::logger::log(format!("Download complete: {} bytes", bytes_written).leak());
                
                // Finalize chunk writer and get final chunk set
                let final_chunks = match chunk_writer.finalize() {
                    Ok(c) => c,
                    Err(e) => {
                        self.show_download_error(screen, &format!("Finalize failed: {:?}", e));
                        return;
                    }
                };

                // Update manifest with final chunks before registering
                manifest.chunks = final_chunks.clone();
                manifest.mark_complete();

                // Persist manifest to ESP filesystem FIRST (so it survives reboots)
                unsafe {
                    let bs = &*self.boot_services;
                    if let Err(e) = super::manifest_io::persist_manifest(bs, self.image_handle, &manifest) {
                        self.show_download_error(screen, &format!("Failed to persist manifest: {:?}", e));
                        return;
                    }
                    morpheus_core::logger::log("Manifest persisted to ESP");
                }

                // Register ISO in storage manager (in-memory cache)
                match self.iso_storage.finalize_download(manifest, final_chunks) {
                    Ok(idx) => {
                        morpheus_core::logger::log(format!("ISO registered at index {}", idx).leak());
                        self.refresh_iso_cache();
                        let msg: &'static str = format!("Download complete: {}", distro.name).leak();
                        self.ui_state.show_result(msg);
                        self.download_state.complete();
                    }
                    Err(e) => {
                        self.show_download_error(screen, &format!("Failed to register ISO: {:?}", e));
                        return;
                    }
                }
            }
            Err(msg) => {
                self.show_download_error(screen, msg);
                return;
            }
        }

        self.needs_full_redraw = true;
        self.render_full(screen);
    }

    /// Allocate chunk partitions from free space regions
    fn allocate_chunk_partitions(
        &self,
        free_regions: &[Option<FreeRegion>; 16],
        chunks_needed: usize,
        total_size: u64,
        block_size: usize,
    ) -> Option<[(u64, u64); MAX_CHUNKS]> {
        const CHUNK_SIZE: u64 = 4 * 1024 * 1024 * 1024; // 4GB
        let sectors_per_chunk = CHUNK_SIZE / block_size as u64;

        let mut partitions = [(0u64, 0u64); MAX_CHUNKS];
        let mut chunks_allocated = 0;
        let mut remaining_bytes = total_size;

        for region in free_regions.iter().flatten() {
            if chunks_allocated >= chunks_needed {
                break;
            }

            let region_size = region.size_lba();
            let mut region_offset = 0u64;

            while chunks_allocated < chunks_needed && region_offset + sectors_per_chunk <= region_size {
                let chunk_bytes = remaining_bytes.min(CHUNK_SIZE);
                let sectors_needed = (chunk_bytes + block_size as u64 - 1) / block_size as u64;

                let start_lba = region.start_lba + region_offset;
                let end_lba = start_lba + sectors_needed - 1;

                partitions[chunks_allocated] = (start_lba, end_lba);
                chunks_allocated += 1;
                remaining_bytes = remaining_bytes.saturating_sub(CHUNK_SIZE);
                region_offset += sectors_per_chunk;
            }
        }

        if chunks_allocated >= chunks_needed {
            Some(partitions)
        } else {
            None
        }
    }

    /// Download URL and write to chunks via ChunkWriter using NATIVE network stack.
    /// This uses our bare-metal TCP/IP stack with VirtIO, running DMA from code caves
    /// in our PE binary - completely firmware-agnostic.
    fn download_with_chunk_writer(
        &mut self,
        url: &str,
        chunk_writer: &mut ChunkWriter,
        block_io_protocol: *mut BlockIoProtocol,
        screen: &mut Screen,
    ) -> Result<usize, &'static str> {
        use dma_pool::DmaPool;
        use morpheus_network::device::hal::StaticHal;
        use morpheus_network::device::pci::{EcamAccess, PciScanner, ecam_bases};
        use morpheus_network::device::virtio::VirtioNetDevice;
        use morpheus_network::device::NetworkDevice;
        use morpheus_network::client::NativeHttpClient;
        use morpheus_network::stack::NetConfig;
        use virtio_drivers::transport::pci::PciTransport;
        use virtio_drivers::transport::pci::bus::{ConfigurationAccess, DeviceFunction, PciRoot};
        use gpt_disk_io::BlockIo;
        use core::cell::Cell;
        use crate::uefi::file_system::get_loaded_image;

        screen.clear();
        screen.put_str_at(5, 2, "=== Native Network Download ===", EFI_LIGHTGREEN, EFI_BLACK);
        
        let mut log_y = 4;

        // Time function for network stack using standalone assembly RDTSC
        // We estimate ~2GHz CPU, so divide by 2_000_000 to get ms
        fn get_time_ms() -> u64 {
            // Use standalone assembly from morpheus_network
            let tsc = unsafe { morpheus_network::read_tsc() };
            // Assume ~2GHz CPU, adjust if needed
            tsc / 2_000_000
        }

        // Step 1: Initialize DMA pool from code caves in our PE binary
        screen.put_str_at(5, log_y, "Initializing DMA pool...", EFI_LIGHTGREEN, EFI_BLACK);
        log_y += 1;
        
        // Get PE load info using embedded relocation data
        use morpheus_persistent::pe::embedded_reloc_data::ORIGINAL_IMAGE_BASE;
        
        let (image_base, image_size) = unsafe {
            let bs = &*self.boot_services;
            match get_loaded_image(bs, self.image_handle) {
                Ok(loaded_image) => {
                    let base = (*loaded_image).image_base as usize;
                    let size = (*loaded_image).image_size as usize;
                    (base, size)
                }
                Err(_) => (0, 0)
            }
        };

        // Calculate relocation delta using compile-time embedded data
        let reloc_delta = if image_base != 0 {
            image_base as i64 - ORIGINAL_IMAGE_BASE as i64
        } else {
            0
        };
        
        // Show detailed PE/DMA info for debugging
        screen.put_str_at(7, log_y, &format!(
            "Linker ImageBase: {:#x}", ORIGINAL_IMAGE_BASE
        ), EFI_DARKGRAY, EFI_BLACK);
        log_y += 1;
        
        screen.put_str_at(7, log_y, &format!(
            "Actual load addr: {:#x} (delta: {:+#x})",
            image_base, reloc_delta
        ), EFI_YELLOW, EFI_BLACK);
        log_y += 1;

        // Check if PE is loaded below 4GB (required for VirtIO transitional 32-bit DMA)
        let pe_under_4gb = image_base < 0x1_0000_0000;
        screen.put_str_at(7, log_y, &format!(
            "PE location: {}",
            if pe_under_4gb { "<4GB (DMA OK)" } else { ">4GB (DMA RISK!)" }
        ), if pe_under_4gb { EFI_LIGHTGREEN } else { EFI_RED }, EFI_BLACK);
        log_y += 1;

        // Initialize static pool - it's in our .bss, so same location as PE
        DmaPool::init_static();
        
        // Show where static pool is located
        let pool_base = DmaPool::base_address();
        let pool_size = DmaPool::total_size();
        let pool_under_4gb = pool_base < 0x1_0000_0000;
        let pool_end = pool_base + pool_size;
        
        screen.put_str_at(7, log_y, &format!(
            "DMA pool: {:#x}-{:#x} ({} KB)",
            pool_base, pool_end, pool_size / 1024
        ), EFI_YELLOW, EFI_BLACK);
        log_y += 1;
        
        screen.put_str_at(7, log_y, &format!(
            "DMA range: {}",
            if pool_under_4gb && pool_end < 0x1_0000_0000 { 
                "<4GB OK" 
            } else { 
                ">4GB - VirtIO may fail!" 
            }
        ), if pool_under_4gb && pool_end < 0x1_0000_0000 { EFI_LIGHTGREEN } else { EFI_RED }, EFI_BLACK);
        log_y += 1;

        // Step 2: Initialize HAL
        screen.put_str_at(5, log_y, "Initializing HAL...", EFI_LIGHTGREEN, EFI_BLACK);
        log_y += 1;
        StaticHal::init();

        // Step 3: Run PCI diagnostics first to understand the environment
        screen.put_str_at(5, log_y, "Running PCI diagnostics...", EFI_LIGHTGREEN, EFI_BLACK);
        log_y += 1;

        use morpheus_network::device::pci::{LegacyIoAccess, diagnostics};
        
        // Run comprehensive diagnostics
        let diag = diagnostics::run_diagnostics();
        
        // Display diagnostic results
        let io_status = if diag.io_port_works { "OK" } else { "FAIL" };
        screen.put_str_at(7, log_y, &format!(
            "I/O ports: {} (0xCF8={:#010x})", io_status, diag.cf8_readback
        ), if diag.io_port_works { EFI_YELLOW } else { EFI_RED }, EFI_BLACK);
        log_y += 1;
        
        screen.put_str_at(7, log_y, &format!(
            "Host bridge: {:04x}:{:04x}", diag.host_bridge_vendor, diag.host_bridge_device
        ), EFI_YELLOW, EFI_BLACK);
        log_y += 1;

        // Show what's at common VirtIO locations
        screen.put_str_at(5, log_y, "All devices on bus 0:", EFI_LIGHTGREEN, EFI_BLACK);
        log_y += 1;
        
        // Show ALL detected devices with their status
        for (loc, vendor, device) in diag.virtio_locations.iter() {
            let status = if *vendor == 0x1AF4 { 
                if *device == 0x1000 || *device == 0x1001 { "VirtIO-NET" }
                else if *device == 0x1041 { "VirtIO-NET(M)" }
                else if *device == 0x1050 { "VirtIO-GPU" }
                else { "VirtIO" }
            } else { 
                "device" 
            };
            let color = if *vendor == 0x1AF4 && (*device == 0x1000 || *device == 0x1001 || *device == 0x1041) {
                EFI_LIGHTGREEN
            } else if *vendor == 0x1AF4 {
                EFI_YELLOW
            } else {
                EFI_DARKGRAY
            };
            screen.put_str_at(7, log_y, &format!(
                "  {:02x}:{:02x}.{}: {:04x}:{:04x} ({})",
                loc.bus, loc.device, loc.function, vendor, device, status
            ), color, EFI_BLACK);
            log_y += 1;
        }

        // Step 4: Full PCI scan
        screen.put_str_at(5, log_y, "Scanning PCI bus 0...", EFI_LIGHTGREEN, EFI_BLACK);
        log_y += 1;
        
        let legacy_io = LegacyIoAccess::new();
        let scanner = PciScanner::new(legacy_io);
        
        // Enumerate all devices on bus 0
        let all_devices = scanner.scan_bus(0);
        screen.put_str_at(7, log_y, &format!("Found: {} devices", all_devices.len()), EFI_YELLOW, EFI_BLACK);
        log_y += 1;
        
        // Show devices found
        for dev in all_devices.iter().take(5) {
            let mark = if dev.vendor_id == 0x1AF4 { "*" } else { " " };
            screen.put_str_at(7, log_y, &format!(
                "{}{:02x}:{:02x}.{} {:04x}:{:04x} {:02x}:{:02x}",
                mark, dev.location.bus, dev.location.device, dev.location.function,
                dev.vendor_id, dev.device_id, dev.class, dev.subclass
            ), EFI_DARKGRAY, EFI_BLACK);
            log_y += 1;
        }
        
        let virtio_devices = scanner.find_virtio_net();
        
        if virtio_devices.is_empty() {
            // Try ECAM fallback
            screen.put_str_at(5, log_y, "Trying ECAM @ 0xB0000000...", EFI_YELLOW, EFI_BLACK);
            log_y += 1;
            
            let ecam_base = ecam_bases::QEMU_Q35;
            let ecam = unsafe { EcamAccess::new(ecam_base as *mut u8) };
            let ecam_scanner = PciScanner::new(ecam);
            let ecam_devices = ecam_scanner.scan_bus(0);
            
            screen.put_str_at(7, log_y, &format!("ECAM: {} devices", ecam_devices.len()), EFI_YELLOW, EFI_BLACK);
            log_y += 1;
            
            for dev in ecam_devices.iter().take(5) {
                let mark = if dev.vendor_id == 0x1AF4 { "*" } else { " " };
                screen.put_str_at(7, log_y, &format!(
                    "{}{:02x}:{:02x}.{} {:04x}:{:04x}",
                    mark, dev.location.bus, dev.location.device, dev.location.function,
                    dev.vendor_id, dev.device_id
                ), EFI_DARKGRAY, EFI_BLACK);
                log_y += 1;
            }
            
            screen.put_str_at(5, log_y + 1, "ERROR: No VirtIO network device found!", EFI_RED, EFI_BLACK);
            screen.put_str_at(5, log_y + 2, "Check QEMU -device virtio-net-pci", EFI_YELLOW, EFI_BLACK);
            screen.put_str_at(5, log_y + 4, "Press any key to continue...", EFI_DARKGRAY, EFI_BLACK);
            
            // Simple busy-wait loop for ~3 seconds so user can see diagnostics
            // This avoids needing keyboard access which requires system_table
            for _ in 0..300_000_000u64 {
                core::hint::spin_loop();
            }
            
            return Err("No VirtIO network device found on PCI bus");
        }

        let device_info = virtio_devices[0];
        screen.put_str_at(5, log_y, &format!(
            "Found VirtIO @ {:02x}:{:02x}.{}",
            device_info.location.bus, device_info.location.device, device_info.location.function
        ), EFI_LIGHTGREEN, EFI_BLACK);
        log_y += 1;

        // Step 4: Create PCI transport
        screen.put_str_at(5, log_y, "Creating PCI transport...", EFI_LIGHTGREEN, EFI_BLACK);
        log_y += 1;
        
        let device_fn = DeviceFunction {
            bus: device_info.location.bus,
            device: device_info.location.device,
            function: device_info.location.function,
        };

        // Legacy I/O configuration access for virtio-drivers
        // Uses standalone assembly from morpheus_network (no inline asm)
        struct LegacyConfigAccess;
        impl ConfigurationAccess for LegacyConfigAccess {
            fn read_word(&self, df: DeviceFunction, reg: u8) -> u32 {
                // Use standalone assembly for PCI config read
                use morpheus_network::device::pci::LegacyIoAccess;
                use morpheus_network::device::pci::ConfigAccess;
                let access = LegacyIoAccess::new();
                let loc = morpheus_network::device::pci::DeviceFunction::new(
                    df.bus, df.device, df.function
                );
                unsafe { access.read32(loc, reg) }
            }
            fn write_word(&mut self, df: DeviceFunction, reg: u8, data: u32) {
                // Use standalone assembly for PCI config write
                use morpheus_network::device::pci::LegacyIoAccess;
                use morpheus_network::device::pci::ConfigAccess;
                let access = LegacyIoAccess::new();
                let loc = morpheus_network::device::pci::DeviceFunction::new(
                    df.bus, df.device, df.function
                );
                unsafe { access.write32(loc, reg, data) }
            }
            unsafe fn unsafe_clone(&self) -> Self { Self }
        }

        // Read BARs for debug using direct PCI access
        let bar0 = LegacyConfigAccess.read_word(device_fn, 0x10);
        let bar1 = LegacyConfigAccess.read_word(device_fn, 0x14);
        let bar2 = LegacyConfigAccess.read_word(device_fn, 0x18);
        let bar4 = LegacyConfigAccess.read_word(device_fn, 0x20);
        screen.put_str_at(7, log_y, &format!(
            "BARs: 0={:#x} 1={:#x} 2={:#x} 4={:#x}",
            bar0, bar1, bar2, bar4
        ), EFI_DARKGRAY, EFI_BLACK);
        log_y += 1;

        let cam = LegacyConfigAccess;
        let mut pci_root = PciRoot::new(cam);
        
        let transport = match PciTransport::new::<StaticHal, LegacyConfigAccess>(&mut pci_root, device_fn) {
            Ok(t) => t,
            Err(e) => {
                screen.put_str_at(5, log_y, &format!("PCI transport failed: {:?}", e), EFI_RED, EFI_BLACK);
                return Err("Failed to create PCI transport");
            }
        };
        
        // Show DMA pool info after transport is created
        let dma_base = DmaPool::base_address();
        let dma_free = DmaPool::free_space();
        screen.put_str_at(7, log_y, &format!(
            "DMA: base={:#x} free={}KB",
            dma_base, dma_free / 1024
        ), EFI_DARKGRAY, EFI_BLACK);
        log_y += 1;

        // Step 5: Create VirtIO network device
        screen.put_str_at(5, log_y, "Creating VirtIO device...", EFI_LIGHTGREEN, EFI_BLACK);
        log_y += 1;
        
        let mut virtio_device = match VirtioNetDevice::<StaticHal, PciTransport>::new(transport) {
            Ok(d) => d,
            Err(e) => {
                screen.put_str_at(5, log_y, &format!("VirtIO failed: {:?}", e), EFI_RED, EFI_BLACK);
                return Err("Failed to create VirtIO network device");
            }
        };
        
        let mac = virtio_device.mac_address();
        screen.put_str_at(7, log_y, &format!(
            "MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        ), EFI_YELLOW, EFI_BLACK);
        log_y += 1;

        // Step 6: DHCP configuration
        screen.put_str_at(5, log_y, "Starting DHCP...", EFI_LIGHTGREEN, EFI_BLACK);
        log_y += 1;
        
        // Test TX directly before creating smoltcp stack
        screen.put_str_at(7, log_y, "Testing raw TX...           ", EFI_YELLOW, EFI_BLACK);
        
        // Create a dummy Ethernet frame (just to test TX path)
        // Broadcast ARP-like frame - minimum 60 bytes for Ethernet
        let test_frame: [u8; 60] = [
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff,  // Dest MAC (broadcast)
            0x52, 0x54, 0x00, 0x12, 0x34, 0x56,  // Src MAC  
            0x08, 0x06,                          // EtherType (ARP)
            // ARP header (28 bytes)
            0x00, 0x01,  // Hardware type (Ethernet)
            0x08, 0x00,  // Protocol type (IPv4)
            0x06,        // Hardware size
            0x04,        // Protocol size
            0x00, 0x01,  // Opcode (request)
            0x52, 0x54, 0x00, 0x12, 0x34, 0x56,  // Sender MAC
            0x0a, 0x00, 0x02, 0x0f,              // Sender IP (10.0.2.15)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // Target MAC (unknown)
            0x0a, 0x00, 0x02, 0x02,              // Target IP (10.0.2.2 - gateway)
            // Padding to reach 60 bytes
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        
        // NetworkDevice trait already imported at line 600
        match virtio_device.transmit(&test_frame) {
            Ok(()) => {
                screen.put_str_at(7, log_y, "Test TX OK!                 ", EFI_LIGHTGREEN, EFI_BLACK);
            }
            Err(e) => {
                screen.put_str_at(7, log_y, &format!("Test TX FAILED: {:?}", e), EFI_RED, EFI_BLACK);
                log_y += 1;
                
                // Wait and return early
                screen.put_str_at(7, log_y, "TX broken - cannot continue", EFI_RED, EFI_BLACK);
                let spin_start = get_time_ms();
                while get_time_ms() - spin_start < 10000 {}
                return Err("VirtIO TX failed");
            }
        }
        log_y += 1;
        
        // Reset TX error counter before starting
        morpheus_network::stack::reset_tx_error_count();
        
        screen.put_str_at(7, log_y, "Creating NativeHttpClient...", EFI_YELLOW, EFI_BLACK);
        let mut client = NativeHttpClient::new(virtio_device, NetConfig::Dhcp, get_time_ms);
        screen.put_str_at(7, log_y, "NativeHttpClient created OK  ", EFI_LIGHTGREEN, EFI_BLACK);
        log_y += 1;
        
        // Try to understand where we hang
        screen.put_str_at(7, log_y, "Calling poll()...           ", EFI_YELLOW, EFI_BLACK);
        
        // Use a manual timeout loop instead of relying on internal timeout
        let poll_start = get_time_ms();
        let poll_timeout_ms = 5000; // 5 second timeout
        
        // Spawn poll in a way we can track
        // Actually, poll() blocks internally, so we can't easily timeout it externally
        // The hang is INSIDE smoltcp or our device driver
        
        // Let's just call poll and see what happens
        client.poll();
        
        let poll_elapsed = get_time_ms() - poll_start;
        screen.put_str_at(7, log_y, &format!("Poll returned in {}ms     ", poll_elapsed), EFI_LIGHTGREEN, EFI_BLACK);
        log_y += 1;
        
        // Check if TX had any errors
        let tx_errors = morpheus_network::stack::tx_error_count();
        screen.put_str_at(7, log_y, &format!("TX errors: {}             ", tx_errors), 
            if tx_errors > 0 { EFI_RED } else { EFI_LIGHTGREEN }, EFI_BLACK);
        log_y += 1;
        
        // Now do the DHCP loop
        let start_time = get_time_ms();
        let mut poll_count = 0u32;
        let mut last_update = start_time;
        
        while !client.is_network_ready() {
            client.poll();
            poll_count += 1;
            
            let now = get_time_ms();
            if now - last_update > 1000 {
                // Update every second with TX error count
                let elapsed = (now - start_time) / 1000;
                let tx_errs = morpheus_network::stack::tx_error_count();
                screen.put_str_at(7, log_y, &format!(
                    "DHCP: {}s, {} polls, {} TX errs", elapsed, poll_count, tx_errs
                ), if tx_errs > 0 { EFI_RED } else { EFI_YELLOW }, EFI_BLACK);
                last_update = now;
            }
            
            if now - start_time > 30_000 {
                log_y += 1;
                let final_tx_errs = morpheus_network::stack::tx_error_count();
                screen.put_str_at(5, log_y, &format!(
                    "DHCP timeout - {} TX errors", final_tx_errs
                ), EFI_RED, EFI_BLACK);
                log_y += 1;
                
                // Spin for 5 seconds so user can see the output
                let spin_start = get_time_ms();
                while get_time_ms() - spin_start < 5000 {}
                
                return Err("DHCP configuration timeout");
            }
        }
        log_y += 1;
        
        if let Some(ip) = client.ip_address() {
            screen.put_str_at(5, log_y, &format!("IP: {}", ip), EFI_LIGHTGREEN, EFI_BLACK);
            log_y += 1;
        }
        screen.put_str_at(5, log_y, "Network ready!", EFI_LIGHTGREEN, EFI_BLACK);
        log_y += 1;

        // Step 7: Download
        screen.put_str_at(5, log_y, "Starting download...", EFI_LIGHTGREEN, EFI_BLACK);
        log_y += 1;
        
        let progress_y = log_y;
        let progress_bytes = Cell::new(0usize);

        let result = client.get_streaming(url, |chunk_data| {
            let mut uefi_block_io = unsafe { UefiBlockIo::new(block_io_protocol) };
            
            chunk_writer.write(chunk_data, |part_start, sector_offset, data| {
                let lba = part_start + sector_offset;
                uefi_block_io.write_blocks(gpt_disk_types::Lba(lba), data)
                    .map_err(|_| IsoError::IoError)
            }).map_err(|_| morpheus_network::error::NetworkError::FileError)?;

            let new_total = progress_bytes.get() + chunk_data.len();
            progress_bytes.set(new_total);
            
            // Update screen every 1MB
            if new_total % (1024 * 1024) < chunk_data.len() {
                screen.put_str_at(7, progress_y, &format!(
                    "Downloaded: {} MB       ",
                    new_total / (1024 * 1024)
                ), EFI_YELLOW, EFI_BLACK);
            }
            Ok(())
        });

        let final_bytes = progress_bytes.get();
        self.download_state.update_progress(final_bytes);

        match result {
            Ok(bytes) => {
                screen.put_str_at(5, progress_y + 1, &format!("Complete: {} bytes", bytes), EFI_LIGHTGREEN, EFI_BLACK);
                Ok(bytes)
            }
            Err(e) => {
                screen.put_str_at(5, progress_y + 1, &format!("Failed: {:?}", e), EFI_RED, EFI_BLACK);
                Err("Download failed")
            }
        }
    }

    /// Legacy UEFI HTTP download - REMOVED
    /// Use download_with_chunk_writer which now uses native stack
    #[allow(dead_code)]
    fn download_native(
        &mut self,
        _url: &str,
        _chunk_writer: &mut ChunkWriter,
        _block_io_protocol: *mut BlockIoProtocol,
        _screen: &mut Screen,
    ) -> Result<usize, &'static str> {
        Err("Use download_with_chunk_writer - it now uses native network stack")
    }

    /// Show download error and return to result mode
    fn show_download_error(&mut self, screen: &mut Screen, msg: &str) {
        morpheus_core::logger::log(format!("Download error: {}", msg).leak());
        // Leak the message to get 'static lifetime - acceptable for rare error paths
        let static_msg: &'static str = alloc::string::String::from(msg).leak();
        self.ui_state.show_result(static_msg);
        self.download_state.fail(static_msg);
        self.needs_full_redraw = true;
        self.render_full(screen);
    }

    /// Get BlockIoProtocol pointer for first physical disk
    fn get_first_disk_block_io(boot_services: &BootServices) -> Option<*mut BlockIoProtocol> {
        // Get buffer size needed for all Block I/O handles
        let mut buffer_size: usize = 0;
        let _ = (boot_services.locate_handle)(
            2, // ByProtocol
            &EFI_BLOCK_IO_PROTOCOL_GUID,
            core::ptr::null(),
            &mut buffer_size,
            core::ptr::null_mut(),
        );

        if buffer_size == 0 {
            return None;
        }

        // Allocate buffer for handles
        let mut handle_buffer: *mut u8 = core::ptr::null_mut();
        let alloc_status = (boot_services.allocate_pool)(2, buffer_size, &mut handle_buffer);

        if alloc_status != 0 || handle_buffer.is_null() {
            return None;
        }

        // Get all Block I/O handles
        let status = (boot_services.locate_handle)(
            2,
            &EFI_BLOCK_IO_PROTOCOL_GUID,
            core::ptr::null(),
            &mut buffer_size,
            handle_buffer as *mut *mut (),
        );

        if status != 0 {
            (boot_services.free_pool)(handle_buffer);
            return None;
        }

        // Iterate through handles and find physical disk
        let handles = handle_buffer as *const *mut ();
        let handle_count = buffer_size / core::mem::size_of::<*mut ()>();

        let mut result = None;
        unsafe {
            for i in 0..handle_count {
                let handle = *handles.add(i);
                let mut block_io_ptr: *mut () = core::ptr::null_mut();

                let proto_status = (boot_services.handle_protocol)(
                    handle,
                    &EFI_BLOCK_IO_PROTOCOL_GUID,
                    &mut block_io_ptr,
                );

                if proto_status == 0 && !block_io_ptr.is_null() {
                    let block_io = &*(block_io_ptr as *const BlockIoProtocol);
                    let media = &*block_io.media;

                    // Only use physical disks, not partitions
                    if !media.logical_partition && media.media_present {
                        result = Some(block_io_ptr as *mut BlockIoProtocol);
                        break;
                    }
                }
            }
        }

        (boot_services.free_pool)(handle_buffer);
        result
    }

    /// Full render - clears screen if needed and draws everything
    fn render_full(&mut self, screen: &mut Screen) {
        if self.needs_full_redraw {
            screen.clear();
            self.needs_full_redraw = false;
        }

        match self.ui_state.mode {
            UiMode::Browse => {
                self.render_header(screen);
                self.render_categories(screen);
                self.render_list(screen);
                self.render_details(screen);
                self.render_footer(screen);
            }
            UiMode::Confirm => {
                self.render_header(screen);
                self.render_confirm_dialog(screen);
            }
            UiMode::Downloading => {
                self.render_header(screen);
                self.render_progress_only(screen);
            }
            UiMode::Result => {
                self.render_header(screen);
                self.render_result(screen);
            }
            UiMode::Manage => {
                self.render_manage_header(screen);
                self.render_iso_list(screen);
                self.render_manage_footer(screen);
            }
            UiMode::ConfirmDelete => {
                self.render_manage_header(screen);
                self.render_iso_list(screen);
                self.render_manage_confirm_dialog(screen, "Delete this ISO?");
            }
        }
    }

    /// Render only the list and details (for navigation - no clear needed)
    fn render_list_and_details(&self, screen: &mut Screen) {
        self.render_list(screen);
        self.render_details(screen);
    }

    fn render_header(&self, screen: &mut Screen) {
        let title = "=== DISTRO DOWNLOADER ===";
        let x = screen.center_x(title.len());
        screen.put_str_at(x, HEADER_Y, title, EFI_LIGHTGREEN, EFI_BLACK);

        let subtitle = "Download Linux distributions to ESP";
        let x = screen.center_x(subtitle.len());
        screen.put_str_at(x, HEADER_Y + 1, subtitle, EFI_DARKGREEN, EFI_BLACK);
    }

    fn render_categories(&self, screen: &mut Screen) {
        let x = 2;
        let y = CATEGORY_Y;
        let mut current_x = x;

        // Clear the category line
        screen.put_str_at(x, y, "                                                                              ", EFI_BLACK, EFI_BLACK);

        screen.put_str_at(x, y, "Category: ", EFI_GREEN, EFI_BLACK);
        current_x += 10;

        for (i, cat) in CATEGORIES.iter().enumerate() {
            let name = cat.name();
            let (fg, bg) = if i == self.ui_state.selected_category {
                (EFI_BLACK, EFI_LIGHTGREEN)
            } else {
                (EFI_GREEN, EFI_BLACK)
            };

            screen.put_str_at(current_x, y, "[", EFI_GREEN, EFI_BLACK);
            current_x += 1;
            screen.put_str_at(current_x, y, name, fg, bg);
            current_x += name.len();
            screen.put_str_at(current_x, y, "]", EFI_GREEN, EFI_BLACK);
            current_x += 2;
        }
    }

    fn render_list(&self, screen: &mut Screen) {
        let x = 2;
        let y = LIST_Y;

        // Column headers
        screen.put_str_at(x + 2, y, "Name              ", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 22, y, "Version   ", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 34, y, "Size         ", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 48, y, "Description                   ", EFI_DARKGREEN, EFI_BLACK);

        // Separator
        screen.put_str_at(x, y + 1, "--------------------------------------------------------------------------------", EFI_DARKGREEN, EFI_BLACK);

        // Clear list area
        for row in 0..VISIBLE_ITEMS {
            screen.put_str_at(x, y + 2 + row, "                                                                                ", EFI_BLACK, EFI_BLACK);
        }

        // Render visible items
        let scroll = self.ui_state.scroll_offset;
        let visible_end = (scroll + VISIBLE_ITEMS).min(self.current_distros.len());

        for (display_idx, list_idx) in (scroll..visible_end).enumerate() {
            let distro = self.current_distros[list_idx];
            let row_y = y + 2 + display_idx;
            let is_selected = list_idx == self.ui_state.selected_distro;

            let (fg, bg) = if is_selected {
                (EFI_BLACK, EFI_LIGHTGREEN)
            } else {
                (EFI_GREEN, EFI_BLACK)
            };

            // Selection indicator
            let marker = if is_selected { ">>" } else { "  " };
            screen.put_str_at(x, row_y, marker, EFI_LIGHTGREEN, EFI_BLACK);

            // Name (padded/truncated to 18 chars)
            let name = Self::pad_or_truncate(distro.name, 18);
            screen.put_str_at(x + 2, row_y, &name, fg, bg);

            // Version (padded/truncated to 10 chars)  
            let version = Self::pad_or_truncate(distro.version, 10);
            screen.put_str_at(x + 22, row_y, &version, fg, bg);

            // Size
            let size = Self::pad_or_truncate(distro.size_str(), 12);
            screen.put_str_at(x + 34, row_y, &size, fg, bg);

            // Description (truncated to 30 chars)
            let desc = Self::pad_or_truncate(distro.description, 30);
            screen.put_str_at(x + 48, row_y, &desc, fg, bg);
        }

        // Scroll indicators
        if scroll > 0 {
            screen.put_str_at(x + 78, y + 2, "^", EFI_LIGHTGREEN, EFI_BLACK);
        } else {
            screen.put_str_at(x + 78, y + 2, " ", EFI_BLACK, EFI_BLACK);
        }
        if visible_end < self.current_distros.len() {
            screen.put_str_at(x + 78, y + 1 + VISIBLE_ITEMS, "v", EFI_LIGHTGREEN, EFI_BLACK);
        } else {
            screen.put_str_at(x + 78, y + 1 + VISIBLE_ITEMS, " ", EFI_BLACK, EFI_BLACK);
        }
    }

    fn render_details(&self, screen: &mut Screen) {
        let x = 2;
        let y = DETAILS_Y;

        // Clear details area
        for row in 0..4 {
            screen.put_str_at(x, y + row, "                                                                                ", EFI_BLACK, EFI_BLACK);
        }

        if let Some(distro) = self.selected_distro() {
            // Box top
            screen.put_str_at(x, y, "+-[ Details ]", EFI_GREEN, EFI_BLACK);
            for i in 14..78 {
                screen.put_str_at(x + i, y, "-", EFI_GREEN, EFI_BLACK);
            }
            screen.put_str_at(x + 78, y, "+", EFI_GREEN, EFI_BLACK);

            // Content line 1
            screen.put_str_at(x, y + 1, "|", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x + 2, y + 1, "Name: ", EFI_DARKGREEN, EFI_BLACK);
            screen.put_str_at(x + 8, y + 1, distro.name, EFI_LIGHTGREEN, EFI_BLACK);
            screen.put_str_at(x + 30, y + 1, "Arch: ", EFI_DARKGREEN, EFI_BLACK);
            screen.put_str_at(x + 36, y + 1, distro.arch, EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x + 50, y + 1, "Live: ", EFI_DARKGREEN, EFI_BLACK);
            screen.put_str_at(x + 56, y + 1, if distro.is_live { "Yes" } else { "No " }, EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x + 78, y + 1, "|", EFI_GREEN, EFI_BLACK);

            // Content line 2 - URL
            screen.put_str_at(x, y + 2, "|", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x + 2, y + 2, "URL: ", EFI_DARKGREEN, EFI_BLACK);
            let url_display = if distro.url.len() > 70 { &distro.url[..70] } else { distro.url };
            screen.put_str_at(x + 7, y + 2, url_display, EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x + 78, y + 2, "|", EFI_GREEN, EFI_BLACK);

            // Box bottom
            screen.put_str_at(x, y + 3, "+", EFI_GREEN, EFI_BLACK);
            for i in 1..78 {
                screen.put_str_at(x + i, y + 3, "-", EFI_GREEN, EFI_BLACK);
            }
            screen.put_str_at(x + 78, y + 3, "+", EFI_GREEN, EFI_BLACK);
        }
    }

    fn render_footer(&self, screen: &mut Screen) {
        let x = 2;
        let y = FOOTER_Y;

        screen.put_str_at(x, y, "+-[ Controls ]", EFI_GREEN, EFI_BLACK);
        for i in 15..78 {
            screen.put_str_at(x + i, y, "-", EFI_GREEN, EFI_BLACK);
        }
        screen.put_str_at(x + 78, y, "+", EFI_GREEN, EFI_BLACK);

        screen.put_str_at(x, y + 1, "|", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 2, y + 1, "[Arrows] Nav", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 17, y + 1, "[ENTER] Download", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 37, y + 1, "[M] Manage ISOs", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 56, y + 1, "[ESC] Back", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 78, y + 1, "|", EFI_GREEN, EFI_BLACK);

        screen.put_str_at(x, y + 2, "+", EFI_GREEN, EFI_BLACK);
        for i in 1..78 {
            screen.put_str_at(x + i, y + 2, "-", EFI_GREEN, EFI_BLACK);
        }
        screen.put_str_at(x + 78, y + 2, "+", EFI_GREEN, EFI_BLACK);
    }

    fn render_confirm_dialog(&self, screen: &mut Screen) {
        if let Some(distro) = self.selected_distro() {
            let x = 10;
            let y = 8;

            // Dialog box using ASCII (more compatible than Unicode box chars)
            screen.put_str_at(x, y,     "+--------------------------------------------------------+", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x, y + 1, "|              CONFIRM DOWNLOAD                          |", EFI_LIGHTGREEN, EFI_BLACK);
            screen.put_str_at(x, y + 2, "+--------------------------------------------------------+", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x, y + 3, "|                                                        |", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x, y + 4, "|                                                        |", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x, y + 5, "|                                                        |", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x, y + 6, "+--------------------------------------------------------+", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x, y + 7, "|     Download to /isos/ on ESP?    [Y]es   [N]o         |", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x, y + 8, "+--------------------------------------------------------+", EFI_GREEN, EFI_BLACK);

            // Content
            screen.put_str_at(x + 3, y + 3, "Distro: ", EFI_DARKGREEN, EFI_BLACK);
            screen.put_str_at(x + 11, y + 3, distro.name, EFI_LIGHTGREEN, EFI_BLACK);

            screen.put_str_at(x + 3, y + 4, "Size:   ", EFI_DARKGREEN, EFI_BLACK);
            screen.put_str_at(x + 11, y + 4, distro.size_str(), EFI_GREEN, EFI_BLACK);

            screen.put_str_at(x + 3, y + 5, "File:   ", EFI_DARKGREEN, EFI_BLACK);
            let filename = if distro.filename.len() > 40 { &distro.filename[..40] } else { distro.filename };
            screen.put_str_at(x + 11, y + 5, filename, EFI_GREEN, EFI_BLACK);
        }
    }

    fn render_progress_only(&self, screen: &mut Screen) {
        if let Some(distro) = self.selected_distro() {
            let x = 10;
            let y = 8;

            screen.put_str_at(x, y, "Downloading: ", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x + 13, y, distro.name, EFI_LIGHTGREEN, EFI_BLACK);

            // Progress bar
            let bar_width = 50;
            let progress = self.download_state.progress_percent();
            let filled = (bar_width * progress) / 100;

            screen.put_str_at(x, y + 2, "[", EFI_GREEN, EFI_BLACK);
            for i in 0..bar_width {
                let ch = if i < filled { "=" } else if i == filled { ">" } else { " " };
                screen.put_str_at(x + 1 + i, y + 2, ch, EFI_LIGHTGREEN, EFI_BLACK);
            }
            screen.put_str_at(x + 1 + bar_width, y + 2, "]", EFI_GREEN, EFI_BLACK);

            // Status
            screen.put_str_at(x, y + 4, "Status: ", EFI_DARKGREEN, EFI_BLACK);
            screen.put_str_at(x + 8, y + 4, self.download_state.status.as_str(), EFI_GREEN, EFI_BLACK);
        }
    }

    fn render_result(&self, screen: &mut Screen) {
        let x = 10;
        let y = 10;

        if self.download_state.status == DownloadStatus::Complete {
            screen.put_str_at(x, y, "SUCCESS: ", EFI_LIGHTGREEN, EFI_BLACK);
            let msg = self.ui_state.status_message.unwrap_or("Download complete!");
            screen.put_str_at(x + 9, y, msg, EFI_LIGHTGREEN, EFI_BLACK);
        } else {
            screen.put_str_at(x, y, "FAILED: ", EFI_RED, EFI_BLACK);
            let msg = self.download_state.error_message.unwrap_or("Download failed");
            screen.put_str_at(x + 8, y, msg, EFI_RED, EFI_BLACK);
        }

        screen.put_str_at(x, y + 2, "Press any key to continue...", EFI_DARKGREEN, EFI_BLACK);
    }

    /// Helper: pad or truncate string to exact length
    fn pad_or_truncate(s: &str, len: usize) -> alloc::string::String {
        use alloc::string::String;
        let mut result = String::with_capacity(len);
        for (i, c) in s.chars().enumerate() {
            if i >= len {
                break;
            }
            result.push(c);
        }
        while result.len() < len {
            result.push(' ');
        }
        result
    }

    /// Main event loop - follows same pattern as main_menu/distro_launcher
    pub fn run(&mut self, screen: &mut Screen, keyboard: &mut Keyboard) {
        // Initial render with clear
        self.needs_full_redraw = true;
        self.render_full(screen);

        loop {
            // Render global rain if active
            crate::tui::rain::render_rain(screen);

            // Poll for input with frame delay (~60fps timing)
            if let Some(key) = keyboard.poll_key_with_delay() {
                // Global rain toggle
                if key.unicode_char == b'x' as u16 || key.unicode_char == b'X' as u16 {
                    crate::tui::rain::toggle_rain(screen);
                    self.needs_full_redraw = true;
                    self.render_full(screen);
                    continue;
                }

                // Handle mode-specific input
                match self.handle_input(&key, screen) {
                    ManageAction::Continue => {}
                    ManageAction::Exit => return,
                }
            }
        }
    }

    // =========================================================================
    // ISO Manager Rendering
    // =========================================================================

    fn render_manage_header(&self, screen: &mut Screen) {
        let title = "=== ISO MANAGER ===";
        let x = screen.center_x(title.len());
        screen.put_str_at(x, HEADER_Y, title, EFI_LIGHTGREEN, EFI_BLACK);

        let subtitle = "Manage downloaded ISO images  |  Press [ESC] to return";
        let x = screen.center_x(subtitle.len());
        screen.put_str_at(x, HEADER_Y + 1, subtitle, EFI_DARKGREEN, EFI_BLACK);
    }

    fn render_iso_list(&self, screen: &mut Screen) {
        let x = 2;
        let y = 4;

        if self.ui_state.iso_count == 0 {
            screen.put_str_at(x, y, "No ISOs stored.", EFI_DARKGRAY, EFI_BLACK);
            screen.put_str_at(x, y + 1, "Download distros from the Browse view to see them here.", EFI_DARKGRAY, EFI_BLACK);
            return;
        }

        // Column headers
        screen.put_str_at(x + 2, y, "NAME                                    ", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 44, y, "SIZE (MB)", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 58, y, "STATUS", EFI_DARKGREEN, EFI_BLACK);

        // Separator
        screen.put_str_at(x, y + 1, "------------------------------------------------------------------------", EFI_DARKGREEN, EFI_BLACK);

        // List ISOs
        for i in 0..self.ui_state.iso_count {
            let row_y = y + 2 + i;

            // Selection indicator
            if i == self.ui_state.selected_iso {
                screen.put_str_at(x, row_y, ">>", EFI_LIGHTGREEN, EFI_BLACK);
            } else {
                screen.put_str_at(x, row_y, "  ", EFI_BLACK, EFI_BLACK);
            }

            // Name (max 40 chars)
            let name = core::str::from_utf8(&self.iso_names[i][..self.iso_name_lens[i].min(40)])
                .unwrap_or("???");
            
            let (fg, bg) = if i == self.ui_state.selected_iso {
                (EFI_BLACK, EFI_GREEN)
            } else {
                (EFI_LIGHTGREEN, EFI_BLACK)
            };
            
            let name_padded = Self::pad_or_truncate(name, 40);
            screen.put_str_at(x + 2, row_y, &name_padded, fg, bg);

            // Size
            let size_str = Self::format_size_mb(self.iso_sizes_mb[i]);
            screen.put_str_at(x + 44, row_y, &size_str, EFI_GREEN, EFI_BLACK);

            // Status
            if self.iso_complete[i] {
                screen.put_str_at(x + 58, row_y, "Ready   ", EFI_GREEN, EFI_BLACK);
            } else {
                screen.put_str_at(x + 58, row_y, "Partial ", EFI_YELLOW, EFI_BLACK);
            }
        }
    }

    fn render_manage_footer(&self, screen: &mut Screen) {
        let x = 2;
        let y = FOOTER_Y;

        screen.put_str_at(x, y, "+-[ Controls ]", EFI_GREEN, EFI_BLACK);
        for i in 15..70 {
            screen.put_str_at(x + i, y, "-", EFI_GREEN, EFI_BLACK);
        }
        screen.put_str_at(x + 70, y, "+", EFI_GREEN, EFI_BLACK);

        screen.put_str_at(x, y + 1, "|", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x + 2, y + 1, "[UP/DOWN] Select", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 22, y + 1, "[D] Delete", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 38, y + 1, "[R] Refresh", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 54, y + 1, "[ESC] Back", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(x + 70, y + 1, "|", EFI_GREEN, EFI_BLACK);

        screen.put_str_at(x, y + 2, "+", EFI_GREEN, EFI_BLACK);
        for i in 1..70 {
            screen.put_str_at(x + i, y + 2, "-", EFI_GREEN, EFI_BLACK);
        }
        screen.put_str_at(x + 70, y + 2, "+", EFI_GREEN, EFI_BLACK);
    }

    fn render_manage_confirm_dialog(&self, screen: &mut Screen, message: &str) {
        let x = 15;
        let y = 10;

        // Get selected ISO name
        let name = if self.ui_state.selected_iso < self.ui_state.iso_count {
            core::str::from_utf8(
                &self.iso_names[self.ui_state.selected_iso]
                    [..self.iso_name_lens[self.ui_state.selected_iso].min(40)],
            )
            .unwrap_or("???")
        } else {
            "???"
        };

        screen.put_str_at(x, y,     "+--------------------------------------------------+", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x, y + 1, "|                    CONFIRM                       |", EFI_LIGHTGREEN, EFI_BLACK);
        screen.put_str_at(x, y + 2, "+--------------------------------------------------+", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x, y + 3, "|                                                  |", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x, y + 4, "|                                                  |", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x, y + 5, "+--------------------------------------------------+", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x, y + 6, "|               [Y]es       [N]o                   |", EFI_GREEN, EFI_BLACK);
        screen.put_str_at(x, y + 7, "+--------------------------------------------------+", EFI_GREEN, EFI_BLACK);

        screen.put_str_at(x + 3, y + 3, message, EFI_WHITE, EFI_BLACK);
        screen.put_str_at(x + 3, y + 4, name, EFI_LIGHTGREEN, EFI_BLACK);
    }

    fn format_size_mb(mb: u64) -> alloc::string::String {
        use alloc::string::String;
        use core::fmt::Write;
        let mut s = String::with_capacity(12);
        let _ = write!(s, "{:>8}", mb);
        s
    }
}

// ============================================================================
// Unit Tests (Pure Rust, no UEFI)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Mock/test helpers - we test the logic without UEFI

    #[test]
    fn test_refresh_distro_list_changes_with_category() {
        // Test that changing category changes the distro list
        let mut ui_state = UiState::new();
        
        let cat1 = ui_state.current_category();
        let distros1: Vec<_> = get_by_category(cat1).collect();
        
        ui_state.next_category(CATEGORIES.len());
        let cat2 = ui_state.current_category();
        let distros2: Vec<_> = get_by_category(cat2).collect();

        // Different categories should have different distros (usually)
        assert_ne!(cat1, cat2);
        // Lists may or may not be same size, but categories differ
    }

    #[test]
    fn test_selected_distro_within_bounds() {
        let ui_state = UiState::new();
        let current_distros: Vec<_> = get_by_category(ui_state.current_category()).collect();
        
        // Initial selection should be valid
        assert!(ui_state.selected_distro < current_distros.len() || current_distros.is_empty());
    }

    #[test]
    fn test_download_state_lifecycle() {
        let mut download_state = DownloadState::new();
        
        // Start check
        download_state.start_check("test.iso");
        assert_eq!(download_state.status, DownloadStatus::Checking);
        
        // Start download
        download_state.start_download(Some(1000));
        assert_eq!(download_state.status, DownloadStatus::Downloading);
        
        // Progress updates
        download_state.update_progress(500);
        assert_eq!(download_state.progress_percent(), 50);
        
        // Complete
        download_state.complete();
        assert_eq!(download_state.status, DownloadStatus::Complete);
    }

    #[test]
    fn test_download_with_retry() {
        let mut download_state = DownloadState::new();
        
        download_state.start_check("test.iso");
        download_state.fail("Connection refused");
        
        // Try next mirror
        assert!(download_state.try_next_mirror(3));
        assert_eq!(download_state.status, DownloadStatus::Checking);
        assert_eq!(download_state.mirror_index, 1);
        
        // Fail again
        download_state.fail("Timeout");
        assert!(download_state.try_next_mirror(3));
        assert_eq!(download_state.mirror_index, 2);
        
        // No more mirrors
        download_state.fail("Error");
        assert!(!download_state.try_next_mirror(3));
    }

    #[test]
    fn test_ui_mode_transitions_browse_to_confirm() {
        let mut ui_state = UiState::new();
        assert!(ui_state.is_browsing());
        
        ui_state.show_confirm();
        assert!(ui_state.is_confirming());
        
        ui_state.return_to_browse();
        assert!(ui_state.is_browsing());
    }

    #[test]
    fn test_ui_mode_transitions_confirm_to_download() {
        let mut ui_state = UiState::new();
        
        ui_state.show_confirm();
        ui_state.start_download();
        assert!(ui_state.is_downloading());
    }

    #[test]
    fn test_navigation_through_categories() {
        let mut ui_state = UiState::new();
        let num_cats = CATEGORIES.len();
        
        // Navigate forward through all categories
        for i in 0..num_cats - 1 {
            assert_eq!(ui_state.selected_category, i);
            ui_state.next_category(num_cats);
        }
        
        // At last category
        assert_eq!(ui_state.selected_category, num_cats - 1);
        
        // Navigate back
        for i in (0..num_cats - 1).rev() {
            ui_state.prev_category();
            assert_eq!(ui_state.selected_category, i);
        }
    }

    #[test]
    fn test_navigation_resets_selection() {
        let mut ui_state = UiState::new();
        let num_cats = CATEGORIES.len();
        
        // Select some distro
        ui_state.selected_distro = 5;
        ui_state.scroll_offset = 2;
        
        // Change category
        ui_state.next_category(num_cats);
        
        // Selection should reset
        assert_eq!(ui_state.selected_distro, 0);
        assert_eq!(ui_state.scroll_offset, 0);
    }

    #[test]
    fn test_catalog_has_all_categories() {
        for category in CATEGORIES {
            let count = get_by_category(*category).count();
            // Each category should have at least one distro
            // (Server might only have Ubuntu Server)
            assert!(count >= 1, "Category {:?} has no distros", category);
        }
    }
}
