//! A snapshot must be durable on its own: snapshot() has to persist the
//! superblock, not just flush the log, or a crash before the next sync loses the
//! marker (the replay boundary never advanced past it).

mod common;

use common::CrashBio;
use morpheus_helix::HelixFs;

const DISK_SECTORS: usize = 4096;

#[test]
fn snapshot_survives_crash() {
    let mut dev = CrashBio::new(DISK_SECTORS);
    let sectors = dev.sectors();
    let mut fs = HelixFs::format_and_mount(&mut dev, 0, sectors, 512, "t", [0u8; 16]).unwrap();

    fs.write(&mut dev, "/a", b"hi", 1).unwrap();
    fs.sync(&mut dev).unwrap();

    let snap = fs.snapshot(&mut dev, "s", 5).unwrap();
    // Power cut with no further sync.
    dev.crash();
    drop(fs);

    let fs2 = HelixFs::mount(&mut dev, 0, 512).unwrap();
    assert!(
        fs2.snapshot_lsns.contains(&snap),
        "snapshot {snap} lost across crash: superblock was not persisted with the marker"
    );
}
