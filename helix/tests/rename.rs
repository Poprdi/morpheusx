//! rename must move a directory's whole subtree, free a clobbered destination's
//! blocks, and reject an overlong target instead of silently corrupting the path.

mod common;

use common::MemBio;
use morpheus_helix::HelixFs;

const DISK_SECTORS: usize = 4096;
const BLOCK: usize = 4096;

fn mount() -> (MemBio, HelixFs) {
    let mut dev = MemBio::new(DISK_SECTORS);
    let sectors = dev.sectors();
    let fs = HelixFs::format_and_mount(&mut dev, 0, sectors, 512, "t", [0u8; 16]).unwrap();
    (dev, fs)
}

#[test]
fn rename_to_overlong_path_is_rejected() {
    let (mut dev, mut fs) = mount();
    fs.write(&mut dev, "/a", b"hi", 1).unwrap();

    let long = format!("/{}", "x".repeat(300));
    assert!(
        fs.rename(&mut dev, "/a", &long, 2).is_err(),
        "overlong rename must fail"
    );
    assert!(
        fs.stat("/a").is_ok(),
        "source must survive a rejected rename"
    );
}

#[test]
fn rename_onto_existing_file_frees_destination() {
    let (mut dev, mut fs) = mount();
    fs.write(&mut dev, "/a", &[0xAAu8; 2 * BLOCK], 1).unwrap();
    fs.write(&mut dev, "/b", &[0xBBu8; 2 * BLOCK], 2).unwrap();
    let used_before = fs.bitmap.allocated_count();

    fs.rename(&mut dev, "/a", "/b", 3).unwrap();

    assert_eq!(
        fs.read(&mut dev, "/b").unwrap(),
        vec![0xAAu8; 2 * BLOCK],
        "rename target wrong"
    );
    assert!(
        fs.read(&mut dev, "/a").is_err(),
        "source must be gone after rename"
    );
    assert_eq!(
        fs.bitmap.allocated_count(),
        used_before - 2,
        "clobbered destination's blocks leaked"
    );
}

#[test]
fn rename_directory_moves_children() {
    let (mut dev, mut fs) = mount();
    fs.mkdir(&mut dev, "/d", 1).unwrap();
    fs.write(&mut dev, "/d/f", b"hello", 2).unwrap();
    fs.write(&mut dev, "/d/sub/g", b"world", 3).unwrap();

    fs.rename(&mut dev, "/d", "/e", 4).unwrap();

    assert_eq!(
        fs.read(&mut dev, "/e/f").unwrap(),
        b"hello",
        "child not moved"
    );
    assert_eq!(
        fs.read(&mut dev, "/e/sub/g").unwrap(),
        b"world",
        "nested child not moved"
    );
    assert!(fs.stat("/d").is_err(), "old directory must be gone");
    assert!(
        fs.read(&mut dev, "/d/f").is_err(),
        "old child path must be gone"
    );

    fs.sync(&mut dev).unwrap();
    drop(fs);
    let fs2 = HelixFs::mount(&mut dev, 0, 512).unwrap();
    assert_eq!(
        fs2.read(&mut dev, "/e/sub/g").unwrap(),
        b"world",
        "subtree rename lost on remount"
    );
}
