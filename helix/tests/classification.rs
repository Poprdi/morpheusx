//! Inline-vs-extent classification must not depend on user payload bytes. A
//! small inline file whose content happens to start with the 0xFF extent marker
//! must replay as inline, not be mistaken for an extent descriptor.

mod common;

use common::MemBio;
use morpheus_helix::HelixFs;

const DISK_SECTORS: usize = 2200;

#[test]
fn inline_data_starting_with_0xff_survives_remount() {
    let mut dev = MemBio::new(DISK_SECTORS);
    let sectors = dev.sectors();
    let mut fs = HelixFs::format_and_mount(&mut dev, 0, sectors, 512, "t", [0u8; 16]).unwrap();

    // 64 bytes (<= 96, inline) whose first byte is the 0xFF extent marker.
    let content = [0xFFu8; 64];
    fs.write(&mut dev, "/a", &content, 100).unwrap();
    assert_eq!(fs.read(&mut dev, "/a").unwrap(), content, "live read wrong");
    fs.sync(&mut dev).unwrap();
    drop(fs);

    let fs2 = HelixFs::mount(&mut dev, 0, 512).unwrap();
    let got = fs2.read(&mut dev, "/a");
    assert_eq!(
        got.as_deref(),
        Ok(&content[..]),
        "inline 0xFF content misclassified as an extent on replay: {got:?}"
    );
}
