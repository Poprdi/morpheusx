//! Main loop runner.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §6.2

// TODO: Implement main_loop
//
// pub fn main_loop(
//     device: &mut impl NetworkDriver,
//     iface: &mut Interface,
//     sockets: &mut SocketSet,
//     app: &mut impl StateMachine,
//     handoff: &BootHandoff,
// ) -> ! {
//     let timeouts = TimeoutConfig::new(handoff.tsc_freq);
//     
//     loop {
//         let iteration_start = read_tsc();
//         let timestamp = tsc_to_smoltcp_instant(iteration_start, handoff.tsc_freq);
//         
//         // Phase 1: Refill RX
//         phase1_rx_refill(device);
//         
//         // Phase 2: smoltcp poll (EXACTLY ONCE)
//         let mut adapter = DeviceAdapter::new(device);
//         iface.poll(timestamp, &mut adapter, sockets);
//         
//         // Phase 3: TX drain
//         phase3_tx_drain(&mut adapter, TX_BUDGET);
//         
//         // Phase 4: App step
//         let result = phase4_app_step(app, iface, sockets, iteration_start, &timeouts);
//         
//         // Phase 5: TX completions
//         phase5_tx_completions(device);
//         
//         // Check iteration timing (debug)
//         #[cfg(debug_assertions)]
//         check_iteration_timing(iteration_start, &timeouts);
//     }
// }
