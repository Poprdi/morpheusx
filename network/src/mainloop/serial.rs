//! Serial output primitives for post-EBS bare-metal execution.
//!
//! Minimal, no-allocation serial output to COM1 (0x3F8).
//! Also mirrors to framebuffer when display feature is enabled.

/// Serial port base address (COM1).
const SERIAL_PORT: u16 = 0x3F8;

/// Write a single byte to COM1 serial port.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn write_byte(byte: u8) {
    unsafe {
        // Wait for transmit buffer empty (bounded)
        let mut retries = 0u32;
        loop {
            let status: u8;
            core::arch::asm!(
                "in al, dx",
                in("dx") SERIAL_PORT + 5,
                out("al") status,
                options(nomem, nostack, preserves_flags)
            );
            if status & 0x20 != 0 {
                break;
            }
            retries += 1;
            if retries > 100 {
                return; // Port not responding
            }
            core::hint::spin_loop();
        }
        core::arch::asm!(
            "out dx, al",
            in("dx") SERIAL_PORT,
            in("al") byte,
            options(nomem, nostack, preserves_flags)
        );
    }
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn write_byte(_byte: u8) {}

/// Write a string to serial port.
#[inline]
pub fn print(s: &str) {
    for byte in s.bytes() {
        write_byte(byte);
    }
    crate::display::display_write(s);
}

/// Write a string with newline.
#[inline]
pub fn println(s: &str) {
    print(s);
    print("\r\n");
}

/// Print a byte as two hex digits.
pub fn print_hex_byte(value: u8) {
    let hi = value >> 4;
    let lo = value & 0xF;
    let hi_char = if hi < 10 { b'0' + hi } else { b'a' + hi - 10 };
    let lo_char = if lo < 10 { b'0' + lo } else { b'a' + lo - 10 };
    write_byte(hi_char);
    write_byte(lo_char);
    
    let buf = [hi_char, lo_char];
    if let Ok(s) = core::str::from_utf8(&buf) {
        crate::display::display_write(s);
    }
}

/// Print a u64 as hex with 0x prefix.
pub fn print_hex(value: u64) {
    print("0x");
    let mut buf = [0u8; 16];
    for i in 0..16 {
        let nibble = ((value >> ((15 - i) * 4)) & 0xF) as u8;
        let c = if nibble < 10 { b'0' + nibble } else { b'a' + nibble - 10 };
        buf[i] = c;
        write_byte(c);
    }
    if let Ok(s) = core::str::from_utf8(&buf) {
        crate::display::display_write(s);
    }
}

/// Print a u32 as decimal.
pub fn print_u32(value: u32) {
    if value == 0 {
        write_byte(b'0');
        crate::display::display_write("0");
        return;
    }
    
    let mut buf = [0u8; 10];
    let mut i = 0;
    let mut val = value;
    while val > 0 {
        buf[i] = b'0' + (val % 10) as u8;
        val /= 10;
        i += 1;
    }
    
    let mut display_buf = [0u8; 10];
    let num_digits = i;
    for j in 0..num_digits {
        display_buf[j] = buf[num_digits - 1 - j];
    }
    
    while i > 0 {
        i -= 1;
        write_byte(buf[i]);
    }
    
    if let Ok(s) = core::str::from_utf8(&display_buf[..num_digits]) {
        crate::display::display_write(s);
    }
}

/// Print MAC address in XX:XX:XX:XX:XX:XX format.
pub fn print_mac(mac: &[u8; 6]) {
    for (i, byte) in mac.iter().enumerate() {
        if i > 0 {
            print(":");
        }
        print_hex_byte(*byte);
    }
}

/// Print IPv4 address in dotted decimal.
pub fn print_ipv4(octets: &[u8; 4]) {
    for (i, octet) in octets.iter().enumerate() {
        if i > 0 {
            print(".");
        }
        print_u32(*octet as u32);
    }
}

// Legacy aliases for compatibility during transition
pub use print as serial_print;
pub use println as serial_println;
pub use print_hex as serial_print_hex;
pub use print_hex_byte as serial_print_hex_byte;
pub use print_u32 as serial_print_decimal;
