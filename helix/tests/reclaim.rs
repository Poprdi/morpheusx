//! Space reclamation: overwriting a file must free the prior version's blocks
//! (unless a snapshot pins them), or a long-lived mount drains to NoSpace and a
//! later remount frees-then-reuses still-referenced blocks.

mod common;

use common::MemBio;
use morpheus_helix::HelixFs;

const DISK_SECTORS: usize = 4096;
const BLOCK: usize = 4096;

#[test]
fn overwrite_frees_prior_extent_blocks() {
    let mut dev = MemBio::new(DISK_SECTORS);
    let sectors = dev.sectors();
    let mut fs = HelixFs::format_and_mount(&mut dev, 0, sectors, 512, "t", [0u8; 16]).unwrap();

    fs.write(&mut dev, "/a", &[0u8; 2 * BLOCK], 1).unwrap();
    let used_after_first = fs.bitmap.allocated_count();

    for i in 0..20u8 {
        fs.write(&mut dev, "/a", &[i + 1; 2 * BLOCK], 10 + i as u64).unwrap();
    }

    assert_eq!(
        fs.bitmap.allocated_count(),
        used_after_first,
        "overwrite leaked blocks: the prior version's extents were never freed"
    );
    // And the file still reads back the last write.
    assert_eq!(fs.read(&mut dev, "/a").unwrap(), vec![20u8; 2 * BLOCK]);
}

fn blocks_of(fs: &HelixFs, path: &str, n: u64) -> Vec<u64> {
    let root = fs.index.lookup(path).unwrap().extent_root;
    (0..n).map(|i| root + i).collect()
}

#[test]
fn snapshot_pins_overwritten_blocks() {
    let mut dev = MemBio::new(DISK_SECTORS);
    let sectors = dev.sectors();
    let mut fs = HelixFs::format_and_mount(&mut dev, 0, sectors, 512, "t", [0u8; 16]).unwrap();

    fs.write(&mut dev, "/a", &[1u8; 2 * BLOCK], 1).unwrap();
    let pinned = blocks_of(&fs, "/a", 2);
    fs.snapshot(&mut dev, "s", 5).unwrap();
    fs.write(&mut dev, "/a", &[2u8; 2 * BLOCK], 10).unwrap();

    for b in pinned {
        assert!(
            fs.bitmap.is_allocated(b),
            "block {b} held by a snapshot was freed on overwrite"
        );
    }
}

#[test]
fn snapshot_pin_survives_remount() {
    let mut dev = MemBio::new(DISK_SECTORS);
    let sectors = dev.sectors();
    let mut fs = HelixFs::format_and_mount(&mut dev, 0, sectors, 512, "t", [0u8; 16]).unwrap();

    fs.write(&mut dev, "/a", &[1u8; 2 * BLOCK], 1).unwrap();
    fs.snapshot(&mut dev, "s", 5).unwrap();
    fs.sync(&mut dev).unwrap();
    drop(fs);

    let mut fs2 = HelixFs::mount(&mut dev, 0, 512).unwrap();
    let pinned = blocks_of(&fs2, "/a", 2);
    fs2.write(&mut dev, "/a", &[2u8; 2 * BLOCK], 10).unwrap();

    for b in pinned {
        assert!(
            fs2.bitmap.is_allocated(b),
            "snapshot pin was lost across remount; block {b} freed"
        );
    }
}
