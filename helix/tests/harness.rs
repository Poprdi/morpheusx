//! Guards the crash-injection harness contract: flush() is the only barrier.

mod common;

use common::CrashBio;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

#[test]
fn crash_drops_only_unflushed_writes() {
    let mut dev = CrashBio::new(8);
    let block = [0xABu8; 512];
    let mut buf = [0u8; 512];

    dev.write_blocks(Lba(0), &block).unwrap();
    dev.crash();
    dev.read_blocks(Lba(0), &mut buf).unwrap();
    assert_eq!(buf, [0u8; 512], "an unflushed write must vanish on crash");

    dev.write_blocks(Lba(1), &block).unwrap();
    dev.flush().unwrap();
    dev.crash();
    dev.read_blocks(Lba(1), &mut buf).unwrap();
    assert_eq!(buf, [0xABu8; 512], "a flushed write must survive a crash");
}
