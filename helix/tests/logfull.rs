//! The log ring must not be terminal. Enough mutations to lap the ring have to
//! trigger a checkpoint that recycles superseded segments; the live namespace
//! must keep accepting writes and survive a remount.

mod common;

use common::MemBio;
use morpheus_helix::HelixFs;

/// 409600 sectors -> 51200 FS blocks -> 2 log segments (2 MiB of log).
const DISK_SECTORS: usize = 409_600;

#[test]
fn log_full_self_heals_via_checkpoint() {
    let mut dev = MemBio::new(DISK_SECTORS);
    let sectors = dev.sectors();
    let mut fs = HelixFs::format_and_mount(&mut dev, 0, sectors, 512, "t", [0u8; 16]).unwrap();

    // ~80 B per record; ~26k records fill both segments. 30k laps the ring.
    const N: u32 = 30_000;
    for i in 0..N {
        let r = fs.write(&mut dev, "/a", &i.to_le_bytes(), 100 + i as u64);
        assert!(r.is_ok(), "write {i} bricked the log (LogFull not self-healed): {r:?}");
    }

    let last = (N - 1).to_le_bytes().to_vec();
    assert_eq!(fs.read(&mut dev, "/a").unwrap(), last, "live content wrong after checkpointing");

    fs.sync(&mut dev).unwrap();
    drop(fs);
    let fs2 = HelixFs::mount(&mut dev, 0, 512).unwrap();
    assert_eq!(
        fs2.read(&mut dev, "/a").unwrap(),
        last,
        "content lost across remount after checkpoint (log/index not recoverable)"
    );
}

#[test]
fn dir_ops_self_heal_on_log_full() {
    let mut dev = MemBio::new(DISK_SECTORS);
    let sectors = dev.sectors();
    let mut fs = HelixFs::format_and_mount(&mut dev, 0, sectors, 512, "t", [0u8; 16]).unwrap();

    // mkdir+unlink cycling laps the ring with a tiny live index — exercises the
    // checkpoint-retry path for directory ops, not just write().
    for i in 0..15_000u32 {
        fs.mkdir(&mut dev, "/d", 1).unwrap_or_else(|e| panic!("mkdir iter {i}: {e:?}"));
        fs.unlink(&mut dev, "/d", 1).unwrap_or_else(|e| panic!("unlink iter {i}: {e:?}"));
    }

    fs.mkdir(&mut dev, "/keep", 2).unwrap();
    fs.sync(&mut dev).unwrap();
    drop(fs);

    let fs2 = HelixFs::mount(&mut dev, 0, 512).unwrap();
    assert!(fs2.stat("/keep").is_ok(), "dir lost across checkpoint + remount");
}
