//! Stub. Bootloader desktop owns all PS/2 mouse I/O; duplicate polling here
//! used to steal bytes off port 0x60 and double-flip the Y axis.

pub fn init() {}
pub fn poll() {}
