//! Kernel I/O substrate shared by the fs/socket/pipe/epoll domains: the readiness
//! model + the true-blocking primitive they park on. Fd table / OFD lives in
//! `storage::fs_api`.

pub mod readiness;
