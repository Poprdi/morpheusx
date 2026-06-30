//! Records buffered across a log-segment boundary must survive. The append path
//! reuses a one-segment write buffer; if it advances to the next segment without
//! first persisting the filled one, every record in it is silently lost.

mod common;

use common::MemBio;
use morpheus_helix::HelixFs;

/// 409600 sectors -> 51200 FS blocks -> exactly 2 log segments (the format
/// heuristic gives 1 segment per ~100*256 blocks).
const DISK_SECTORS: usize = 409_600;

#[test]
fn records_survive_log_segment_boundary() {
    let mut dev = MemBio::new(DISK_SECTORS);
    let sectors = dev.sectors();
    let mut fs = HelixFs::format_and_mount(&mut dev, 0, sectors, 512, "t", [0u8; 16]).unwrap();

    // Each inline record is ~176 B; > 5957 of them fill segment 0 (1 MiB) and
    // spill into segment 1. 6500 crosses the boundary exactly once, with NO sync
    // in between, so the boundary crossing happens purely in the write buffer.
    const N: usize = 6500;
    for i in 0..N {
        // % 251 keeps the first byte clear of 0xFF (the extent-record marker),
        // isolating this test from the inline/extent classification bug.
        let data = [(i % 251) as u8; 96];
        fs.write(&mut dev, &format!("/f{i:05}"), &data, 1000 + i as u64).unwrap();
    }
    fs.sync(&mut dev).unwrap();
    drop(fs);

    let fs2 = HelixFs::mount(&mut dev, 0, 512).unwrap();

    // The earliest files lived in segment 0; they must still be there.
    for i in 0..256 {
        let got = fs2.read(&mut dev, &format!("/f{i:05}"));
        assert!(got.is_ok(), "/f{i:05} lost across the segment boundary: {got:?}");
        assert_eq!(got.unwrap(), [(i % 251) as u8; 96], "/f{i:05} content corrupted");
    }
}
