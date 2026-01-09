pub mod block_io;
pub mod block_io_adapter;
pub mod disk;
pub mod file_system;
pub mod gpt_adapter;

// Note: HTTP is handled by post-EBS network stack (morpheus_network crate)
// UEFI HTTP Protocol bindings removed - we use our own bare-metal TCP/IP stack
