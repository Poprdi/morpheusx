//! Serial-output shim. Re-exports `morpheus-hal-x86_64::serial` under the
//! `serial_*` names the e1000e diagnostics use (cycle prevents depending on
//! `morpheus-net-stack::mainloop::serial`).

use morpheus_hal_x86_64::serial as hal;

#[inline]
pub fn serial_print(s: &str) {
    hal::puts(s);
}

#[inline]
pub fn serial_println(s: &str) {
    hal::puts(s);
    hal::puts("\r\n");
}

#[inline]
pub fn serial_print_hex(value: u64) {
    hal::puts("0x");
    hal::puts_hex_u64(value);
}

#[inline]
pub fn serial_print_decimal(value: u32) {
    hal::puts_dec_u32(value);
}
