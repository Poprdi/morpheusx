//! Mount/replay recovery invariants.

mod common;

use common::MemBio;
use morpheus_helix::HelixFs;

/// 4096 × 512 B = 2 MiB: 512 FS blocks → 1 log segment, ~250 data blocks.
const DISK_SECTORS: usize = 4096;

/// Format, write `/f` twice (so version_count > 1), sync, then drop the engine.
/// Returns the device so the caller can remount and inspect persisted state.
fn disk_with_twice_written_file() -> MemBio {
    let mut dev = MemBio::new(DISK_SECTORS);
    let sectors = dev.sectors();
    let mut fs = HelixFs::format_and_mount(&mut dev, 0, sectors, 512, "test", [0u8; 16]).unwrap();
    fs.write(&mut dev, "/f", b"v1", 100).unwrap();
    fs.write(&mut dev, "/f", b"v2-larger", 200).unwrap();
    fs.sync(&mut dev).unwrap();

    // Sanity: the live engine tracks the history correctly before any remount.
    let live = fs.stat("/f").unwrap();
    assert_eq!(live.version_count, 2, "live version_count");
    assert_eq!(live.created_ns, 100, "live created_ns");

    dev
}

#[test]
fn version_count_survives_remount() {
    let mut dev = disk_with_twice_written_file();

    let fs = HelixFs::mount(&mut dev, 0, 512).unwrap();
    let st = fs.stat("/f").unwrap();

    assert_eq!(
        st.version_count, 2,
        "version_count must survive remount (replay lost the history)"
    );
}

#[test]
fn created_ns_survives_remount() {
    let mut dev = disk_with_twice_written_file();

    let fs = HelixFs::mount(&mut dev, 0, 512).unwrap();
    let st = fs.stat("/f").unwrap();

    assert_eq!(
        st.created_ns, 100,
        "created_ns must reflect the first write, not the latest, after remount"
    );
}
