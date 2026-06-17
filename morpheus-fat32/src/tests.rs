//! Self-contained host tests against an in-memory FAT32 image built byte-for-byte.

use crate::types::*;
use crate::*;
use gpt_disk_io::BlockIo;
use gpt_disk_types::{BlockSize, Lba};

const SECTOR: usize = 512;
const SPC: usize = 1; // sectors per cluster
const RESERVED: usize = 32;
const NUM_FATS: usize = 1;
const TOTAL_CLUSTERS: usize = 64; // tiny heap; parser does not gate on the 65525 cutoff

/// RAM-backed `BlockIo` for tests only.
struct MemBio {
    data: alloc::vec::Vec<u8>,
}

impl BlockIo for MemBio {
    type Error = crate::error::Fat32Error;
    fn block_size(&self) -> BlockSize {
        BlockSize::BS_512
    }
    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        Ok((self.data.len() / SECTOR) as u64)
    }
    fn read_blocks(&mut self, start: Lba, dst: &mut [u8]) -> Result<(), Self::Error> {
        let off = start.0 as usize * SECTOR;
        dst.copy_from_slice(&self.data[off..off + dst.len()]);
        Ok(())
    }
    fn write_blocks(&mut self, _start: Lba, _src: &[u8]) -> Result<(), Self::Error> {
        Err(crate::error::Fat32Error::ReadOnly)
    }
    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

struct Builder {
    sectors_per_fat: usize,
    img: alloc::vec::Vec<u8>,
    next_free_cluster: u32,
}

impl Builder {
    fn new() -> Self {
        // One FAT entry per cluster (4 bytes); +2 reserved entries.
        let fat_bytes = (TOTAL_CLUSTERS + 2) * 4;
        let sectors_per_fat = fat_bytes.div_ceil(SECTOR);
        let data_start = RESERVED + NUM_FATS * sectors_per_fat;
        let total_sectors = data_start + TOTAL_CLUSTERS * SPC;

        let mut img = alloc::vec![0u8; total_sectors * SECTOR];
        Self::write_boot(&mut img, sectors_per_fat, total_sectors);
        // Reserve clusters 0 and 1, mark cluster 2 (root) as EOC initially.
        Self::set_fat(&mut img, sectors_per_fat, 0, 0x0FFF_FFF8);
        Self::set_fat(&mut img, sectors_per_fat, 1, 0x0FFF_FFFF);
        Self::set_fat(&mut img, sectors_per_fat, 2, 0x0FFF_FFFF);

        Self {
            sectors_per_fat,
            img,
            next_free_cluster: 3,
        }
    }

    fn write_boot(img: &mut [u8], spf: usize, total: usize) {
        let b = &mut img[..SECTOR];
        b[0..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
        b[11..13].copy_from_slice(&(SECTOR as u16).to_le_bytes()); // bytes/sector
        b[13] = SPC as u8;
        b[14..16].copy_from_slice(&(RESERVED as u16).to_le_bytes());
        b[16] = NUM_FATS as u8;
        b[17..19].copy_from_slice(&0u16.to_le_bytes()); // root entry count (FAT32 = 0)
        b[19..21].copy_from_slice(&0u16.to_le_bytes()); // total_16 (use 32)
        b[22..24].copy_from_slice(&0u16.to_le_bytes()); // sectors/fat 16 (FAT32 = 0)
        b[32..36].copy_from_slice(&(total as u32).to_le_bytes()); // total_32
        b[36..40].copy_from_slice(&(spf as u32).to_le_bytes()); // sectors/fat 32
        b[44..48].copy_from_slice(&2u32.to_le_bytes()); // root cluster
        b[510] = 0x55;
        b[511] = 0xAA;
    }

    fn fat_offset(spf: usize, cluster: u32) -> usize {
        // FAT 0 lives right after the reserved region.
        let _ = spf;
        RESERVED * SECTOR + cluster as usize * 4
    }

    fn set_fat(img: &mut [u8], spf: usize, cluster: u32, val: u32) {
        let off = Self::fat_offset(spf, cluster);
        img[off..off + 4].copy_from_slice(&val.to_le_bytes());
    }

    fn cluster_offset(&self, cluster: u32) -> usize {
        let data_start = RESERVED + NUM_FATS * self.sectors_per_fat;
        (data_start + (cluster as usize - 2) * SPC) * SECTOR
    }

    /// Allocate `len` bytes worth of clusters, chain them, write payload, return start cluster.
    fn alloc_chain(&mut self, payload: &[u8]) -> u32 {
        let bpc = SECTOR * SPC;
        let n = payload.len().div_ceil(bpc).max(1);
        let start = self.next_free_cluster;
        let mut clusters = alloc::vec::Vec::new();
        for _ in 0..n {
            clusters.push(self.next_free_cluster);
            self.next_free_cluster += 1;
        }
        for (i, &c) in clusters.iter().enumerate() {
            let next = if i + 1 < clusters.len() {
                clusters[i + 1]
            } else {
                0x0FFF_FFFF
            };
            Self::set_fat(&mut self.img, self.sectors_per_fat, c, next);
        }
        let mut off = 0;
        for &c in &clusters {
            let dst = self.cluster_offset(c);
            let chunk = (payload.len() - off).min(bpc);
            self.img[dst..dst + chunk].copy_from_slice(&payload[off..off + chunk]);
            off += chunk;
        }
        start
    }

    /// Build one 32-byte short-name entry. `name` is a raw 8.3 already padded by caller.
    fn short_entry(name83: &[u8; 11], attr: u8, start_cluster: u32, size: u32) -> [u8; 32] {
        let mut e = [0u8; 32];
        e[0..11].copy_from_slice(name83);
        e[11] = attr;
        e[20..22].copy_from_slice(&((start_cluster >> 16) as u16).to_le_bytes());
        e[26..28].copy_from_slice(&(start_cluster as u16).to_le_bytes());
        e[28..32].copy_from_slice(&size.to_le_bytes());
        e
    }

    /// LFN slots for an ASCII long name (one short entry follows; checksum computed
    /// from its 8.3 form).
    fn lfn_slots(long: &str, short83: &[u8; 11]) -> alloc::vec::Vec<u8> {
        let cksum = lfn_checksum(short83);
        let units: alloc::vec::Vec<u16> = long.encode_utf16().collect();
        let n_slots = units.len().div_ceil(13);
        let mut out = alloc::vec::Vec::new();
        // On disk, slots are stored last-ordinal-first.
        for slot in (0..n_slots).rev() {
            let mut e = [0u8; 32];
            let ord = (slot + 1) as u8;
            let is_last = slot == n_slots - 1;
            e[0] = if is_last { ord | 0x40 } else { ord };
            e[11] = ATTR_LONG_NAME;
            e[13] = cksum;
            let ranges: [(usize, usize); 3] = [(1, 11), (14, 26), (28, 32)];
            let mut unit_idx = slot * 13;
            for (s, en) in ranges {
                let mut i = s;
                while i + 1 < en {
                    let u = if unit_idx < units.len() {
                        units[unit_idx]
                    } else if unit_idx == units.len() {
                        0x0000
                    } else {
                        0xFFFF
                    };
                    e[i..i + 2].copy_from_slice(&u.to_le_bytes());
                    unit_idx += 1;
                    i += 2;
                }
            }
            out.extend_from_slice(&e);
        }
        out
    }

    fn finish(self) -> MemBio {
        MemBio { data: self.img }
    }

    /// Like `finish`, but prepend `pad_sectors` of junk so the FAT32 volume
    /// starts at a non-zero LBA — exercises partition-relative addressing.
    fn finish_at(self, pad_sectors: usize) -> MemBio {
        let mut data = alloc::vec![0xCDu8; pad_sectors * SECTOR];
        data.extend_from_slice(&self.img);
        MemBio { data }
    }
}

fn lfn_checksum(short83: &[u8; 11]) -> u8 {
    let mut sum: u8 = 0;
    for &c in short83 {
        sum = sum.rotate_right(1).wrapping_add(c);
    }
    sum
}

fn name83(base: &str, ext: &str) -> [u8; 11] {
    let mut n = [b' '; 11];
    for (i, c) in base.bytes().take(8).enumerate() {
        n[i] = c;
    }
    for (i, c) in ext.bytes().take(3).enumerate() {
        n[8 + i] = c;
    }
    n
}

/// Build an image: root has HELLO.TXT, a dir SUB/ containing DEEP.BIN, and a
/// long-named file via LFN. Returns the engine.
fn build_fs() -> Fat32Fs<MemBio> {
    let mut b = Builder::new();

    let hello = b"Hello, FAT32 world!\n";
    let hello_cluster = b.alloc_chain(hello);

    // A file spanning >1 cluster (>512 bytes) to exercise chain walk on read.
    let big: alloc::vec::Vec<u8> = (0..1300u32).map(|i| (i & 0xFF) as u8).collect();
    let big_cluster = b.alloc_chain(&big);

    // Subdirectory with one entry (DEEP.BIN). Build its cluster blob first.
    let deep = b"deep file contents";
    let deep_cluster = b.alloc_chain(deep);

    let mut subdir_blob = alloc::vec::Vec::new();
    // "." and ".." (resolver skips them, but real dirs have them).
    subdir_blob.extend_from_slice(&Builder::short_entry(
        &name83(".", ""),
        ATTR_DIRECTORY,
        0,
        0,
    ));
    subdir_blob.extend_from_slice(&Builder::short_entry(
        &name83("..", ""),
        ATTR_DIRECTORY,
        0,
        0,
    ));
    subdir_blob.extend_from_slice(&Builder::short_entry(
        &name83("DEEP", "BIN"),
        ATTR_ARCHIVE,
        deep_cluster,
        deep.len() as u32,
    ));
    let sub_cluster = b.alloc_chain(&subdir_blob);

    // LFN file: "LongFileName.txt" with short alias LONGFI~1.TXT.
    let long_payload = b"lfn payload";
    let long_cluster = b.alloc_chain(long_payload);
    let long_short = name83("LONGFI~1", "TXT");

    // Root directory blob (cluster 2).
    let mut root = alloc::vec::Vec::new();
    root.extend_from_slice(&Builder::short_entry(
        &name83("HELLO", "TXT"),
        ATTR_ARCHIVE,
        hello_cluster,
        hello.len() as u32,
    ));
    root.extend_from_slice(&Builder::short_entry(
        &name83("BIG", "BIN"),
        ATTR_ARCHIVE,
        big_cluster,
        big.len() as u32,
    ));
    root.extend_from_slice(&Builder::short_entry(
        &name83("SUB", ""),
        ATTR_DIRECTORY,
        sub_cluster,
        0,
    ));
    root.extend_from_slice(&Builder::lfn_slots("LongFileName.txt", &long_short));
    root.extend_from_slice(&Builder::short_entry(
        &long_short,
        ATTR_ARCHIVE,
        long_cluster,
        long_payload.len() as u32,
    ));
    // Volume label entry (must be skipped by reader).
    root.extend_from_slice(&Builder::short_entry(
        &name83("VOLNAME", ""),
        ATTR_VOLUME_ID,
        0,
        0,
    ));

    // Write root blob into cluster 2 directly.
    let off = b.cluster_offset(2);
    assert!(root.len() <= SECTOR * SPC, "root dir overflows one cluster");
    b.img[off..off + root.len()].copy_from_slice(&root);

    Fat32Fs::open(b.finish(), 0).expect("mount")
}

#[test]
fn mounts_and_reads_bpb() {
    let fs = build_fs();
    assert_eq!(fs.bpb.bytes_per_sector, 512);
    assert_eq!(fs.bpb.root_cluster, 2);
    assert!(!fs.capabilities_writable());
}

#[test]
fn readdir_root() {
    let mut fs = build_fs();
    let entries = fs.readdir("/").unwrap();
    let names: alloc::vec::Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"HELLO.TXT"), "{:?}", names);
    assert!(names.contains(&"BIG.BIN"), "{:?}", names);
    assert!(names.contains(&"SUB"), "{:?}", names);
    assert!(names.contains(&"LongFileName.txt"), "{:?}", names);
    // Volume label must not appear.
    assert!(!names.contains(&"VOLNAME"), "{:?}", names);
}

#[test]
fn stat_file_and_dir() {
    let mut fs = build_fs();
    let st = fs.stat("/HELLO.TXT").unwrap();
    assert_eq!(st.file_type, FileType::Regular);
    assert_eq!(st.size, 20);

    let d = fs.stat("/SUB").unwrap();
    assert_eq!(d.file_type, FileType::Directory);
}

#[test]
fn read_small_file() {
    let mut fs = build_fs();
    let mut cookie = fs.open_file("/HELLO.TXT").unwrap();
    let mut buf = [0u8; 64];
    let n = fs.read(&mut cookie, &mut buf).unwrap();
    assert_eq!(&buf[..n], b"Hello, FAT32 world!\n");
    // EOF
    let n2 = fs.read(&mut cookie, &mut buf).unwrap();
    assert_eq!(n2, 0);
}

#[test]
fn read_multi_cluster_file() {
    let mut fs = build_fs();
    let mut cookie = fs.open_file("/BIG.BIN").unwrap();
    let mut out = alloc::vec::Vec::new();
    let mut buf = [0u8; 100];
    loop {
        let n = fs.read(&mut cookie, &mut buf).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    let expect: alloc::vec::Vec<u8> = (0..1300u32).map(|i| (i & 0xFF) as u8).collect();
    assert_eq!(out, expect);
}

#[test]
fn nested_path() {
    let mut fs = build_fs();
    let st = fs.stat("/SUB/DEEP.BIN").unwrap();
    assert_eq!(st.file_type, FileType::Regular);

    let mut cookie = fs.open_file("/SUB/DEEP.BIN").unwrap();
    let mut buf = [0u8; 64];
    let n = fs.read(&mut cookie, &mut buf).unwrap();
    assert_eq!(&buf[..n], b"deep file contents");
}

#[test]
fn long_filename_resolves() {
    let mut fs = build_fs();
    let mut cookie = fs.open_file("/LongFileName.txt").unwrap();
    let mut buf = [0u8; 64];
    let n = fs.read(&mut cookie, &mut buf).unwrap();
    assert_eq!(&buf[..n], b"lfn payload");
}

#[test]
fn missing_path_errors() {
    let mut fs = build_fs();
    assert_eq!(
        fs.stat("/nope.txt"),
        Err(crate::error::Fat32Error::NotFound)
    );
    assert_eq!(
        fs.open_file("/SUB").unwrap_err(),
        crate::error::Fat32Error::IsADirectory
    );
    assert_eq!(
        fs.readdir("/HELLO.TXT").unwrap_err(),
        crate::error::Fat32Error::NotADirectory
    );
    // A non-directory component mid-path is rejected, not silently descended.
    assert_eq!(
        fs.stat("/HELLO.TXT/x").unwrap_err(),
        crate::error::Fat32Error::NotADirectory
    );
}

/// Root resolves and reading an empty file yields 0 with no chain access.
#[test]
fn root_resolves_and_zero_len_file() {
    let mut b = Builder::new();
    // start_cluster 0 + size 0 = legitimately empty file.
    let mut root = alloc::vec::Vec::new();
    root.extend_from_slice(&Builder::short_entry(
        &name83("EMPTY", "TXT"),
        ATTR_ARCHIVE,
        0,
        0,
    ));
    let off = b.cluster_offset(2);
    b.img[off..off + root.len()].copy_from_slice(&root);
    let mut fs = Fat32Fs::open(b.finish(), 0).expect("mount");

    let st = fs.stat("/").unwrap();
    assert_eq!(st.file_type, FileType::Directory);

    let st = fs.stat("/EMPTY.TXT").unwrap();
    assert_eq!(st.size, 0);
    let mut cookie = fs.open_file("/EMPTY.TXT").unwrap();
    let mut buf = [0u8; 16];
    assert_eq!(fs.read(&mut cookie, &mut buf).unwrap(), 0);
}

/// Deleted (0xE5) and end-of-dir (0x00) markers must terminate/skip cleanly,
/// and the NT case bits at offset 0x0C must lowercase the 8.3 name.
#[test]
fn deleted_entries_and_nt_case_bits() {
    let mut b = Builder::new();
    let live = b"live\n";
    let live_cluster = b.alloc_chain(live);

    let mut root = alloc::vec::Vec::new();
    // A deleted entry (first byte 0xE5) that must be skipped.
    let mut dead = Builder::short_entry(&name83("GHOST", "TXT"), ATTR_ARCHIVE, 99, 123);
    dead[0] = 0xE5;
    root.extend_from_slice(&dead);
    // NT-case-bits entry: stored uppercase, bits say lowercase base + ext.
    let mut cased = Builder::short_entry(
        &name83("FILE", "TXT"),
        ATTR_ARCHIVE,
        live_cluster,
        live.len() as u32,
    );
    cased[12] = 0x08 | 0x10; // lower base + lower ext
    root.extend_from_slice(&cased);
    let off = b.cluster_offset(2);
    b.img[off..off + root.len()].copy_from_slice(&root);
    let mut fs = Fat32Fs::open(b.finish(), 0).expect("mount");

    let names: alloc::vec::Vec<alloc::string::String> = fs
        .readdir("/")
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(!names.iter().any(|n| n.starts_with("GHOST")), "{:?}", names);
    assert!(names.contains(&"file.txt".into()), "{:?}", names);

    // Lowercase path resolves case-insensitively to the same file.
    let mut cookie = fs.open_file("/FILE.TXT").unwrap();
    let mut buf = [0u8; 16];
    let n = fs.read(&mut cookie, &mut buf).unwrap();
    assert_eq!(&buf[..n], live);
}

/// An LFN spanning more than one slot (>13 UTF-16 units) reassembles in order.
#[test]
fn lfn_multislot() {
    let mut b = Builder::new();
    let payload = b"x";
    let cluster = b.alloc_chain(payload);
    let short = name83("AVERYL~1", "TXT");
    let long = "AVeryLongFileNameOverThirteen.txt"; // 33 chars > 1 slot

    let mut root = alloc::vec::Vec::new();
    root.extend_from_slice(&Builder::lfn_slots(long, &short));
    root.extend_from_slice(&Builder::short_entry(
        &short,
        ATTR_ARCHIVE,
        cluster,
        payload.len() as u32,
    ));
    let off = b.cluster_offset(2);
    b.img[off..off + root.len()].copy_from_slice(&root);
    let mut fs = Fat32Fs::open(b.finish(), 0).expect("mount");

    let names: alloc::vec::Vec<alloc::string::String> = fs
        .readdir("/")
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(names.contains(&long.into()), "{:?}", names);
    fs.open_file(long).expect("open by long name");
}

/// Same image mounted at a non-zero partition LBA must read identically;
/// catches a missing `lba_start` add in any addressing path.
#[test]
fn mounts_at_partition_offset() {
    let mut b = Builder::new();
    let hello = b"offset hello";
    let hello_cluster = b.alloc_chain(hello);
    let mut root = alloc::vec::Vec::new();
    root.extend_from_slice(&Builder::short_entry(
        &name83("HELLO", "TXT"),
        ATTR_ARCHIVE,
        hello_cluster,
        hello.len() as u32,
    ));
    let off = b.cluster_offset(2);
    b.img[off..off + root.len()].copy_from_slice(&root);

    let pad = 2048usize; // typical first-partition LBA
    let mut fs = Fat32Fs::open(b.finish_at(pad), pad as u64).expect("mount at offset");
    let mut cookie = fs.open_file("/HELLO.TXT").unwrap();
    let mut buf = [0u8; 32];
    let n = fs.read(&mut cookie, &mut buf).unwrap();
    assert_eq!(&buf[..n], hello);
}

/// Partial reads with a buffer smaller than the file accumulate correctly and
/// a mid-file cursor reads from the right offset.
#[test]
fn partial_and_resumed_reads() {
    let mut fs = build_fs();
    let mut cookie = fs.open_file("/HELLO.TXT").unwrap();
    let mut small = [0u8; 5];
    let n1 = fs.read(&mut cookie, &mut small).unwrap();
    assert_eq!(&small[..n1], b"Hello");
    let n2 = fs.read(&mut cookie, &mut small).unwrap();
    assert_eq!(&small[..n2], b", FAT");
    assert_eq!(cookie.cursor, 10);
}

// Raw boot-sector mutation helpers for BPB-rejection tests.
fn boot_image() -> alloc::vec::Vec<u8> {
    let b = Builder::new();
    b.finish().data
}

// Fat32Fs is not Debug, so check the Err arm with matches! rather than unwrap_err.
fn open_err(img: alloc::vec::Vec<u8>) -> crate::error::Fat32Error {
    match Fat32Fs::open(MemBio { data: img }, 0) {
        Ok(_) => panic!("expected open to fail"),
        Err(e) => e,
    }
}

#[test]
fn rejects_bad_boot_signature() {
    let mut img = boot_image();
    img[510] = 0x00;
    assert_eq!(open_err(img), crate::error::Fat32Error::NotFat32);
}

#[test]
fn rejects_fat16_markers() {
    // Non-zero 16-bit sectors-per-FAT marks FAT12/16, not FAT32.
    let mut img = boot_image();
    img[22..24].copy_from_slice(&8u16.to_le_bytes());
    assert_eq!(open_err(img), crate::error::Fat32Error::NotFat32);
}

#[test]
fn rejects_zero_sectors_per_cluster() {
    let mut img = boot_image();
    img[13] = 0; // sectors_per_cluster must be a non-zero power of two
    assert_eq!(open_err(img), crate::error::Fat32Error::BadGeometry);
}

#[test]
fn rejects_block_size_mismatch() {
    // BPB claims 1024-byte sectors but the device reports 512.
    let mut img = boot_image();
    img[11..13].copy_from_slice(&1024u16.to_le_bytes());
    assert_eq!(open_err(img), crate::error::Fat32Error::InvalidBlockSize);
}
