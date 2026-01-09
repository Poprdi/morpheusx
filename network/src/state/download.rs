//! ISO download orchestration state machine.
//!
//! Composes DHCP → HTTP → Verify state machines.
//!
//! # Reference
//! NETWORK_IMPL_GUIDE.md §5.6

// TODO: Implement IsoDownloadState
//
// pub enum IsoDownloadState {
//     Init,
//     WaitingForNetwork { dhcp: DhcpState },
//     Downloading { http: HttpDownloadState, progress: DownloadProgress },
//     Verifying { data_ptr: *const u8, data_len: usize, expected_hash: [u8; 32] },
//     Done { iso_ptr: *const u8, iso_len: usize },
//     Failed { error: DownloadError },
// }
