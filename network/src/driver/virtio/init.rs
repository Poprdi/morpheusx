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

// TODO: Implement virtio_net_init()
