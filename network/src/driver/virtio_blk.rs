//! VirtIO block device driver.
//!
//! Provides block I/O for ISO persistence via VirtIO-blk.
//!
//! # Architecture
//!
//! VirtIO-blk uses a single virtqueue for all I/O operations.
//! Each request is a 3-descriptor chain:
//!   1. Request header (16 bytes): type, reserved, sector
//!   2. Data buffer: read/write data
//!   3. Status byte: completion status
//!
//! # Reference
//! VirtIO Spec 1.2 §5.2, NETWORK_IMPL_GUIDE.md §6

use core::ptr;
use crate::driver::block_traits::{BlockDriver, BlockDriverInit, BlockError, BlockCompletion, BlockDeviceInfo};
use crate::types::VirtqueueState;

// ═══════════════════════════════════════════════════════════════════════════
// ASM BINDINGS
// ═══════════════════════════════════════════════════════════════════════════

extern "win64" {
    fn asm_virtio_blk_read_capacity(mmio_base: u64) -> u64;
    fn asm_virtio_blk_read_blk_size(mmio_base: u64) -> u32;
    fn asm_virtio_blk_submit_read(
        vq: *mut VirtqueueState,
        sector: u64,
        data_buf_phys: u64,
        num_sectors: u64,
        header_buf_phys: u64,
        status_buf_phys: u64,
        desc_idx: u16,
    ) -> u32;
    fn asm_virtio_blk_submit_write(
        vq: *mut VirtqueueState,
        sector: u64,
        data_buf_phys: u64,
        num_sectors: u64,
        header_buf_phys: u64,
        status_buf_phys: u64,
        desc_idx: u16,
    ) -> u32;
    fn asm_virtio_blk_poll_complete(
        vq: *mut VirtqueueState,
        result: *mut BlkPollResult,
    ) -> u32;
    fn asm_virtio_blk_notify(vq: *mut VirtqueueState);
}

// Use existing VirtIO init functions
extern "win64" {
    fn asm_virtio_reset(mmio_base: u64) -> u32;
    fn asm_virtio_set_status(mmio_base: u64, status: u8);
    fn asm_virtio_get_status(mmio_base: u64) -> u8;
    fn asm_virtio_read_features(mmio_base: u64) -> u64;
    fn asm_virtio_write_features(mmio_base: u64, features: u64);
    fn asm_virtio_setup_queue(
        mmio_base: u64,
        queue_idx: u32,
        desc_phys: u64,
        avail_phys: u64,
        used_phys: u64,
        queue_size: u32,
    ) -> u32;
}

// ═══════════════════════════════════════════════════════════════════════════
// TYPES
// ═══════════════════════════════════════════════════════════════════════════

/// Result from asm_virtio_blk_poll_complete
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct BlkPollResult {
    pub desc_idx: u16,
    pub status: u8,
    pub _pad: u8,
    pub bytes_written: u32,
}

/// VirtIO-blk request header (16 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtioBlkReqHeader {
    /// Request type: 0=read, 1=write, 4=flush
    pub req_type: u32,
    /// Reserved
    pub reserved: u32,
    /// Starting sector
    pub sector: u64,
}

impl VirtioBlkReqHeader {
    pub const TYPE_IN: u32 = 0;   // Read
    pub const TYPE_OUT: u32 = 1;  // Write
    pub const TYPE_FLUSH: u32 = 4;
}

/// Status codes
pub const VIRTIO_BLK_S_OK: u8 = 0;
pub const VIRTIO_BLK_S_IOERR: u8 = 1;
pub const VIRTIO_BLK_S_UNSUPP: u8 = 2;

/// VirtIO status bits
const VIRTIO_STATUS_ACKNOWLEDGE: u8 = 0x01;
const VIRTIO_STATUS_DRIVER: u8 = 0x02;
const VIRTIO_STATUS_FEATURES_OK: u8 = 0x08;
const VIRTIO_STATUS_DRIVER_OK: u8 = 0x04;
const VIRTIO_STATUS_FAILED: u8 = 0x80;

/// VirtIO-blk feature bits
const VIRTIO_F_VERSION_1: u64 = 1 << 32;
const VIRTIO_BLK_F_SIZE_MAX: u64 = 1 << 1;
const VIRTIO_BLK_F_SEG_MAX: u64 = 1 << 2;
const VIRTIO_BLK_F_BLK_SIZE: u64 = 1 << 6;
const VIRTIO_BLK_F_FLUSH: u64 = 1 << 9;
const VIRTIO_BLK_F_RO: u64 = 1 << 5;

/// Required features
const REQUIRED_FEATURES: u64 = VIRTIO_F_VERSION_1;

/// Desired features
const DESIRED_FEATURES: u64 = VIRTIO_BLK_F_BLK_SIZE | VIRTIO_BLK_F_FLUSH;

// ═══════════════════════════════════════════════════════════════════════════
// CONFIGURATION
// ═══════════════════════════════════════════════════════════════════════════

/// VirtIO-blk driver configuration.
#[derive(Debug, Clone)]
pub struct VirtioBlkConfig {
    /// Queue size (number of descriptors)
    pub queue_size: u16,
    /// Physical address of descriptor table
    pub desc_phys: u64,
    /// Physical address of available ring
    pub avail_phys: u64,
    /// Physical address of used ring
    pub used_phys: u64,
    /// Physical address of request headers (one per descriptor/3)
    pub headers_phys: u64,
    /// Physical address of status bytes (one per descriptor/3)
    pub status_phys: u64,
    /// CPU pointer to headers
    pub headers_cpu: u64,
    /// CPU pointer to status bytes
    pub status_cpu: u64,
    /// Physical address for notify MMIO
    pub notify_addr: u64,
}

/// Initialization errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtioBlkInitError {
    /// Device reset failed
    ResetFailed,
    /// Feature negotiation failed
    FeatureNegotiationFailed,
    /// Queue setup failed
    QueueSetupFailed,
    /// Device reported failure
    DeviceFailed,
    /// Invalid configuration
    InvalidConfig,
}

// ═══════════════════════════════════════════════════════════════════════════
// REQUEST TRACKING
// ═══════════════════════════════════════════════════════════════════════════

/// Track in-flight request.
#[derive(Debug, Clone, Copy)]
struct InFlightRequest {
    /// Caller's request ID
    request_id: u32,
    /// First descriptor index (head of 3-chain)
    desc_idx: u16,
    /// Is this slot in use?
    active: bool,
}

impl Default for InFlightRequest {
    fn default() -> Self {
        Self {
            request_id: 0,
            desc_idx: 0,
            active: false,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// DRIVER
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum in-flight requests (queue_size / 3 since each request uses 3 descriptors)
const MAX_IN_FLIGHT: usize = 32;

/// VirtIO block device driver.
pub struct VirtioBlkDriver {
    /// MMIO base address
    mmio_base: u64,
    /// Negotiated features
    features: u64,
    /// Device info
    info: BlockDeviceInfo,
    /// Virtqueue state
    queue: VirtqueueState,
    /// In-flight requests
    in_flight: [InFlightRequest; MAX_IN_FLIGHT],
    /// Next free descriptor set (each request uses 3)
    next_desc_set: u16,
    /// Headers CPU pointer
    headers_cpu: *mut VirtioBlkReqHeader,
    /// Status CPU pointer
    status_cpu: *mut u8,
}

impl VirtioBlkDriver {
    /// Create and initialize VirtIO-blk driver.
    ///
    /// # Safety
    /// - `mmio_base` must be valid VirtIO-blk MMIO address
    /// - `config` must contain valid physical addresses
    pub unsafe fn new(mmio_base: u64, config: VirtioBlkConfig) -> Result<Self, VirtioBlkInitError> {
        // Validate config
        if config.queue_size < 3 || config.queue_size > 256 {
            return Err(VirtioBlkInitError::InvalidConfig);
        }
        
        // ═══════════════════════════════════════════════════════════════════
        // STEP 1: Reset device
        // ═══════════════════════════════════════════════════════════════════
        asm_virtio_set_status(mmio_base, 0);
        
        // Wait for reset (simple spin - bounded)
        for _ in 0..1_000_000 {
            if asm_virtio_get_status(mmio_base) == 0 {
                break;
            }
            core::hint::spin_loop();
        }
        
        if asm_virtio_get_status(mmio_base) != 0 {
            return Err(VirtioBlkInitError::ResetFailed);
        }
        
        // ═══════════════════════════════════════════════════════════════════
        // STEP 2: Set ACKNOWLEDGE
        // ═══════════════════════════════════════════════════════════════════
        asm_virtio_set_status(mmio_base, VIRTIO_STATUS_ACKNOWLEDGE);
        
        // ═══════════════════════════════════════════════════════════════════
        // STEP 3: Set DRIVER
        // ═══════════════════════════════════════════════════════════════════
        asm_virtio_set_status(mmio_base, VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER);
        
        // ═══════════════════════════════════════════════════════════════════
        // STEP 4: Feature negotiation
        // ═══════════════════════════════════════════════════════════════════
        let device_features = asm_virtio_read_features(mmio_base);
        
        if device_features & REQUIRED_FEATURES != REQUIRED_FEATURES {
            asm_virtio_set_status(mmio_base, VIRTIO_STATUS_FAILED);
            return Err(VirtioBlkInitError::FeatureNegotiationFailed);
        }
        
        let our_features = REQUIRED_FEATURES | (DESIRED_FEATURES & device_features);
        asm_virtio_write_features(mmio_base, our_features);
        
        // ═══════════════════════════════════════════════════════════════════
        // STEP 5: Set FEATURES_OK
        // ═══════════════════════════════════════════════════════════════════
        asm_virtio_set_status(mmio_base, 
            VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_FEATURES_OK);
        
        // Verify features accepted
        let status = asm_virtio_get_status(mmio_base);
        if status & VIRTIO_STATUS_FEATURES_OK == 0 {
            asm_virtio_set_status(mmio_base, VIRTIO_STATUS_FAILED);
            return Err(VirtioBlkInitError::FeatureNegotiationFailed);
        }
        
        // ═══════════════════════════════════════════════════════════════════
        // STEP 6: Setup virtqueue (queue 0)
        // ═══════════════════════════════════════════════════════════════════
        let result = asm_virtio_setup_queue(
            mmio_base,
            0,  // queue index
            config.desc_phys,
            config.avail_phys,
            config.used_phys,
            config.queue_size as u32,
        );
        
        if result != 0 {
            asm_virtio_set_status(mmio_base, VIRTIO_STATUS_FAILED);
            return Err(VirtioBlkInitError::QueueSetupFailed);
        }
        
        // ═══════════════════════════════════════════════════════════════════
        // STEP 7: Set DRIVER_OK
        // ═══════════════════════════════════════════════════════════════════
        asm_virtio_set_status(mmio_base,
            VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | 
            VIRTIO_STATUS_FEATURES_OK | VIRTIO_STATUS_DRIVER_OK);
        
        // ═══════════════════════════════════════════════════════════════════
        // STEP 8: Read device info
        // ═══════════════════════════════════════════════════════════════════
        let capacity = asm_virtio_blk_read_capacity(mmio_base);
        let sector_size = if our_features & VIRTIO_BLK_F_BLK_SIZE != 0 {
            asm_virtio_blk_read_blk_size(mmio_base)
        } else {
            512
        };
        let read_only = device_features & VIRTIO_BLK_F_RO != 0;
        
        let info = BlockDeviceInfo {
            total_sectors: capacity,
            sector_size,
            max_sectors_per_request: 128, // Conservative default
            read_only,
        };
        
        // Build queue state
        let queue = VirtqueueState {
            desc_base: config.desc_phys,
            avail_base: config.avail_phys,
            used_base: config.used_phys,
            queue_size: config.queue_size,
            queue_index: 0,
            _pad: 0,
            notify_addr: config.notify_addr,
            last_used_idx: 0,
            next_avail_idx: 0,
            _pad2: 0,
            desc_cpu_ptr: 0, // Not needed for blk
            buffer_cpu_base: 0,
            buffer_bus_base: 0,
            buffer_size: 0,
            buffer_count: 0,
        };
        
        Ok(Self {
            mmio_base,
            features: our_features,
            info,
            queue,
            in_flight: [InFlightRequest::default(); MAX_IN_FLIGHT],
            next_desc_set: 0,
            headers_cpu: config.headers_cpu as *mut VirtioBlkReqHeader,
            status_cpu: config.status_cpu as *mut u8,
        })
    }
    
    /// Allocate a descriptor set (3 consecutive descriptors).
    fn alloc_desc_set(&mut self) -> Option<(u16, u32)> {
        // Find free slot in in_flight
        for (slot_idx, slot) in self.in_flight.iter_mut().enumerate() {
            if !slot.active {
                let desc_idx = (slot_idx * 3) as u16;
                return Some((desc_idx, slot_idx as u32));
            }
        }
        None
    }
    
    /// Get header physical address for a descriptor set.
    fn header_phys(&self, slot_idx: usize) -> u64 {
        // Assuming headers are contiguous
        let base = self.headers_cpu as u64;
        base + (slot_idx * core::mem::size_of::<VirtioBlkReqHeader>()) as u64
    }
    
    /// Get status physical address for a descriptor set.
    fn status_phys(&self, slot_idx: usize) -> u64 {
        let base = self.status_cpu as u64;
        base + slot_idx as u64
    }
    
    /// Read status byte for a slot.
    fn read_status(&self, slot_idx: usize) -> u8 {
        unsafe { ptr::read_volatile(self.status_cpu.add(slot_idx)) }
    }
}

impl BlockDriver for VirtioBlkDriver {
    fn info(&self) -> BlockDeviceInfo {
        self.info
    }
    
    fn can_submit(&self) -> bool {
        // Check if we have a free descriptor set
        self.in_flight.iter().any(|s| !s.active)
    }
    
    fn submit_read(
        &mut self,
        sector: u64,
        buffer_phys: u64,
        num_sectors: u32,
        request_id: u32,
    ) -> Result<(), BlockError> {
        // Validate
        if sector + num_sectors as u64 > self.info.total_sectors {
            return Err(BlockError::InvalidSector);
        }
        
        // Allocate descriptor set
        let (desc_idx, slot_idx) = self.alloc_desc_set()
            .ok_or(BlockError::QueueFull)?;
        
        // Write header
        let header = VirtioBlkReqHeader {
            req_type: VirtioBlkReqHeader::TYPE_IN,
            reserved: 0,
            sector,
        };
        unsafe {
            ptr::write_volatile(
                self.headers_cpu.add(slot_idx as usize),
                header,
            );
        }
        
        // Initialize status
        unsafe {
            ptr::write_volatile(self.status_cpu.add(slot_idx as usize), 0xFF);
        }
        
        // Submit via ASM
        let result = unsafe {
            asm_virtio_blk_submit_read(
                &mut self.queue,
                sector,
                buffer_phys,
                num_sectors as u64,
                self.header_phys(slot_idx as usize),
                self.status_phys(slot_idx as usize),
                desc_idx,
            )
        };
        
        if result != 0 {
            return Err(BlockError::QueueFull);
        }
        
        // Track in-flight
        self.in_flight[slot_idx as usize] = InFlightRequest {
            request_id,
            desc_idx,
            active: true,
        };
        
        Ok(())
    }
    
    fn submit_write(
        &mut self,
        sector: u64,
        buffer_phys: u64,
        num_sectors: u32,
        request_id: u32,
    ) -> Result<(), BlockError> {
        // Check read-only
        if self.info.read_only {
            return Err(BlockError::ReadOnly);
        }
        
        // Validate
        if sector + num_sectors as u64 > self.info.total_sectors {
            return Err(BlockError::InvalidSector);
        }
        
        // Allocate descriptor set
        let (desc_idx, slot_idx) = self.alloc_desc_set()
            .ok_or(BlockError::QueueFull)?;
        
        // Write header
        let header = VirtioBlkReqHeader {
            req_type: VirtioBlkReqHeader::TYPE_OUT,
            reserved: 0,
            sector,
        };
        unsafe {
            ptr::write_volatile(
                self.headers_cpu.add(slot_idx as usize),
                header,
            );
        }
        
        // Initialize status
        unsafe {
            ptr::write_volatile(self.status_cpu.add(slot_idx as usize), 0xFF);
        }
        
        // Submit via ASM
        let result = unsafe {
            asm_virtio_blk_submit_write(
                &mut self.queue,
                sector,
                buffer_phys,
                num_sectors as u64,
                self.header_phys(slot_idx as usize),
                self.status_phys(slot_idx as usize),
                desc_idx,
            )
        };
        
        if result != 0 {
            return Err(BlockError::QueueFull);
        }
        
        // Track in-flight
        self.in_flight[slot_idx as usize] = InFlightRequest {
            request_id,
            desc_idx,
            active: true,
        };
        
        Ok(())
    }
    
    fn poll_completion(&mut self) -> Option<BlockCompletion> {
        let mut result = BlkPollResult::default();
        
        let has_completion = unsafe {
            asm_virtio_blk_poll_complete(&mut self.queue, &mut result)
        };
        
        if has_completion == 0 {
            return None;
        }
        
        // Find matching in-flight request
        let slot_idx = (result.desc_idx / 3) as usize;
        if slot_idx >= MAX_IN_FLIGHT {
            return None;
        }
        
        // Check if slot is active before doing anything
        if !self.in_flight[slot_idx].active {
            return None;
        }
        
        // Read status BEFORE taking mutable borrow of in_flight slot
        let status = self.read_status(slot_idx);
        
        // Now we can safely mutably borrow to extract info and clear
        let in_flight = &mut self.in_flight[slot_idx];
        let request_id = in_flight.request_id;
        
        // Mark slot as free
        in_flight.active = false;
        
        let completion = BlockCompletion {
            request_id,
            status,
            bytes_transferred: result.bytes_written,
        };
        
        Some(completion)
    }
    
    fn notify(&mut self) {
        unsafe {
            asm_virtio_blk_notify(&mut self.queue);
        }
    }
    
    fn flush(&mut self) -> Result<(), BlockError> {
        if self.features & VIRTIO_BLK_F_FLUSH == 0 {
            return Err(BlockError::Unsupported);
        }
        
        // TODO: Implement flush command
        // For now, consider it always successful
        Ok(())
    }
}

impl BlockDriverInit for VirtioBlkDriver {
    type Error = VirtioBlkInitError;
    type Config = VirtioBlkConfig;
    
    fn supported_vendors() -> &'static [u16] {
        &[0x1AF4] // VirtIO vendor
    }
    
    fn supported_devices() -> &'static [u16] {
        &[0x1001, 0x1042] // virtio-blk legacy and modern
    }
    
    unsafe fn create(mmio_base: u64, config: Self::Config) -> Result<Self, Self::Error> {
        Self::new(mmio_base, config)
    }
}

// Safety: VirtioBlkDriver only contains raw pointers that are not shared
unsafe impl Send for VirtioBlkDriver {}
