//! Main loop phase implementations.
//!
//! # 5-Phase Structure
//! 1. RX Refill (~20µs)
//! 2. smoltcp Poll - EXACTLY ONCE (~200µs)
//! 3. TX Drain (~40µs)
//! 4. App State Step (~400µs)
//! 5. TX Completions (~20µs)
//!
//! Target: <1ms per iteration
//! Maximum: 5ms per iteration
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §6.2, §6.3

// TODO: Implement phase functions
//
// pub fn phase1_rx_refill(device: &mut impl NetworkDriver) { ... }
// pub fn phase2_smoltcp_poll(iface: &mut Interface, device: &mut DeviceAdapter, sockets: &mut SocketSet, timestamp: Instant) { ... }
// pub fn phase3_tx_drain(adapter: &mut DeviceAdapter, budget: usize) { ... }
// pub fn phase4_app_step(app: &mut impl StateMachine, ...) -> StepResult { ... }
// pub fn phase5_tx_completions(device: &mut impl NetworkDriver) { ... }
