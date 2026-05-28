//! VirtIO-net `NetworkDriver` impl.

use super::config::{VirtioConfig, VIRTIO_NET_DEVICE_IDS, VIRTIO_VENDOR_ID};
use super::init::{virtio_net_init, virtio_net_init_transport, VirtioInitError};
use super::{rx, tx};
use crate::traits::{DriverInit, MacAddress, NetworkDriver, RxError, TxError};
use morpheus_virtio::dma::{BufferPool, DmaRegion};
use morpheus_virtio::transport::VirtioTransport;
use morpheus_virtio::types::VirtqueueState;

pub struct VirtioNetDriver {
    /// MMIO base, or common_cfg for PCI Modern.
    base_addr: u64,
    transport: VirtioTransport,
    mac: MacAddress,
    features: u64,
    rx_state: VirtqueueState,
    tx_state: VirtqueueState,
    rx_pool: BufferPool,
    tx_pool: BufferPool,
}

impl VirtioNetDriver {
    /// Legacy MMIO path.
    ///
    /// # Safety
    /// `mmio_base` must be a valid VirtIO MMIO address; DMA region allocated.
    pub unsafe fn new(mmio_base: u64, config: VirtioConfig) -> Result<Self, VirtioInitError> {
        let (features, rx_state, tx_state, mac) = virtio_net_init(mmio_base, &config)?;

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
            base_addr: mmio_base,
            transport: VirtioTransport::mmio(mmio_base),
            mac,
            features,
            rx_state,
            tx_state,
            rx_pool,
            tx_pool,
        };

        rx::prefill_queue(&mut driver.rx_state, &mut driver.rx_pool)?;

        Ok(driver)
    }

    /// Auto-selects MMIO or PCI Modern based on the transport.
    ///
    /// # Safety
    /// Transport addresses must be valid; DMA region allocated.
    pub unsafe fn new_with_transport(
        transport: VirtioTransport,
        config: VirtioConfig,
        tsc_freq: u64,
    ) -> Result<Self, VirtioInitError> {
        let (features, rx_state, tx_state, mac) =
            virtio_net_init_transport(&transport, &config, tsc_freq)?;

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
            base_addr: transport.base,
            transport,
            mac,
            features,
            rx_state,
            tx_state,
            rx_pool,
            tx_pool,
        };

        rx::prefill_queue(&mut driver.rx_state, &mut driver.rx_pool)?;

        Ok(driver)
    }

    pub fn features(&self) -> u64 {
        self.features
    }

    pub fn mmio_base(&self) -> u64 {
        self.base_addr
    }

    pub fn transport(&self) -> &VirtioTransport {
        &self.transport
    }

    pub fn rx_state(&self) -> &VirtqueueState {
        &self.rx_state
    }

    pub fn tx_state(&self) -> &VirtqueueState {
        &self.tx_state
    }

    pub fn rx_buffers_available(&self) -> usize {
        self.rx_pool.available()
    }

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
        true // always try; poll returns fast when empty
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
