//! Log-record decode must not trust an on-disk length field. A record whose
//! payload_len cannot physically fit in its segment is corruption and must be
//! reported as such — never honoured by allocating/reading that many bytes.

mod common;

use common::MemBio;
use morpheus_helix::{HelixError, HelixFs};

const DISK_SECTORS: usize = 2200;

#[test]
fn read_record_rejects_impossible_payload_len() {
    let mut dev = MemBio::new(DISK_SECTORS);
    let sectors = dev.sectors();
    let mut fs = HelixFs::format_and_mount(&mut dev, 0, sectors, 512, "t", [0u8; 16]).unwrap();
    fs.write(&mut dev, "/a", b"hello", 100).unwrap();
    fs.sync(&mut dev).unwrap();

    // First record sits at segment 0, byte 64 (after the segment header);
    // payload_len is at +20. 2 MiB cannot fit a 1 MiB segment -> corrupt.
    let payload_len_off = fs.sb.log_start_block as usize * 4096 + 64 + 20;
    dev.poke(payload_len_off, &(2u32 * 1024 * 1024).to_le_bytes());

    let res = fs.log.read_record(&mut dev, 0, 64);
    assert!(
        matches!(res, Err(HelixError::LogCrcMismatch)),
        "impossible payload_len must be reported as corruption, got {res:?}"
    );
}
