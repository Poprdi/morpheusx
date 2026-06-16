//! Partition-table enumeration: GPT-first with MBR fallback. Refactored out of
//! the bootloader's `select_data_region` (one-shot "pick one region" policy) into
//! a reusable scan returning ALL partitions, for the storage subsystem's volume
//! layer (spec §3 layer 2, §9 step 1a).

use alloc::vec::Vec;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

/// GPT name field is 72 bytes (36 UTF-16LE code units); stored raw.
pub const PART_NAME_LEN: usize = 72;

const GPT_SIG: &[u8; 8] = b"EFI PART";

/// One partition discovered on a device. `name` is the raw on-disk label bytes
/// (UTF-16LE for GPT, zeroed for MBR); `type_guid` is the GPT type GUID
/// (little-endian on-disk) or a synthetic `[mbr_type, 0, ..]` for MBR.
#[derive(Clone, Copy)]
pub struct PartitionEntry {
    pub lba_start: u64,
    pub lba_count: u64,
    pub type_guid: [u8; 16],
    pub name: [u8; PART_NAME_LEN],
}

#[inline(always)]
fn le_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

#[inline(always)]
fn le_u64(buf: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        buf[off],
        buf[off + 1],
        buf[off + 2],
        buf[off + 3],
        buf[off + 4],
        buf[off + 5],
        buf[off + 6],
        buf[off + 7],
    ])
}

/// Enumerate all partitions on `dev`: parse the GPT header at LBA 1 and walk its
/// entry array; if no GPT magic, fall back to the four MBR primary slots. Returns
/// an empty Vec on an unpartitioned/whole-disk device or any read error.
pub fn enumerate_partitions<B: BlockIo>(dev: &mut B) -> Vec<PartitionEntry> {
    let sector_size = dev.block_size().to_u64() as usize;
    if sector_size == 0 {
        return Vec::new();
    }

    let total_sectors = dev.num_blocks().unwrap_or(0);

    // LBA0 (MBR/protective) + LBA1 (GPT header).
    let mut first_two = alloc::vec![0u8; sector_size * 2];
    if dev.read_blocks(Lba(0), &mut first_two).is_err() {
        return Vec::new();
    }

    let has_gpt =
        first_two.len() >= sector_size + 8 && &first_two[sector_size..sector_size + 8] == GPT_SIG;

    if has_gpt {
        return enumerate_gpt(dev, &first_two, sector_size);
    }

    let has_mbr = first_two.len() >= 512 && first_two[510] == 0x55 && first_two[511] == 0xAA;
    if has_mbr {
        return enumerate_mbr(&first_two, total_sectors);
    }

    Vec::new()
}

fn enumerate_gpt<B: BlockIo>(
    dev: &mut B,
    first_two: &[u8],
    sector_size: usize,
) -> Vec<PartitionEntry> {
    let hdr = &first_two[sector_size..sector_size * 2];
    let entries_lba = le_u64(hdr, 72);
    let num_entries = le_u32(hdr, 80) as usize;
    let entry_size = le_u32(hdr, 84) as usize;

    let mut out = Vec::new();
    if entry_size < 56 || num_entries == 0 {
        return out;
    }

    let entries_per_sector = sector_size / entry_size;
    if entries_per_sector == 0 {
        return out;
    }

    let mut sec = alloc::vec![0u8; sector_size];
    for idx in 0..num_entries {
        let sector_delta = (idx / entries_per_sector) as u64;
        let idx_in_sector = idx % entries_per_sector;
        let lba = match entries_lba.checked_add(sector_delta) {
            Some(v) => v,
            None => break,
        };

        if dev.read_blocks(Lba(lba), &mut sec).is_err() {
            break;
        }

        let off = idx_in_sector * entry_size;
        let ent = &sec[off..off + entry_size];

        // First 16 bytes all-zero = unused entry.
        if ent[..16].iter().all(|b| *b == 0) {
            continue;
        }

        let first_lba = le_u64(ent, 32);
        let last_lba = le_u64(ent, 40);
        if first_lba == 0 || last_lba < first_lba {
            continue;
        }

        let mut type_guid = [0u8; 16];
        type_guid.copy_from_slice(&ent[..16]);

        let mut name = [0u8; PART_NAME_LEN];
        if entry_size >= 56 {
            let avail = core::cmp::min(PART_NAME_LEN, entry_size - 56);
            name[..avail].copy_from_slice(&ent[56..56 + avail]);
        }

        out.push(PartitionEntry {
            lba_start: first_lba,
            lba_count: last_lba - first_lba + 1,
            type_guid,
            name,
        });
    }

    out
}

fn enumerate_mbr(first_two: &[u8], total_sectors: u64) -> Vec<PartitionEntry> {
    const MBR_PART_OFF: usize = 446;
    const MBR_PART_SIZE: usize = 16;
    const MBR_PARTS: usize = 4;

    let mut out = Vec::new();
    // Index `i` also derives the byte offset into the table, not just a slot index.
    #[allow(clippy::needless_range_loop)]
    for i in 0..MBR_PARTS {
        let off = MBR_PART_OFF + i * MBR_PART_SIZE;
        let ptype = first_two[off + 4];

        // empty / GPT protective / ESP / extended containers
        if ptype == 0x00 || ptype == 0xEE || ptype == 0xEF {
            continue;
        }
        if ptype == 0x05 || ptype == 0x0F || ptype == 0x85 {
            continue;
        }

        let start = le_u32(first_two, off + 8) as u64;
        let sectors = le_u32(first_two, off + 12) as u64;
        if start == 0 || sectors == 0 {
            continue;
        }
        if total_sectors != 0 && start.saturating_add(sectors) > total_sectors {
            continue;
        }

        // MBR has no GUID/name; encode the type byte so callers can still sniff.
        let mut type_guid = [0u8; 16];
        type_guid[0] = ptype;

        out.push(PartitionEntry {
            lba_start: start,
            lba_count: sectors,
            type_guid,
            name: [0u8; PART_NAME_LEN],
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use morpheus_block_types::MemBlockDevice;

    const SECTOR: usize = 512;

    /// In-memory disk image; caller must keep alive for the device's lifetime.
    struct Img {
        buf: alloc::vec::Vec<u8>,
    }

    impl Img {
        fn new(sectors: usize) -> Self {
            Img {
                buf: alloc::vec![0u8; sectors * SECTOR],
            }
        }

        fn sector_mut(&mut self, lba: usize) -> &mut [u8] {
            &mut self.buf[lba * SECTOR..(lba + 1) * SECTOR]
        }

        fn dev(&mut self) -> MemBlockDevice {
            // SAFETY: self.buf is a live heap region of `len` bytes that outlives
            // the device (the test holds `Img` past every dev() use); the device
            // never reads/writes past total_bytes.
            unsafe { MemBlockDevice::new(self.buf.as_mut_ptr(), self.buf.len(), SECTOR as u32) }
        }
    }

    fn write_u32(b: &mut [u8], off: usize, v: u32) {
        b[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn write_u64(b: &mut [u8], off: usize, v: u64) {
        b[off..off + 8].copy_from_slice(&v.to_le_bytes());
    }

    fn build_gpt(sectors: usize, entries_lba: u64, entries: &[(u64, u64, u8, &str)]) -> Img {
        let entry_size = 128usize;
        let num_entries = 128u32;
        let mut img = Img::new(sectors);

        // Protective MBR boot signature at LBA0 so a stray reader still sees one,
        // but GPT detection keys off the LBA1 magic regardless.
        {
            let m = img.sector_mut(0);
            m[510] = 0x55;
            m[511] = 0xAA;
        }

        {
            let hdr = img.sector_mut(1);
            hdr[0..8].copy_from_slice(GPT_SIG);
            write_u64(hdr, 72, entries_lba);
            write_u32(hdr, 80, num_entries);
            write_u32(hdr, 84, entry_size as u32);
        }

        for (i, &(first, last, gtype, name)) in entries.iter().enumerate() {
            let lba = entries_lba as usize + (i * entry_size) / SECTOR;
            let off = (i * entry_size) % SECTOR;
            let sec = img.sector_mut(lba);
            let ent = &mut sec[off..off + entry_size];
            // Non-zero type GUID marks the entry used.
            ent[0] = gtype;
            ent[1] = 0xAA;
            write_u64(ent, 32, first);
            write_u64(ent, 40, last);
            // Name as UTF-16LE at offset 56.
            for (j, u) in name.encode_utf16().enumerate() {
                let p = 56 + j * 2;
                ent[p..p + 2].copy_from_slice(&u.to_le_bytes());
            }
        }
        img
    }

    fn build_mbr(sectors: usize, parts: &[(u8, u32, u32)]) -> Img {
        let mut img = Img::new(sectors);
        let m = img.sector_mut(0);
        for (i, &(ptype, start, count)) in parts.iter().enumerate() {
            let off = 446 + i * 16;
            m[off + 4] = ptype;
            write_u32(m, off + 8, start);
            write_u32(m, off + 12, count);
        }
        m[510] = 0x55;
        m[511] = 0xAA;
        img
    }

    #[test]
    fn gpt_multiple_partitions() {
        let mut img = build_gpt(64, 2, &[(40, 47, 0x11, "ESP"), (48, 63, 0x22, "DATA")]);
        let mut dev = img.dev();
        let parts = enumerate_partitions(&mut dev);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].lba_start, 40);
        assert_eq!(parts[0].lba_count, 8);
        assert_eq!(parts[0].type_guid[0], 0x11);
        assert_eq!(parts[1].lba_start, 48);
        assert_eq!(parts[1].lba_count, 16);
        // UTF-16LE name preserved raw.
        assert_eq!(&parts[1].name[0..2], &[b'D', 0]);
    }

    #[test]
    fn gpt_skips_unused_and_garbage_entries() {
        // Slot 0 used, slot 1 zero (unused), slot 2 has last < first (garbage).
        let mut img = build_gpt(64, 2, &[(40, 47, 0x11, "OK")]);
        {
            // Hand-write a third entry with last < first at index 2.
            let entry_size = 128usize;
            let lba = 2 + (2 * entry_size) / SECTOR;
            let off = (2 * entry_size) % SECTOR;
            let sec = img.sector_mut(lba);
            let ent = &mut sec[off..off + entry_size];
            ent[0] = 0x33;
            write_u64(ent, 32, 50);
            write_u64(ent, 40, 49);
        }
        let mut dev = img.dev();
        let parts = enumerate_partitions(&mut dev);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].lba_start, 40);
    }

    #[test]
    fn gpt_entries_spanning_multiple_sectors() {
        // 128-byte entries, 4 per sector; put used entries at index 0 and 5
        // (sector 2 and sector 3) to exercise the per-sector read loop.
        let mut img = build_gpt(64, 2, &[(40, 43, 0x11, "A")]);
        {
            let entry_size = 128usize;
            let idx = 5usize;
            let lba = 2 + (idx * entry_size) / SECTOR;
            let off = (idx * entry_size) % SECTOR;
            let sec = img.sector_mut(lba);
            let ent = &mut sec[off..off + entry_size];
            ent[0] = 0x22;
            write_u64(ent, 32, 44);
            write_u64(ent, 40, 47);
        }
        let mut dev = img.dev();
        let parts = enumerate_partitions(&mut dev);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].lba_start, 40);
        assert_eq!(parts[1].lba_start, 44);
    }

    #[test]
    fn mbr_fallback_multiple() {
        // No GPT magic at LBA1 → MBR path. Linux-data (0x83) + FAT32-LBA (0x0C).
        let mut img = build_mbr(256, &[(0x83, 64, 100), (0x0C, 164, 90)]);
        let mut dev = img.dev();
        let parts = enumerate_partitions(&mut dev);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].lba_start, 64);
        assert_eq!(parts[0].lba_count, 100);
        // MBR type byte encoded into type_guid[0].
        assert_eq!(parts[0].type_guid[0], 0x83);
        assert_eq!(parts[1].type_guid[0], 0x0C);
        // No GPT name for MBR.
        assert!(parts[0].name.iter().all(|&b| b == 0));
    }

    #[test]
    fn mbr_skips_protective_extended_and_out_of_range() {
        let mut img = build_mbr(
            128,
            &[
                (0xEE, 1, 127),  // GPT protective → skip
                (0x05, 1, 10),   // extended container → skip
                (0x83, 10, 200), // start+count past disk end → skip
                (0x07, 20, 50),  // valid NTFS
            ],
        );
        let mut dev = img.dev();
        let parts = enumerate_partitions(&mut dev);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].type_guid[0], 0x07);
        assert_eq!(parts[0].lba_start, 20);
    }

    #[test]
    fn no_table_returns_empty() {
        // Neither GPT magic nor an MBR boot signature.
        let mut img = Img::new(16);
        let mut dev = img.dev();
        let parts = enumerate_partitions(&mut dev);
        assert!(parts.is_empty());
    }

    #[test]
    fn empty_mbr_table_returns_empty() {
        // Boot signature present but all four slots empty (type 0x00).
        let mut img = build_mbr(64, &[]);
        let mut dev = img.dev();
        let parts = enumerate_partitions(&mut dev);
        assert!(parts.is_empty());
    }

    #[test]
    fn gpt_zero_entries_returns_empty() {
        let mut img = build_gpt(64, 2, &[(40, 47, 0x11, "X")]);
        // Stomp num_entries to 0; header still has the magic.
        write_u32(img.sector_mut(1), 80, 0);
        let mut dev = img.dev();
        let parts = enumerate_partitions(&mut dev);
        assert!(parts.is_empty());
    }
}
