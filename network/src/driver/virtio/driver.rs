//! VirtIO driver implementation.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §4, §8.4

use crate::dma::{DmaRegion, BufferPool};
use crate::types::{VirtqueueState, MacAddress};
use crate::driver::traits::{NetworkDriver, DriverInit, TxError, RxError};
use super::config::{VirtioConfig, VIRTIO_VENDOR_ID, VIRTIO_NET_DEVICE_IDS};
use super::init::{virtio_net_init, VirtioInitError};
use super::{tx, rx};

/// VirtIO network driver.
pub struct VirtioNetDriver {
    /// MMIO base address.
    mmio_base: u64,
    /// MAC address.
    mac: MacAddress,
    /// Negotiated features.
    features: u64,
    /// RX virtqueue state.
    rx_state: VirtqueueState,
    /// TX virtqueue state.
    tx_state: VirtqueueState,
    /// RX buffer pool.
    rx_pool: BufferPool,
    /// TX buffer pool.
    tx_pool: BufferPool,
}

impl VirtioNetDriver {
    /// Create a new VirtIO driver.
    ///
    /// # Arguments
    /// - `mmio_base`: Device MMIO base address
    /// - `config`: DMA configuration
    ///
    /// # Safety
    /// - `mmio_base` must be valid VirtIO MMIO address
    /// - DMA region must be properly allocated
    pub unsafe fn new(mmio_base: u64, config: VirtioConfig) -> Result<Self, VirtioInitError> {
        // Initialize device
        let (features, rx_state, tx_state, mac) = virtio_net_init(mmio_base, &config)?;
        
        // Create buffer pools
        let rx_pool = BufferPool::new(
            config.dma_cpu_base.add(DmaRegion::RX_BUFFERS_OFFSET),
            config.dma_bus_base + DmaRegion::RX_BUFFERS_OFFSET as u64,
            config.buffer_size,
            config.queue_size as usize,
        );
        
        let tx_pool = BufferPool::new(
            config.dma_cpu_base.add(DmaRegion::TX_BUFFERS_OFFSET),
            config.dma_bus_base + DmaRegion::TX_BUFFERS_OFFSET as u64,
            config.buffer_size,
            config.queue_size as usize,
        );
        
        let mut driver = Self {
            mmio_base,
            mac,
            features,
            rx_state,
            tx_state,
            rx_pool,
            tx_pool,
        };
        
        // Pre-fill RX queue
        rx::prefill_queue(&mut driver.rx_state, &mut driver.rx_pool)?;
        
        Ok(driver)
    }
    
    /// Get negotiated features.
    pub fn features(&self) -> u64 {
        self.features
    }
    
    /// Get MMIO base address.
    pub fn mmio_base(&self) -> u64 {
        self.mmio_base
    }
    
    /// Get RX queue state (for debugging).
    pub fn rx_state(&self) -> &VirtqueueState {
        &self.rx_state
    }
    
    /// Get TX queue state (for debugging).
    pub fn tx_state(&self) -> &VirtqueueState {
        &self.tx_state
    }
    
    /// Get number of available RX buffers.
    pub fn rx_buffers_available(&self) -> usize {
        self.rx_pool.available()
    }
    
    /// Get number of available TX buffers.
    pub fn tx_buffers_available(&self) -> usize {
        self.tx_pool.available()
    }
}

impl NetworkDriver for VirtioNetDriver {
    fn mac_address(&self) -> MacAddress {
        self.mac
    }
    
    fn can_transmit(&self) -> bool {
        self.tx_pool.available() > 0
    }
    
    fn can_receive(&self) -> bool {
        // We can always try to receive - ASM will return quickly if nothing
        true
    }
    
    fn transmit(&mut self, frame: &[u8]) -> Result<(), TxError> {
        tx::transmit(&mut self.tx_state, &mut self.tx_pool, frame)
    }
    
    fn receive(&mut self, buffer: &mut [u8]) -> Result<Option<usize>, RxError> {
        rx::receive(&mut self.rx_state, &mut self.rx_pool, buffer)
    }
    
    fn refill_rx_queue(&mut self) {
        rx::refill_queue(&mut self.rx_state, &mut self.rx_pool)
    }
    
    fn collect_tx_completions(&mut self) {
        tx::collect_completions(&mut self.tx_state, &mut self.tx_pool)
    }
    
    fn link_up(&self) -> bool {
        // TODO: Check link status register if VIRTIO_NET_F_STATUS negotiated
        true
    }
}

impl DriverInit for VirtioNetDriver {
    type Error = VirtioInitError;
    type Config = VirtioConfig;
    
    fn supported_vendors() -> &'static [u16] {
        &[VIRTIO_VENDOR_ID]
    }
    
    fn supported_devices() -> &'static [u16] {
        VIRTIO_NET_DEVICE_IDS
    }
    
    unsafe fn create(mmio_base: u64, config: Self::Config) -> Result<Self, Self::Error> {
        Self::new(mmio_base, config)
    }
}
