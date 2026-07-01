//! Path hygiene: malformed paths must be rejected (not stored as phantom,
//! invisible entries), and a file write must never shadow/clobber a directory.

mod common;

use common::MemBio;
use morpheus_helix::HelixFs;

const DISK_SECTORS: usize = 4096;

fn mount() -> (MemBio, HelixFs) {
    let mut dev = MemBio::new(DISK_SECTORS);
    let sectors = dev.sectors();
    let fs = HelixFs::format_and_mount(&mut dev, 0, sectors, 512, "t", [0u8; 16]).unwrap();
    (dev, fs)
}

#[test]
fn malformed_paths_rejected() {
    let (mut dev, mut fs) = mount();
    for p in ["//a", "/a//b", "/a/../b", "/a/./b", "/a/"] {
        assert!(
            fs.write(&mut dev, p, b"x", 1).is_err(),
            "{p} must be rejected, not stored"
        );
    }
    fs.write(&mut dev, "/ok", b"y", 1).unwrap();
    assert_eq!(fs.read(&mut dev, "/ok").unwrap(), b"y");
}

#[test]
fn write_over_directory_rejected() {
    let (mut dev, mut fs) = mount();
    fs.mkdir(&mut dev, "/d", 1).unwrap();
    fs.write(&mut dev, "/d/child", b"c", 2).unwrap();

    assert!(
        fs.write(&mut dev, "/d", b"x", 3).is_err(),
        "a write over a directory must fail"
    );
    assert!(
        fs.stat("/d").unwrap().is_dir(),
        "directory was clobbered/shadowed by a file write"
    );
    assert_eq!(
        fs.read(&mut dev, "/d/child").unwrap(),
        b"c",
        "child orphaned by a shadowing write"
    );
}
