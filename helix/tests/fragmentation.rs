//! Fragmented-file integrity: a file whose data blocks are not physically
//! contiguous must read back byte-for-byte. Today only the first physical block
//! survives in the index, so the read path streams contiguous blocks and pulls
//! in a neighbouring file's data for every block past the first extent.

mod common;

use common::MemBio;
use morpheus_helix::HelixFs;

/// 2200 sectors -> 275 FS blocks -> exactly 16 data blocks. Small enough to
/// pack full and punch single-block holes deterministically.
const DISK_SECTORS: usize = 2200;
const BLOCK: usize = 4096;

fn forced_fragmentation() -> (MemBio, HelixFs, Vec<u8>) {
    let mut dev = MemBio::new(DISK_SECTORS);
    let sectors = dev.sectors();
    let mut fs = HelixFs::format_and_mount(&mut dev, 0, sectors, 512, "frag", [0u8; 16]).unwrap();

    // Fill all 16 data blocks with 16 one-block files, each a distinct pattern.
    for i in 0..16u8 {
        let buf = vec![0x10u8 + i; BLOCK];
        fs.write(&mut dev, &format!("/f{i}"), &buf, 100 + i as u64)
            .unwrap();
    }

    // Punch single-block holes: free every odd block. No two free blocks are
    // adjacent, so a 2-block allocation cannot be contiguous.
    for i in (1..16u8).step_by(2) {
        fs.unlink(&mut dev, &format!("/f{i}"), 500).unwrap();
    }

    // This 2-block file must fragment across two non-adjacent holes.
    let frag = vec![0xEEu8; 2 * BLOCK];
    fs.write(&mut dev, "/frag", &frag, 900).unwrap();

    (dev, fs, frag)
}

#[test]
fn fragmented_file_reads_back_intact() {
    let (mut dev, fs, written) = forced_fragmentation();

    let read = fs.read(&mut dev, "/frag").unwrap();

    assert_eq!(
        read.len(),
        written.len(),
        "fragmented file size changed on read"
    );
    assert_eq!(
        read, written,
        "fragmented file read back corrupt: blocks past the first extent came from a neighbouring file"
    );
}

#[test]
fn unlink_fragmented_frees_only_its_own_blocks() {
    let (mut dev, mut fs, _written) = forced_fragmentation();

    // /frag owns two scattered data blocks plus one extent-node block (3 total).
    let free_before = fs.bitmap.free_count();
    let evens: [u64; 8] = [0, 2, 4, 6, 8, 10, 12, 14]; // the still-live neighbour files

    fs.unlink(&mut dev, "/frag", 1000).unwrap();

    assert_eq!(
        fs.bitmap.free_count(),
        free_before + 3,
        "unlink freed the wrong number of blocks (data runs + node block)"
    );
    for b in evens {
        assert!(
            fs.bitmap.is_allocated(b),
            "unlink wrongly freed a live neighbour block {b}"
        );
    }
}

#[test]
fn fragmented_file_survives_remount_with_blocks_reserved() {
    let (mut dev, mut fs, written) = forced_fragmentation();
    // Capture /frag's physical blocks (two data runs + the node block) to assert
    // the rebuild reserves exactly them.
    let owned = frag_blocks(&mut dev, &fs);
    fs.sync(&mut dev).unwrap();
    drop(fs);

    let fs2 = HelixFs::mount(&mut dev, 0, 512).unwrap();
    assert_eq!(
        fs2.read(&mut dev, "/frag").unwrap(),
        written,
        "fragmented file corrupt after remount (node not replayed)"
    );
    for b in owned {
        assert!(
            fs2.bitmap.is_allocated(b),
            "bitmap rebuild left fragmented block {b} free; a later write would clobber it"
        );
    }
}

/// /frag's physical blocks: its extent-node block plus every run it lists.
fn frag_blocks(dev: &mut MemBio, fs: &HelixFs) -> Vec<u64> {
    let node_block = fs.index.lookup("/frag").unwrap().extent_root;
    let mut blocks = vec![node_block];
    let extents = morpheus_helix::extent::read_extent_node(
        dev,
        fs.partition_lba_start,
        fs.sb.data_start_block,
        fs.device_block_size,
        node_block,
    )
    .unwrap();
    for (_logical, physical, count) in extents {
        for j in 0..count as u64 {
            blocks.push(physical + j);
        }
    }
    blocks
}
