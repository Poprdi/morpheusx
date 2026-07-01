//! Isolated hardening: on-disk log-segment headers must be self-consistent, and
//! temporal reads of an (extent-backed) older version must return that version.

mod common;

use common::MemBio;
use morpheus_helix::types::LogSegmentHeader;
use morpheus_helix::HelixFs;

const DISK_SECTORS: usize = 4096;

#[test]
fn segment_header_valid_after_flush() {
    let mut dev = MemBio::new(DISK_SECTORS);
    let sectors = dev.sectors();
    let mut fs = HelixFs::format_and_mount(&mut dev, 0, sectors, 512, "t", [0u8; 16]).unwrap();
    fs.write(&mut dev, "/a", b"hi", 1).unwrap();
    fs.sync(&mut dev).unwrap();

    let seg0 = fs.sb.log_start_block as usize * 4096;
    let bytes = dev.peek(seg0, 64);
    let hdr: LogSegmentHeader =
        unsafe { core::ptr::read_unaligned(bytes.as_ptr() as *const LogSegmentHeader) };

    assert!(
        hdr.is_valid(),
        "on-disk segment header fails its own CRC after flush"
    );
    assert_eq!(
        hdr.record_count, 1,
        "record_count written at the wrong offset"
    );
}

#[test]
fn read_file_at_lsn_returns_old_extent_version() {
    let mut dev = MemBio::new(DISK_SECTORS);
    let sectors = dev.sectors();
    let mut fs = HelixFs::format_and_mount(&mut dev, 0, sectors, 512, "t", [0u8; 16]).unwrap();

    // v1 is an extent file (>96 B); a snapshot pins it so its block survives the
    // overwrite, then v2 supersedes it.
    fs.write(&mut dev, "/f", &[0xABu8; 200], 1).unwrap();
    fs.snapshot(&mut dev, "s", 2).unwrap();
    fs.write(&mut dev, "/f", &[0xCDu8; 300], 3).unwrap();
    fs.sync(&mut dev).unwrap();

    let versions = fs.versions(&mut dev, "/f").unwrap();
    let v1_lsn = versions[0].0;

    let out = morpheus_helix::ops::read::read_file_at_lsn(
        &mut dev,
        &fs.log,
        fs.partition_lba_start,
        fs.sb.data_start_block,
        fs.device_block_size,
        "/f",
        v1_lsn,
    )
    .unwrap();

    assert_eq!(
        out,
        vec![0xABu8; 200],
        "temporal read returned wrong bytes for the old extent version"
    );
}
