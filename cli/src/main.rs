//! morpheus-cli — host-side HelixFS utility for MorpheusX development.
//!
//! Injects files into a `helix-data.img` raw disk image from the host,
//! so they are available at runtime when MorpheusX boots in QEMU.
//!
//! # Usage
//!
//! ```text
//! morpheus-cli inject <disk-image> <elf-binary> [--dest /bin/name]
//! ```
//!
//! # Examples
//!
//! ```bash
//! # Inject the e2e test binary into the default location
//! cargo run -p morpheus-cli -- inject testing/helix-data.img \
//!     target/x86_64-morpheus/release/syscall-e2e
//!
//! # Inject with a custom path inside HelixFS
//! cargo run -p morpheus-cli -- inject testing/helix-data.img \
//!     target/x86_64-morpheus/release/syscall-e2e --dest /bin/e2e
//! ```

use std::env;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use gpt_disk_io::BlockIo;
use gpt_disk_types::{BlockSize, Lba};

use morpheus_helix::bitmap::BlockBitmap;
use morpheus_helix::error::HelixError;
use morpheus_helix::format;
use morpheus_helix::index::btree::NamespaceIndex;
use morpheus_helix::log::recovery::{recover_superblock, replay_log};
use morpheus_helix::types::*;
use morpheus_helix::vfs::{FdTable, HelixInstance, MountTable};
use morpheus_helix::vfs::{self};

// ──────────────────────────────────────────────────────────────────────────────
// FileBlockDevice — wraps a std::fs::File as a block device
// ──────────────────────────────────────────────────────────────────────────────

const SECTOR_SIZE: u32 = 512;

struct FileBlockDevice {
    file: File,
    total_sectors: u64,
}

impl FileBlockDevice {
    fn open(path: &str) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let len = file.metadata()?.len();
        let total_sectors = len / SECTOR_SIZE as u64;
        Ok(Self { file, total_sectors })
    }
}

#[derive(Debug, Clone, Copy)]
struct FileIoError;

impl std::fmt::Display for FileIoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "file block I/O error")
    }
}

impl BlockIo for FileBlockDevice {
    type Error = FileIoError;

    fn block_size(&self) -> BlockSize {
        BlockSize::new(SECTOR_SIZE).expect("valid sector size")
    }

    fn num_blocks(&mut self) -> Result<u64, Self::Error> {
        Ok(self.total_sectors)
    }

    fn read_blocks(&mut self, start_lba: Lba, dst: &mut [u8]) -> Result<(), Self::Error> {
        let offset = start_lba.0 * SECTOR_SIZE as u64;
        self.file.seek(SeekFrom::Start(offset)).map_err(|_| FileIoError)?;
        self.file.read_exact(dst).map_err(|_| FileIoError)
    }

    fn write_blocks(&mut self, start_lba: Lba, src: &[u8]) -> Result<(), Self::Error> {
        let offset = start_lba.0 * SECTOR_SIZE as u64;
        self.file.seek(SeekFrom::Start(offset)).map_err(|_| FileIoError)?;
        self.file.write_all(src).map_err(|_| FileIoError)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.file.flush().map_err(|_| FileIoError)
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// CLI commands
// ──────────────────────────────────────────────────────────────────────────────

fn usage() {
    eprintln!("morpheus-cli — MorpheusX HelixFS host utility");
    eprintln!();
    eprintln!("USAGE:");
    eprintln!("  morpheus-cli inject <disk-image> <binary> [--dest /bin/name]");
    eprintln!("  morpheus-cli ls     <disk-image> [path]");
    eprintln!("  morpheus-cli mkbin  <disk-image>");
    eprintln!();
    eprintln!("EXAMPLES:");
    eprintln!(
        "  morpheus-cli inject testing/helix-data.img \\\n      target/x86_64-morpheus/release/syscall-e2e"
    );
    eprintln!(
        "  morpheus-cli inject testing/helix-data.img my-app --dest /bin/app"
    );
    eprintln!("  morpheus-cli ls testing/helix-data.img /bin");
}

// Mount an existing or fresh HelixFS from a FileBlockDevice.
// Returns (device, mount_table).
fn mount(disk: &str) -> Result<(FileBlockDevice, MountTable), String> {
    let mut dev = FileBlockDevice::open(disk)
        .map_err(|e| format!("cannot open '{}': {}", disk, e))?;

    println!("[helix] disk: {} sectors × {} bytes", dev.total_sectors, SECTOR_SIZE);

    // Try to recover an existing superblock.
    let (sb, do_replay) = match recover_superblock(&mut dev, 0, SECTOR_SIZE) {
        Ok(sb) => {
            if sb.version != HELIX_VERSION {
                println!(
                    "[helix] version mismatch (disk={} expected={}) — reformatting",
                    sb.version, HELIX_VERSION
                );
                let sb = do_format(&mut dev)?;
                (sb, false) // fresh format, nothing to replay
            } else {
                println!("[helix] found valid superblock (version {})", sb.version);
                (sb, true)
            }
        }
        Err(_) => {
            println!("[helix] no valid superblock found — formatting");
            let sb = do_format(&mut dev)?;
            (sb, false)
        }
    };

    // Build in-memory HelixInstance.
    let mut instance = HelixInstance {
        sb,
        log: morpheus_helix::log::LogEngine::new(&sb, 0, SECTOR_SIZE),
        index: NamespaceIndex::new(),
        bitmap: BlockBitmap::new(sb.data_block_count),
        partition_lba_start: 0,
        device_block_size: SECTOR_SIZE,
    };

    // Reload the head log segment (so future writes append correctly).
    instance
        .log
        .reload_head_segment(&mut dev)
        .map_err(|e| format!("reload_head_segment: {:?}", e))?;

    if do_replay {
        // Replay the log to rebuild the in-memory index.
        replay_log(&mut dev, &instance.log, &mut instance.index)
            .map_err(|e| format!("replay_log: {:?}", e))?;

        // Rebuild bitmap so we don't overwrite existing file data.
        rebuild_bitmap(&mut instance);

        println!(
            "[helix] replayed log — {} index entries",
            instance.index.all_entries().len()
        );
    }

    let mut mount_table = MountTable::new();
    mount_table
        .mount("/", instance, false)
        .map_err(|e| format!("mount: {:?}", e))?;

    Ok((dev, mount_table))
}

fn do_format(dev: &mut FileBlockDevice) -> Result<HelixSuperblock, String> {
    let total_sectors = dev.total_sectors;
    let uuid = [
        0x4D, 0x58, 0x52, 0x4F, 0x4F, 0x54, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
    ]; // "MXROOT"
    format::format_helix(dev, 0, total_sectors, SECTOR_SIZE, "root", uuid)
        .map_err(|e| format!("format_helix: {:?}", e))?;
    recover_superblock(dev, 0, SECTOR_SIZE)
        .map_err(|e| format!("recover after format: {:?}", e))
}

/// Rebuild the block bitmap from index entries (mirrors vfs/global.rs private fn).
fn rebuild_bitmap(instance: &mut HelixInstance) {
    for entry in instance.index.all_entries() {
        if entry.flags & entry_flags::IS_DELETED != 0 { continue; }
        if entry.flags & entry_flags::IS_DIR != 0 { continue; }
        if entry.flags & entry_flags::IS_INLINE != 0 { continue; }
        if entry.extent_root == BLOCK_NULL { continue; }
        let blocks = entry.size.div_ceil(BLOCK_SIZE as u64);
        if blocks > 0 {
            instance.bitmap.mark_range_used(entry.extent_root, blocks);
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// inject command
// ──────────────────────────────────────────────────────────────────────────────

fn cmd_inject(disk: &str, binary: &str, dest: &str) -> Result<(), String> {
    println!("[inject] disk   : {}", disk);
    println!("[inject] binary : {}", binary);
    println!("[inject] dest   : {}", dest);

    // Read ELF bytes from host filesystem.
    let elf_bytes = std::fs::read(binary)
        .map_err(|e| format!("cannot read '{}': {}", binary, e))?;
    println!("[inject] binary size: {} bytes ({:.1} KB)", elf_bytes.len(), elf_bytes.len() as f64 / 1024.0);

    // Validate it looks like an ELF64.
    if elf_bytes.len() < 4 || &elf_bytes[0..4] != b"\x7fELF" {
        return Err(format!("'{}' does not appear to be an ELF binary", binary));
    }

    let (mut dev, mut mt) = mount(disk)?;
    let mut fdt = FdTable::new();

    // Ensure /bin directory exists (note: mkdir is index-only, no block_io needed).
    match vfs::vfs_mkdir(&mut mt, "/bin", 0) {
        Ok(()) => println!("[inject] created /bin"),
        Err(HelixError::AlreadyExists) => println!("[inject] /bin already exists"),
        Err(e) => return Err(format!("vfs_mkdir /bin: {:?}", e)),
    }

    // Open (create / overwrite) destination path.
    let flags = open_flags::O_WRITE | open_flags::O_CREATE | open_flags::O_TRUNC;
    let fd = vfs::vfs_open(&mut dev, &mut mt, &mut fdt, dest, flags, 0)
        .map_err(|e| format!("vfs_open {}: {:?}", dest, e))?;

    // Write ELF data.
    let written = vfs::vfs_write(&mut dev, &mut mt, &mut fdt, fd, &elf_bytes, 0)
        .map_err(|e| format!("vfs_write: {:?}", e))?;

    vfs::vfs_close(&mut fdt, fd)
        .map_err(|e| format!("vfs_close: {:?}", e))?;

    // Flush log + update superblock.
    vfs::vfs_sync(&mut dev, &mut mt)
        .map_err(|e| format!("vfs_sync: {:?}", e))?;

    println!("[inject] OK — wrote {} bytes to {}", written, dest);
    println!("[inject] flush complete. Boot MorpheusX and run:  exec {}", dest_name(dest));
    Ok(())
}

fn dest_name(dest: &str) -> &str {
    dest.rsplit('/').next().unwrap_or(dest)
}

// ──────────────────────────────────────────────────────────────────────────────
// ls command
// ──────────────────────────────────────────────────────────────────────────────

fn cmd_ls(disk: &str, path: &str) -> Result<(), String> {
    let (_dev, mt) = mount(disk)?;
    let entries = vfs::vfs_readdir(&mt, path)
        .map_err(|e| format!("vfs_readdir {}: {:?}", path, e))?;

    println!("{}/  ({} entries)", path, entries.len());
    for e in &entries {
        let name = std::str::from_utf8(&e.name[..e.name_len as usize]).unwrap_or("?");
        let kind = if e.is_dir { "DIR " } else { "FILE" };
        println!("  {} {:>10} B   {}", kind, e.size, name);
    }
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// mkbin command — pre-create /bin directory
// ──────────────────────────────────────────────────────────────────────────────

fn cmd_mkbin(disk: &str) -> Result<(), String> {
    let (_dev, mut mt) = mount(disk)?;
    match vfs::vfs_mkdir(&mut mt, "/bin", 0) {
        Ok(()) | Err(HelixError::AlreadyExists) => {
            println!("[mkbin] /bin ready");
            Ok(())
        }
        Err(e) => Err(format!("vfs_mkdir: {:?}", e)),
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// entry point
// ──────────────────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        usage();
        std::process::exit(1);
    }

    let result = match args[1].as_str() {
        "inject" => {
            if args.len() < 4 {
                eprintln!("Usage: morpheus-cli inject <disk-image> <binary> [--dest /bin/name]");
                std::process::exit(1);
            }
            let disk = &args[2];
            let binary = &args[3];

            // Default dest = /bin/<filename>
            let default_dest = format!(
                "/bin/{}",
                Path::new(binary)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("binary")
            );
            let dest = args
                .windows(2)
                .find(|w| w[0] == "--dest")
                .map(|w| w[1].as_str())
                .unwrap_or(&default_dest);

            cmd_inject(disk, binary, dest)
        }
        "ls" => {
            if args.len() < 3 {
                eprintln!("Usage: morpheus-cli ls <disk-image> [path]");
                std::process::exit(1);
            }
            let disk = &args[2];
            let path = args.get(3).map(|s| s.as_str()).unwrap_or("/");
            cmd_ls(disk, path)
        }
        "mkbin" => {
            if args.len() < 3 {
                eprintln!("Usage: morpheus-cli mkbin <disk-image>");
                std::process::exit(1);
            }
            cmd_mkbin(&args[2])
        }
        _ => {
            usage();
            std::process::exit(1);
        }
    };

    if let Err(e) = result {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}
