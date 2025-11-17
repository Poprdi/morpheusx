// Storage manager utility functions

pub fn format_number(num: u64, buf: &mut [u8]) -> usize {
    if num == 0 {
        buf[0] = b'0';
        return 1;
    }

    let mut n = num;
    let mut len = 0;
    while n > 0 {
        n /= 10;
        len += 1;
    }

    n = num;
    for i in (0..len).rev() {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }

    len
}
