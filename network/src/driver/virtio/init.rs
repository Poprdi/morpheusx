//! VirtIO initialization sequence.
//!
//! # Initialization Steps
//! 1. Reset device
//! 2. Set ACKNOWLEDGE
//! 3. Set DRIVER
//! 4. Feature negotiation
//! 5. Set FEATURES_OK
//! 6. Verify FEATURES_OK
//! 7. Configure virtqueues
//! 8. Pre-fill RX queue
//! 9. Set DRIVER_OK
//! 10. Read MAC address
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §4.5

use super::config::{status, negotiate_features, features, VirtioConfig};
use super::transport::{VirtioTransport, TransportType};
use crate::types::{VirtqueueState, MacAddress};
use crate::driver::traits::RxError;

/// VirtIO initialization error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtioInitError {
    /// Device reset timed out.
    ResetTimeout,
    /// Feature negotiation failed.
    FeatureNegotiationFailed,
    /// Device rejected features.
    FeaturesRejected,
    /// Queue setup failed.
    QueueSetupFailed,
    /// RX prefill failed.
    RxPrefillFailed(usize),
    /// Device error.
    DeviceError,
}

impl From<RxError> for VirtioInitError {
    fn from(_err: RxError) -> Self {
        VirtioInitError::DeviceError
    }
}

/// Initialize VirtIO network device.
///
/// # Arguments
/// - `mmio_base`: MMIO base address from PCI BAR
/// - `config`: Pre-allocated DMA configuration
///
/// # Returns
/// Tuple of (negotiated_features, rx_queue_state, tx_queue_state, mac_address)
///
/// # Safety
/// - `mmio_base` must be valid VirtIO MMIO address
/// - DMA region must be properly allocated
#[cfg(target_arch = "x86_64")]
pub unsafe fn virtio_net_init(
    mmio_base: u64,
    config: &VirtioConfig,
) -> Result<(u64, VirtqueueState, VirtqueueState, MacAddress), VirtioInitError> {
    use crate::asm::drivers::virtio::{device, queue};
    
    // ═══════════════════════════════════════════════════════════
    // STEP 1: RESET DEVICE
    // ═══════════════════════════════════════════════════════════
    let reset_result = device::reset(mmio_base);
    if !reset_result {
        return Err(VirtioInitError::ResetTimeout);
    }
    
    // ═══════════════════════════════════════════════════════════
    // STEP 2: SET ACKNOWLEDGE
    // ═══════════════════════════════════════════════════════════
    device::set_status(mmio_base, status::ACKNOWLEDGE);
    
    // ═══════════════════════════════════════════════════════════
    // STEP 3: SET DRIVER
    // ═══════════════════════════════════════════════════════════
    device::set_status(mmio_base, status::ACKNOWLEDGE | status::DRIVER);
    
    // ═══════════════════════════════════════════════════════════
    // STEP 4: FEATURE NEGOTIATION
    // ═══════════════════════════════════════════════════════════
    let device_features = device::read_features(mmio_base);
    let our_features = negotiate_features(device_features)
        .map_err(|_| VirtioInitError::FeatureNegotiationFailed)?;
    device::write_features(mmio_base, our_features);
    
    // ═══════════════════════════════════════════════════════════
    // STEP 5: SET FEATURES_OK
    // ═══════════════════════════════════════════════════════════
    device::set_status(
        mmio_base,
        status::ACKNOWLEDGE | status::DRIVER | status::FEATURES_OK,
    );
    
    // ═══════════════════════════════════════════════════════════
    // STEP 6: VERIFY FEATURES_OK
    // ═══════════════════════════════════════════════════════════
    let current_status = device::get_status(mmio_base);
    if current_status & status::FEATURES_OK == 0 {
        device::set_status(mmio_base, status::FAILED);
        return Err(VirtioInitError::FeaturesRejected);
    }
    
    // ═══════════════════════════════════════════════════════════
    // STEP 7: CONFIGURE VIRTQUEUES
    // ═══════════════════════════════════════════════════════════
    
    // Setup RX queue (index 0)
    let rx_queue = setup_queue(mmio_base, 0, config)?;
    
    // Setup TX queue (index 1)
    let tx_queue = setup_queue(mmio_base, 1, config)?;
    
    // ═══════════════════════════════════════════════════════════
    // STEP 8: PRE-FILL RX QUEUE (deferred to driver)
    // ═══════════════════════════════════════════════════════════
    // RX prefill is done by the driver after queue setup
    
    // ═══════════════════════════════════════════════════════════
    // STEP 9: SET DRIVER_OK
    // ═══════════════════════════════════════════════════════════
    device::set_status(
        mmio_base,
        status::ACKNOWLEDGE | status::DRIVER | status::FEATURES_OK | status::DRIVER_OK,
    );
    
    // ═══════════════════════════════════════════════════════════
    // STEP 10: READ MAC ADDRESS
    // ═══════════════════════════════════════════════════════════
    let mac = if our_features & features::VIRTIO_NET_F_MAC != 0 {
        device::read_mac(mmio_base).unwrap_or_else(generate_local_mac)
    } else {
        generate_local_mac()
    };
    
    Ok((our_features, rx_queue, tx_queue, mac))
}

/// Setup a single virtqueue.
#[cfg(target_arch = "x86_64")]
unsafe fn setup_queue(
    mmio_base: u64,
    queue_index: u16,
    config: &VirtioConfig,
) -> Result<VirtqueueState, VirtioInitError> {
    use crate::asm::drivers::virtio::queue;
    use crate::dma::DmaRegion;
    
    // Select queue
    queue::select(mmio_base, queue_index);
    
    // Get queue size from device
    let device_queue_size = queue::get_size(mmio_base);
    let queue_size = core::cmp::min(device_queue_size, config.queue_size);
    
    if queue_size == 0 {
        return Err(VirtioInitError::QueueSetupFailed);
    }
    
    // Calculate offsets based on queue index
    let (desc_offset, avail_offset, used_offset, buffer_offset) = if queue_index == 0 {
        // RX queue
        (
            DmaRegion::RX_DESC_OFFSET,
            DmaRegion::RX_AVAIL_OFFSET,
            DmaRegion::RX_USED_OFFSET,
            DmaRegion::RX_BUFFERS_OFFSET,
        )
    } else {
        // TX queue
        (
            DmaRegion::TX_DESC_OFFSET,
            DmaRegion::TX_AVAIL_OFFSET,
            DmaRegion::TX_USED_OFFSET,
            DmaRegion::TX_BUFFERS_OFFSET,
        )
    };
    
    // Calculate bus addresses
    let desc_bus = config.dma_bus_base + desc_offset as u64;
    let avail_bus = config.dma_bus_base + avail_offset as u64;
    let used_bus = config.dma_bus_base + used_offset as u64;
    let buffer_bus = config.dma_bus_base + buffer_offset as u64;
    
    // Calculate CPU pointers
    let desc_cpu = config.dma_cpu_base.add(desc_offset);
    let avail_cpu = config.dma_cpu_base.add(avail_offset);
    let used_cpu = config.dma_cpu_base.add(used_offset);
    let buffer_cpu = config.dma_cpu_base.add(buffer_offset);
    
    // Set queue size
    queue::set_size(mmio_base, queue_size);
    
    // Set queue addresses
    queue::set_desc_addr(mmio_base, desc_bus);
    queue::set_driver_addr(mmio_base, avail_bus);
    queue::set_device_addr(mmio_base, used_bus);
    
    // Enable queue
    queue::enable(mmio_base);
    
    // Get notify offset
    let notify_offset = queue::get_notify_offset(mmio_base);
    let notify_addr = mmio_base + notify_offset as u64;
    
    // Create queue state
    Ok(VirtqueueState {
        desc_base: desc_bus,
        avail_base: avail_bus,
        used_base: used_bus,
        queue_size,
        queue_index,
        _pad: 0,
        notify_addr,
        last_used_idx: 0,
        next_avail_idx: 0,
        _pad2: 0,
        desc_cpu_ptr: desc_cpu as u64,
        buffer_cpu_base: buffer_cpu as u64,
        buffer_bus_base: buffer_bus,
        buffer_size: config.buffer_size as u32,
        buffer_count: queue_size as u32,
    })
}

/// Generate a locally-administered MAC address.
fn generate_local_mac() -> MacAddress {
    // Use a simple deterministic MAC for now
    // In production, should use TSC or other entropy source
    [0x02, 0x00, 0x00, 0x00, 0x00, 0x01]
}

// ═══════════════════════════════════════════════════════════════════════════
// TRANSPORT-AWARE INITIALIZATION
// ═══════════════════════════════════════════════════════════════════════════

/// Initialize VirtIO network device using transport abstraction.
///
/// This function auto-selects the correct initialization path based
/// on the transport type (MMIO or PCI Modern).
///
/// # Arguments
/// - `transport`: Transport handle (already configured with addresses)
/// - `config`: Pre-allocated DMA configuration
/// - `tsc_freq`: TSC frequency for timeout calculations
///
/// # Returns
/// Tuple of (negotiated_features, rx_queue_state, tx_queue_state, mac_address)
#[cfg(target_arch = "x86_64")]
pub unsafe fn virtio_net_init_transport(
    transport: &VirtioTransport,
    config: &VirtioConfig,
    tsc_freq: u64,
) -> Result<(u64, VirtqueueState, VirtqueueState, MacAddress), VirtioInitError> {
    
    // ═══════════════════════════════════════════════════════════
    // STEP 1: RESET DEVICE
    // ═══════════════════════════════════════════════════════════
    if !transport.reset(tsc_freq) {
        return Err(VirtioInitError::ResetTimeout);
    }
    
    // ═══════════════════════════════════════════════════════════
    // STEP 2: SET ACKNOWLEDGE
    // ═══════════════════════════════════════════════════════════
    transport.set_status(status::ACKNOWLEDGE);
    
    // ═══════════════════════════════════════════════════════════
    // STEP 3: SET DRIVER
    // ═══════════════════════════════════════════════════════════
    transport.set_status(status::ACKNOWLEDGE | status::DRIVER);
    
    // ═══════════════════════════════════════════════════════════
    // STEP 4: FEATURE NEGOTIATION
    // ═══════════════════════════════════════════════════════════
    let device_features = transport.read_features();
    let our_features = negotiate_features(device_features)
        .map_err(|_| VirtioInitError::FeatureNegotiationFailed)?;
    transport.write_features(our_features);
    
    // ═══════════════════════════════════════════════════════════
    // STEP 5: SET FEATURES_OK
    // ═══════════════════════════════════════════════════════════
    transport.set_status(
        status::ACKNOWLEDGE | status::DRIVER | status::FEATURES_OK,
    );
    
    // ═══════════════════════════════════════════════════════════
    // STEP 6: VERIFY FEATURES_OK
    // ═══════════════════════════════════════════════════════════
    let current_status = transport.get_status();
    if current_status & status::FEATURES_OK == 0 {
        transport.set_status(status::FAILED);
        return Err(VirtioInitError::FeaturesRejected);
    }
    
    // ═══════════════════════════════════════════════════════════
    // STEP 7: CONFIGURE VIRTQUEUES
    // ═══════════════════════════════════════════════════════════
    
    // Setup RX queue (index 0)
    let rx_queue = setup_queue_transport(transport, 0, config)?;
    
    // Setup TX queue (index 1)
    let tx_queue = setup_queue_transport(transport, 1, config)?;
    
    // ═══════════════════════════════════════════════════════════
    // STEP 9: SET DRIVER_OK
    // ═══════════════════════════════════════════════════════════
    transport.set_status(
        status::ACKNOWLEDGE | status::DRIVER | status::FEATURES_OK | status::DRIVER_OK,
    );
    
    // ═══════════════════════════════════════════════════════════
    // STEP 10: READ MAC ADDRESS
    // ═══════════════════════════════════════════════════════════
    let mac = if our_features & features::VIRTIO_NET_F_MAC != 0 {
        let mut mac_buf = [0u8; 6];
        if transport.read_mac(&mut mac_buf) {
            mac_buf
        } else {
            generate_local_mac()
        }
    } else {
        generate_local_mac()
    };
    
    Ok((our_features, rx_queue, tx_queue, mac))
}

/// Setup a single virtqueue using transport abstraction.
#[cfg(target_arch = "x86_64")]
unsafe fn setup_queue_transport(
    transport: &VirtioTransport,
    queue_index: u16,
    config: &VirtioConfig,
) -> Result<VirtqueueState, VirtioInitError> {
    use crate::dma::DmaRegion;
    
    // Select queue
    transport.select_queue(queue_index);
    
    // Get queue size from device
    let device_queue_size = transport.get_queue_size();
    let queue_size = core::cmp::min(device_queue_size, config.queue_size);
    
    if queue_size == 0 {
        return Err(VirtioInitError::QueueSetupFailed);
    }
    
    // Calculate offsets based on queue index
    let (desc_offset, avail_offset, used_offset, buffer_offset) = if queue_index == 0 {
        // RX queue
        (
            DmaRegion::RX_DESC_OFFSET,
            DmaRegion::RX_AVAIL_OFFSET,
            DmaRegion::RX_USED_OFFSET,
            DmaRegion::RX_BUFFERS_OFFSET,
        )
    } else {
        // TX queue
        (
            DmaRegion::TX_DESC_OFFSET,
            DmaRegion::TX_AVAIL_OFFSET,
            DmaRegion::TX_USED_OFFSET,
            DmaRegion::TX_BUFFERS_OFFSET,
        )
    };
    
    // Calculate bus addresses
    let desc_bus = config.dma_bus_base + desc_offset as u64;
    let avail_bus = config.dma_bus_base + avail_offset as u64;
    let used_bus = config.dma_bus_base + used_offset as u64;
    let buffer_bus = config.dma_bus_base + buffer_offset as u64;
    
    // Calculate CPU pointers
    let buffer_cpu = config.dma_cpu_base.add(buffer_offset);
    let desc_cpu = config.dma_cpu_base.add(desc_offset);
    
    // Set queue size
    transport.set_queue_size(queue_size);
    
    // Set queue addresses
    transport.set_queue_desc(desc_bus);
    transport.set_queue_avail(avail_bus);
    transport.set_queue_used(used_bus);
    
    // Enable queue
    transport.enable_queue();
    
    // Get notify address
    let notify_addr = transport.get_notify_addr(queue_index);
    
    // Create queue state
    Ok(VirtqueueState {
        desc_base: desc_bus,
        avail_base: avail_bus,
        used_base: used_bus,
        queue_size,
        queue_index,
        _pad: 0,
        notify_addr,
        last_used_idx: 0,
        next_avail_idx: 0,
        _pad2: 0,
        desc_cpu_ptr: desc_cpu as u64,
        buffer_cpu_base: buffer_cpu as u64,
        buffer_bus_base: buffer_bus,
        buffer_size: config.buffer_size as u32,
        buffer_count: queue_size as u32,
    })
}

// Stub for non-x86_64 platforms
#[cfg(not(target_arch = "x86_64"))]
pub unsafe fn virtio_net_init_transport(
    _transport: &VirtioTransport,
    _config: &VirtioConfig,
    _tsc_freq: u64,
) -> Result<(u64, VirtqueueState, VirtqueueState, MacAddress), VirtioInitError> {
    Err(VirtioInitError::DeviceError)
}
