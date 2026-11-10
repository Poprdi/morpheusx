//! Regressions from the 2026-11 correctness audit. See docs/REGRESSIONS.md.

mod common;

use common::{CrashBio, MemBio, MemBioError};
use gpt_disk_io::BlockIo;
use gpt_disk_types::{BlockSize, Lba};
use morpheus_helix::HelixFs;

const B: usize = 4096;
/// 2 MiB: one log segment, ~250 data blocks.
const SMALL: usize = 4096;
/// 200 MiB: two log segments.
const TWO_SEG: usize = 409_600;

fn mk(sectors: usize) -> (MemBio, HelixFs) {
    let mut dev = MemBio::new(sectors);
    let n = dev.sectors();
    let fs = HelixFs::format_and_mount(&mut dev, 0, n, 512, "t", [0u8; 16]).unwrap();
    (dev, fs)
}

/// Fails every flush once `flushes_left` hits zero.
struct FlushLimit {
    inner: CrashBio,
    flushes_left: u64,
}

impl BlockIo for FlushLimit {
    type Error = MemBioError;

    fn block_size(&self) -> BlockSize {
        self.inner.block_size()
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        self.inner.num_blocks()
    }

    fn read_blocks(&mut self, s: Lba, d: &mut [u8]) -> Result<(), Self::Error> {
        self.inner.read_blocks(s, d)
    }

    fn write_blocks(&mut self, s: Lba, d: &[u8]) -> Result<(), Self::Error> {
        self.inner.write_blocks(s, d)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        if self.flushes_left == 0 {
            return Err(MemBioError("power cut"));
        }
        self.flushes_left -= 1;
        self.inner.flush()
    }
}

// BUG: RB-0007 blocks of a committed version reused before the superseding state commits
#[test]
fn overwrite_then_reuse_before_commit() {
    let (mut dev, mut fs) = mk(SMALL);
    fs.write(&mut dev, "/a", &[0xAA; 2 * B], 1).unwrap();
    fs.sync(&mut dev).unwrap();
    fs.write(&mut dev, "/a", &[0xBB; 2 * B], 2).unwrap();
    fs.write(&mut dev, "/b", &[0xCC; 2 * B], 3).unwrap();
    drop(fs); // power cut on a write-through device

    let fs = HelixFs::mount(&mut dev, 0, 512).unwrap();
    assert_eq!(fs.read(&mut dev, "/a").unwrap(), vec![0xAA; 2 * B]);
}

// BUG: RB-0007 same hole through unlink
#[test]
fn unlink_then_reuse_before_commit() {
    let (mut dev, mut fs) = mk(SMALL);
    fs.write(&mut dev, "/a", &[0xAA; 2 * B], 1).unwrap();
    fs.sync(&mut dev).unwrap();
    fs.unlink(&mut dev, "/a", 2).unwrap();
    fs.write(&mut dev, "/b", &[0xCC; 2 * B], 3).unwrap();
    drop(fs);

    let fs = HelixFs::mount(&mut dev, 0, 512).unwrap();
    assert_eq!(fs.read(&mut dev, "/a").unwrap(), vec![0xAA; 2 * B]);
}

// BUG: RB-0007 same hole when power fails between the log flush and the superblock
#[test]
fn power_cut_inside_sync() {
    let mut dev = FlushLimit {
        inner: CrashBio::new(SMALL),
        flushes_left: u64::MAX,
    };
    let n = dev.inner.sectors();
    let mut fs = HelixFs::format_and_mount(&mut dev, 0, n, 512, "t", [0u8; 16]).unwrap();
    fs.write(&mut dev, "/a", &[0xAA; 2 * B], 1).unwrap();
    fs.sync(&mut dev).unwrap();
    fs.write(&mut dev, "/a", &[0xBB; 2 * B], 2).unwrap();
    fs.write(&mut dev, "/b", &[0xCC; 2 * B], 3).unwrap();
    dev.flushes_left = 1;
    assert!(fs.sync(&mut dev).is_err());
    dev.inner.crash();
    dev.flushes_left = u64::MAX;
    drop(fs);

    let fs = HelixFs::mount(&mut dev, 0, 512).unwrap();
    assert_eq!(fs.read(&mut dev, "/a").unwrap(), vec![0xAA; 2 * B]);
}

// BUG: RB-0008 rename(p, p) freed and tombstoned the file
#[test]
fn rename_onto_itself_is_a_noop() {
    let (mut dev, mut fs) = mk(SMALL);
    fs.write(&mut dev, "/a", &[0x55; 2 * B], 1).unwrap();
    fs.rename(&mut dev, "/a", "/a", 2).unwrap();
    assert_eq!(fs.read(&mut dev, "/a").unwrap(), vec![0x55; 2 * B]);
}

// BUG: RB-0009 a damaged committed log record mounted as a silently rolled-back volume
#[test]
fn damaged_committed_log_fails_mount() {
    let (mut dev, mut fs) = mk(SMALL);
    for i in 0..50 {
        fs.write(&mut dev, &format!("/f{i}"), b"x", i).unwrap();
    }
    fs.sync(&mut dev).unwrap();
    let log_start = fs.sb.log_start_block as usize * B;
    drop(fs);

    for off in [64 + 20, 64 + 16, 3000] {
        let mut d = MemBio::new(SMALL);
        d.poke(0, &dev.peek(0, SMALL * 512));
        let byte = d.peek(log_start + off, 1)[0];
        d.poke(log_start + off, &[!byte]);
        assert!(
            HelixFs::mount(&mut d, 0, 512).is_err(),
            "mount accepted a log damaged at +{off}"
        );
    }
}

// BUG: RB-0010 snapshot-pinned blocks were handed out again after a remount
#[test]
fn snapshot_version_survives_remount_and_checkpoint() {
    let (mut dev, mut fs) = mk(SMALL);
    fs.write(&mut dev, "/a", &[0x11; 2 * B], 1).unwrap();
    let snap = fs.snapshot(&mut dev, "s", 2).unwrap();
    fs.write(&mut dev, "/a", &[0x22; 2 * B], 3).unwrap();
    fs.sync(&mut dev).unwrap();
    drop(fs);

    let mut fs = HelixFs::mount(&mut dev, 0, 512).unwrap();
    fs.write(&mut dev, "/b", &[0x33; 2 * B], 4).unwrap();
    assert_eq!(
        fs.read_at_lsn(&mut dev, "/a", snap).unwrap(),
        vec![0x11; 2 * B]
    );

    fs.checkpoint(&mut dev).unwrap();
    drop(fs);
    let mut fs = HelixFs::mount(&mut dev, 0, 512).unwrap();
    assert!(fs.snapshot_lsns.contains(&snap));
    fs.write(&mut dev, "/c", &[0x44; 2 * B], 5).unwrap();
    assert_eq!(
        fs.read_at_lsn(&mut dev, "/a", snap).unwrap(),
        vec![0x11; 2 * B]
    );
    assert_eq!(fs.read(&mut dev, "/a").unwrap(), vec![0x22; 2 * B]);
}

// BUG: RB-0010 unlink and rename-over ignored snapshot pins
#[test]
fn snapshot_pins_unlinked_and_replaced_files() {
    let (mut dev, mut fs) = mk(SMALL);
    fs.write(&mut dev, "/a", &[0x11; 2 * B], 1).unwrap();
    fs.write(&mut dev, "/b", &[0x22; 2 * B], 2).unwrap();
    fs.write(&mut dev, "/c", &[0x33; 2 * B], 3).unwrap();
    let snap = fs.snapshot(&mut dev, "s", 4).unwrap();
    fs.unlink(&mut dev, "/a", 5).unwrap();
    fs.rename(&mut dev, "/c", "/b", 6).unwrap();
    for i in 0..20 {
        fs.write(&mut dev, &format!("/n{i}"), &[0x99; 2 * B], 7)
            .unwrap();
    }
    assert_eq!(
        fs.read_at_lsn(&mut dev, "/a", snap).unwrap(),
        vec![0x11; 2 * B]
    );
    assert_eq!(
        fs.read_at_lsn(&mut dev, "/b", snap).unwrap(),
        vec![0x22; 2 * B]
    );
}

// BUG: RB-0011 extent data was never checksummed on read
#[test]
fn damaged_data_block_is_an_error() {
    let (mut dev, mut fs) = mk(SMALL);
    fs.write(&mut dev, "/a", &[0x77; 2 * B], 1).unwrap();
    fs.sync(&mut dev).unwrap();
    let root = fs.index.lookup("/a").unwrap().extent_root;
    let off = (fs.sb.data_start_block + root) as usize * B;
    dev.poke(off + 10, &[0x00]);
    assert!(fs.read(&mut dev, "/a").is_err());
    drop(fs);

    let fs = HelixFs::mount(&mut dev, 0, 512).unwrap();
    assert!(fs.read(&mut dev, "/a").is_err());
}

// BUG: RB-0012 a one-segment log could never leave LogFull
#[test]
fn single_segment_log_recycles() {
    let (mut dev, mut fs) = mk(SMALL);
    for i in 0..30_000u32 {
        fs.write(&mut dev, "/a", &i.to_le_bytes(), i as u64)
            .unwrap_or_else(|e| panic!("write {i}: {e:?}"));
    }
    fs.sync(&mut dev).unwrap();
    drop(fs);
    let fs = HelixFs::mount(&mut dev, 0, 512).unwrap();
    assert_eq!(fs.read(&mut dev, "/a").unwrap(), 29_999u32.to_le_bytes());
}

// BUG: RB-0013 full disk + full log left no way to delete anything
#[test]
fn full_disk_can_still_unlink() {
    let (mut dev, mut fs) = mk(TWO_SEG);
    let free = fs.bitmap.free_count() as usize;
    fs.write(&mut dev, "/big", &vec![1u8; free * B], 1).unwrap();
    for i in 0..40_000u32 {
        fs.mkdir(&mut dev, "/d", 1)
            .and_then(|_| fs.unlink(&mut dev, "/d", 1))
            .unwrap_or_else(|e| panic!("iter {i}: {e:?}"));
    }
    fs.unlink(&mut dev, "/big", 2).unwrap();
    fs.sync(&mut dev).unwrap();
    assert!(fs.bitmap.free_count() as usize >= free);
}

// BUG: RB-0013 a checkpoint needed one contiguous run of free blocks
#[test]
fn checkpoint_fits_in_fragmented_space() {
    // 2200 sectors -> 16 data blocks
    let (mut dev, mut fs) = mk(2200);
    for i in 0..16u8 {
        fs.write(&mut dev, &format!("/f{i}"), &[i + 1; B], 1)
            .unwrap();
    }
    for i in (1..16u8).step_by(2) {
        fs.unlink(&mut dev, &format!("/f{i}"), 2).unwrap();
    }
    for i in 0..20u8 {
        fs.write(&mut dev, &format!("/s{i}"), &[i], 3).unwrap();
    }
    // 28 entries need 4 blocks; only single-block holes are free
    fs.checkpoint(&mut dev).unwrap();
    assert_ne!(fs.sb.index_flags, 0, "expected a fragmented checkpoint");
    drop(fs);

    let mut fs = HelixFs::mount(&mut dev, 0, 512).unwrap();
    for i in (0..16u8).step_by(2) {
        assert_eq!(
            fs.read(&mut dev, &format!("/f{i}")).unwrap(),
            vec![i + 1; B]
        );
    }
    for i in 0..20u8 {
        assert_eq!(fs.read(&mut dev, &format!("/s{i}")).unwrap(), vec![i]);
    }
    // the checkpoint's own blocks (4 + node) stay reserved after the remount
    assert_eq!(fs.bitmap.free_count(), 8 - 5);
    fs.unlink(&mut dev, "/f0", 4).unwrap();
    fs.unlink(&mut dev, "/f2", 4).unwrap();
    fs.checkpoint(&mut dev).unwrap();
    drop(fs);
    let fs = HelixFs::mount(&mut dev, 0, 512).unwrap();
    assert_eq!(fs.read(&mut dev, "/f4").unwrap(), vec![5; B]);
    assert!(fs.stat("/f0").is_err());
}

// BUG: RB-0014 a failed fragmented allocation leaked every block it had taken
#[test]
fn failed_write_leaks_nothing() {
    let (mut dev, mut fs) = mk(SMALL);
    let free = fs.bitmap.free_count();
    let r = fs.write(&mut dev, "/big", &vec![1u8; (free as usize + 5) * B], 1);
    assert!(r.is_err());
    assert_eq!(fs.bitmap.free_count(), free);
}

// BUG: RB-0015 mount trusted any checksum-valid superblock
#[test]
fn bad_superblock_geometry_fails_mount() {
    let (mut dev, mut fs) = mk(SMALL);
    fs.sync(&mut dev).unwrap();
    let good = fs.sb;
    drop(fs);

    let mut cases = Vec::new();
    let mut sb = good;
    sb.log_segment_count = 0;
    cases.push(sb);
    let mut sb = good;
    sb.log_head_offset = 8;
    cases.push(sb);
    let mut sb = good;
    sb.log_head_segment = 99;
    cases.push(sb);
    let mut sb = good;
    sb.data_block_count = u64::MAX / 2;
    cases.push(sb);
    let mut sb = good;
    sb.index_root_block = sb.data_block_count;
    sb.index_depth = 1;
    cases.push(sb);

    for (i, mut sb) in cases.into_iter().enumerate() {
        sb.update_crc();
        // SAFETY: the superblock is a 4096-byte repr(C) POD.
        let bytes = unsafe { core::slice::from_raw_parts(&sb as *const _ as *const u8, B) };
        dev.poke(0, bytes);
        dev.poke(B, bytes);
        assert!(
            HelixFs::mount(&mut dev, 0, 512).is_err(),
            "case {i} mounted"
        );
    }
}

// BUG: RB-0016 a directory rename could be split by a checkpoint
#[test]
fn directory_rename_is_all_or_nothing_across_log_full() {
    let (mut dev, mut fs) = mk(SMALL);
    for i in 0..100 {
        fs.write(&mut dev, &format!("/d/f{i:03}"), b"x", 1).unwrap();
    }
    // leave less log than the rename needs
    let mut n = 0u32;
    while fs.log.head_offset() < 1024 * 1024 - 16 * 1024 {
        fs.write(&mut dev, "/x", &n.to_le_bytes(), 1).unwrap();
        n += 1;
    }
    fs.rename(&mut dev, "/d", "/e", 5).unwrap();
    fs.sync(&mut dev).unwrap();
    drop(fs);

    let fs = HelixFs::mount(&mut dev, 0, 512).unwrap();
    for i in 0..100 {
        assert!(fs.stat(&format!("/e/f{i:03}")).is_ok());
        assert!(fs.stat(&format!("/d/f{i:03}")).is_err());
    }
}

/// The sliced CRCs must match a bit-at-a-time reference at every length.
#[test]
fn sliced_crcs_match_bitwise_reference() {
    fn ref32(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0x82F6_3B78
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    fn ref64(data: &[u8]) -> u64 {
        let mut crc = !0u64;
        for &b in data {
            crc ^= b as u64;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xC96C_5795_D787_0F42
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    let data: Vec<u8> = (0..300u32).map(|i| (i * 131 + 7) as u8).collect();
    for len in 0..data.len() {
        let d = &data[..len];
        assert_eq!(morpheus_helix::crc::crc32c(d), ref32(d), "crc32c len {len}");
        assert_eq!(morpheus_helix::crc::crc64(d), ref64(d), "crc64 len {len}");
        let (a, b) = d.split_at(len / 3);
        assert_eq!(morpheus_helix::crc::crc32c_two(a, b), ref32(d));
    }
}
