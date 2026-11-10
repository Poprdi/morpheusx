//! Randomized power-cut test. After any crash the volume must mount and hold
//! exactly the files of some operation boundary at or after the last sync.

mod common;

use common::{MemBioError, SECTOR};
use gpt_disk_io::BlockIo;
use gpt_disk_types::{BlockSize, Lba};
use morpheus_helix::HelixFs;
use std::collections::BTreeMap;

const DISK_SECTORS: usize = 4096;
const BLOCK: usize = 4096;

/// Disk that loses power after a set number of writes. `cached` drops every
/// write since the last flush at the cut; otherwise writes land immediately.
struct CutDisk {
    durable: Vec<u8>,
    cache: Vec<u8>,
    cached: bool,
    writes_left: u64,
}

impl CutDisk {
    fn new(cached: bool) -> Self {
        let bytes = DISK_SECTORS * SECTOR;
        Self {
            durable: vec![0; bytes],
            cache: vec![0; bytes],
            cached,
            writes_left: u64::MAX,
        }
    }

    fn power_cycle(&mut self) {
        if self.cached {
            self.cache.copy_from_slice(&self.durable);
        } else {
            self.durable.copy_from_slice(&self.cache);
        }
        self.writes_left = u64::MAX;
    }
}

impl BlockIo for CutDisk {
    type Error = MemBioError;

    fn block_size(&self) -> BlockSize {
        BlockSize::BS_512
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        Ok(DISK_SECTORS as u64)
    }

    fn read_blocks(&mut self, start: Lba, dst: &mut [u8]) -> Result<(), Self::Error> {
        let off = start.0 as usize * SECTOR;
        dst.copy_from_slice(&self.cache[off..off + dst.len()]);
        Ok(())
    }

    fn write_blocks(&mut self, start: Lba, src: &[u8]) -> Result<(), Self::Error> {
        if self.writes_left == 0 {
            return Err(MemBioError("power cut"));
        }
        self.writes_left -= 1;
        let off = start.0 as usize * SECTOR;
        self.cache[off..off + src.len()].copy_from_slice(src);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        if self.writes_left == 0 {
            return Err(MemBioError("power cut"));
        }
        self.durable.copy_from_slice(&self.cache);
        Ok(())
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

type Model = BTreeMap<String, Vec<u8>>;

fn files_of(fs: &HelixFs, dev: &mut CutDisk) -> Result<Model, String> {
    let mut out = Model::new();
    for e in fs.index.all_entries() {
        if e.flags & 0b11 != 0 {
            continue; // dir or tombstone
        }
        let path = morpheus_helix::index::btree::path_str(&e.path).to_string();
        let data = fs
            .read(dev, &path)
            .map_err(|err| format!("{path}: {err:?}"))?;
        out.insert(path, data);
    }
    Ok(out)
}

fn run(seed: u64, cached: bool) {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut dev = CutDisk::new(cached);
    let mut fs =
        HelixFs::format_and_mount(&mut dev, 0, DISK_SECTORS as u64, 512, "t", [0; 16]).unwrap();

    let mut model = Model::new();
    let mut history = vec![model.clone()];
    let mut synced = 0usize;
    dev.writes_left = 20 + rng.below(600);

    for step in 0..400u64 {
        let name = format!("/d{}/f{}", rng.below(2), rng.below(6));
        let failed = match rng.below(10) {
            0..=4 => {
                let len = match rng.below(4) {
                    0 => rng.below(97) as usize,
                    1 => BLOCK,
                    _ => rng.below(6 * BLOCK as u64) as usize,
                };
                let data = vec![(step % 251) as u8 + 1; len];
                let r = fs.write(&mut dev, &name, &data, step);
                if r.is_ok() {
                    model.insert(name, data);
                }
                r.is_err()
            },
            5 => {
                let r = fs.unlink(&mut dev, &name, step);
                if r.is_ok() {
                    model.remove(&name);
                }
                r.is_err() && model.contains_key(&name)
            },
            6 => {
                let to = format!("/d{}/f{}", rng.below(2), rng.below(6));
                let r = fs.rename(&mut dev, &name, &to, step);
                if r.is_ok() && name != to {
                    let v = model.remove(&name).unwrap();
                    model.insert(to, v);
                }
                r.is_err() && model.contains_key(&name)
            },
            7 => {
                let r = fs.sync(&mut dev);
                if r.is_ok() {
                    synced = history.len() - 1;
                }
                r.is_err()
            },
            8 => {
                let r = if rng.below(2) == 0 {
                    fs.snapshot(&mut dev, "s", step).map(|_| ())
                } else {
                    fs.checkpoint(&mut dev)
                };
                if r.is_ok() {
                    synced = history.len() - 1;
                }
                r.is_err()
            },
            _ => false,
        };
        history.push(model.clone());
        // an op may fail for lack of space; only a dead disk ends the run
        if failed && dev.writes_left == 0 {
            break;
        }
    }

    drop(fs);
    dev.power_cycle();

    let mut fs = HelixFs::mount(&mut dev, 0, 512)
        .unwrap_or_else(|e| panic!("seed {seed} cached={cached}: mount failed: {e:?}"));
    let got = files_of(&fs, &mut dev)
        .unwrap_or_else(|e| panic!("seed {seed} cached={cached}: unreadable file {e}"));
    assert!(
        history[synced..].contains(&got),
        "seed {seed} cached={cached}: recovered state matches no boundary since the last sync"
    );

    // the recovered volume keeps working (inline write: the disk may be full)
    fs.write(&mut dev, "/after", b"ok", 1).unwrap();
    fs.sync(&mut dev).unwrap();
    drop(fs);
    let fs = HelixFs::mount(&mut dev, 0, 512).unwrap();
    assert_eq!(fs.read(&mut dev, "/after").unwrap(), b"ok");
}

#[test]
fn power_cut_write_through() {
    for seed in 1..=300 {
        run(seed, false);
    }
}

#[test]
fn power_cut_write_cache() {
    for seed in 1..=300 {
        run(seed, true);
    }
}
