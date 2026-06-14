//! Host-side HelixFS utility: inject ELF binaries into `helix-data.img` so they
//! exist when MorpheusX boots in QEMU.

use std::env;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use gpt_disk_io::BlockIo;
use gpt_disk_types::{BlockSize, Lba};

use morpheus_helix::error::HelixError;
use morpheus_helix::log::recovery::recover_superblock;
use morpheus_helix::HelixFs;

const SECTOR_SIZE: u32 = 512;

/// "MXROOT" volume UUID stamped into a freshly formatted image.
const MXROOT_UUID: [u8; 16] = [
    0x4D, 0x58, 0x52, 0x4F, 0x4F, 0x54, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
];

struct FileBlockDevice {
    file: File,
    total_sectors: u64,
}

impl FileBlockDevice {
    fn open(path: &str) -> io::Result<Self> {
        let mut file = OpenOptions::new().read(true).write(true).open(path)?;
        let mut len = file.metadata()?.len();
        if len == 0 {
            // Block devices report 0 via metadata; fall back to seek-to-end.
            len = file.seek(SeekFrom::End(0))?;
        }
        let total_sectors = len / SECTOR_SIZE as u64;
        Ok(Self {
            file,
            total_sectors,
        })
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
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|_| FileIoError)?;
        self.file.read_exact(dst).map_err(|_| FileIoError)
    }

    fn write_blocks(&mut self, start_lba: Lba, src: &[u8]) -> Result<(), Self::Error> {
        let offset = start_lba.0 * SECTOR_SIZE as u64;
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|_| FileIoError)?;
        self.file.write_all(src).map_err(|_| FileIoError)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.file.flush().map_err(|_| FileIoError)
    }
}

fn usage() {
    eprintln!("morpheus-cli — MorpheusX HelixFS host utility");
    eprintln!();
    eprintln!("USAGE:");
    eprintln!("  morpheus-cli inject <disk-image> <binary> [--dest /bin/name]");
    eprintln!("  morpheus-cli pack   <disk-image> <output-image> [--max-mb N]");
    eprintln!("  morpheus-cli ls     <disk-image> [path]");
    eprintln!("  morpheus-cli mkbin  <disk-image>");
    eprintln!();
    eprintln!("EXAMPLES:");
    eprintln!(
        "  morpheus-cli inject testing/helix-data.img \\\n      target/x86_64-morpheus/release/syscall-e2e"
    );
    eprintln!("  morpheus-cli inject testing/helix-data.img my-app --dest /bin/app");
    eprintln!("  morpheus-cli pack /dev/sdb2 testing/helix.img --max-mb 384");
    eprintln!("  morpheus-cli ls testing/helix-data.img /bin");
}

fn cmd_pack(disk: &str, output: &str, max_mb: u64) -> Result<(), String> {
    if max_mb == 0 {
        return Err("--max-mb must be > 0".to_string());
    }

    let mut dev =
        FileBlockDevice::open(disk).map_err(|e| format!("cannot open '{}': {}", disk, e))?;

    let sb = recover_superblock(&mut dev, 0, SECTOR_SIZE)
        .map_err(|e| format!("recover_superblock: {:?}", e))?;

    let mut stage_blocks = 2u64;
    let log_hi = sb.log_end_block.saturating_add(1);
    if log_hi > stage_blocks {
        stage_blocks = log_hi;
    }

    let data_hi = sb.data_start_block.saturating_add(sb.blocks_used);
    if data_hi > stage_blocks {
        stage_blocks = data_hi;
    }

    if stage_blocks > sb.total_blocks {
        stage_blocks = sb.total_blocks;
    }

    if stage_blocks == 0 {
        return Err("empty Helix footprint".to_string());
    }

    let mut bytes = stage_blocks
        .checked_mul(sb.block_size as u64)
        .ok_or_else(|| "footprint byte overflow".to_string())?;

    let max_bytes = max_mb
        .checked_mul(1024)
        .and_then(|v| v.checked_mul(1024))
        .ok_or_else(|| "max size overflow".to_string())?;

    if bytes > max_bytes {
        bytes = max_bytes;
    }

    let rem = bytes % SECTOR_SIZE as u64;
    if rem != 0 {
        bytes = bytes.saturating_sub(rem);
    }

    if bytes == 0 {
        return Err("packed image size resolved to 0 bytes".to_string());
    }

    let mut out = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(output)
        .map_err(|e| format!("cannot create '{}': {}", output, e))?;

    const CHUNK: usize = 1024 * 1024;
    let mut buf = vec![0u8; CHUNK];
    let mut copied = 0u64;

    while copied < bytes {
        let remaining = (bytes - copied) as usize;
        let n = remaining.min(CHUNK);
        dev.file
            .seek(SeekFrom::Start(copied))
            .map_err(|_| "seek failed".to_string())?;
        dev.file
            .read_exact(&mut buf[..n])
            .map_err(|_| "read failed".to_string())?;
        out.write_all(&buf[..n])
            .map_err(|_| "write failed".to_string())?;
        copied += n as u64;
    }

    out.flush().map_err(|e| format!("flush failed: {}", e))?;

    println!("[pack] source : {}", disk);
    println!("[pack] output : {}", output);
    println!("[pack] bytes  : {}", bytes);
    println!("[pack] done");
    Ok(())
}

/// Open `disk` and mount its HelixFS engine, formatting if there's no valid /
/// compatible superblock. `HelixFs::mount` does superblock recovery, log replay,
/// and bitmap rebuild internally.
fn mount(disk: &str) -> Result<(FileBlockDevice, HelixFs), String> {
    let mut dev =
        FileBlockDevice::open(disk).map_err(|e| format!("cannot open '{}': {}", disk, e))?;

    println!(
        "[helix] disk: {} sectors × {} bytes",
        dev.total_sectors, SECTOR_SIZE
    );

    let fs = match HelixFs::mount(&mut dev, 0, SECTOR_SIZE) {
        Ok(fs) => {
            println!(
                "[helix] mounted existing superblock (version {})",
                fs.sb.version
            );
            fs
        },
        Err(e) => {
            println!("[helix] {:?} — formatting", e);
            format_disk(&mut dev)?;
            HelixFs::mount(&mut dev, 0, SECTOR_SIZE)
                .map_err(|e| format!("mount after format: {:?}", e))?
        },
    };

    Ok((dev, fs))
}

/// Format `dev` as a clean HelixFS over its whole extent.
fn format_disk(dev: &mut FileBlockDevice) -> Result<(), String> {
    let total_sectors = dev.total_sectors;
    morpheus_helix::format::format_helix(dev, 0, total_sectors, SECTOR_SIZE, "root", MXROOT_UUID)
        .map_err(|e| format!("format_helix: {:?}", e))?;
    dev.flush()
        .map_err(|_| "flush after format failed".to_string())
}

fn cmd_inject(disk: &str, binary: &str, dest: &str) -> Result<(), String> {
    println!("[inject] disk   : {}", disk);
    println!("[inject] binary : {}", binary);
    println!("[inject] dest   : {}", dest);

    let elf_bytes =
        std::fs::read(binary).map_err(|e| format!("cannot read '{}': {}", binary, e))?;
    println!(
        "[inject] binary size: {} bytes ({:.1} KB)",
        elf_bytes.len(),
        elf_bytes.len() as f64 / 1024.0
    );

    if elf_bytes.len() < 4 || &elf_bytes[0..4] != b"\x7fELF" {
        // Not an executable — a data file (e.g. a .kmap layout). Allowed.
        println!(
            "[inject] note: '{}' is not an ELF — injecting as a data file",
            binary
        );
    }

    let (mut dev, mut fs) = mount(disk)?;

    // Create every parent directory of `dest` (mkdir -p). Lets inject target any
    // path, e.g. /system/keymaps/de.kmap. write() also auto-creates parents, but
    // doing it explicitly surfaces per-dir errors and keeps the log tidy.
    {
        let comps: Vec<&str> = dest.split('/').filter(|c| !c.is_empty()).collect();
        let mut path = String::new();
        for comp in comps.iter().take(comps.len().saturating_sub(1)) {
            path.push('/');
            path.push_str(comp);
            match fs.mkdir(&path, 0) {
                Ok(()) => println!("[inject] created {}", path),
                Err(HelixError::AlreadyExists) => {},
                Err(e) => return Err(format!("mkdir {}: {:?}", path, e)),
            }
        }
    }

    fs.write(&mut dev, dest, &elf_bytes, 0)
        .map_err(|e| format!("write {}: {:?}", dest, e))?;

    // Flush log + update superblock.
    fs.sync(&mut dev).map_err(|e| format!("sync: {:?}", e))?;

    let written = elf_bytes.len();
    println!("[inject] OK — wrote {} bytes to {}", written, dest);
    println!(
        "[inject] flush complete. Boot MorpheusX and run:  exec {}",
        dest_name(dest)
    );
    Ok(())
}

fn dest_name(dest: &str) -> &str {
    dest.rsplit('/').next().unwrap_or(dest)
}

fn cmd_ls(disk: &str, path: &str) -> Result<(), String> {
    let (_dev, fs) = mount(disk)?;
    let entries = fs
        .readdir(path)
        .map_err(|e| format!("readdir {}: {:?}", path, e))?;

    println!("{}/  ({} entries)", path, entries.len());
    for e in &entries {
        let name = std::str::from_utf8(&e.name[..e.name_len as usize]).unwrap_or("?");
        let kind = if e.is_dir { "DIR " } else { "FILE" };
        println!("  {} {:>10} B   {}", kind, e.size, name);
    }
    Ok(())
}

fn cmd_format(disk: &str) -> Result<(), String> {
    let mut dev =
        FileBlockDevice::open(disk).map_err(|e| format!("cannot open '{}': {}", disk, e))?;
    println!("[format] wiping and reformatting {}", disk);
    format_disk(&mut dev)?;
    println!("[format] done — clean HelixFS ready");
    Ok(())
}

fn cmd_mkbin(disk: &str) -> Result<(), String> {
    let (mut dev, mut fs) = mount(disk)?;
    match fs.mkdir("/bin", 0) {
        Ok(()) | Err(HelixError::AlreadyExists) => {},
        Err(e) => return Err(format!("mkdir: {:?}", e)),
    }
    // Persist the directory record (mkdir only touches the in-memory log).
    fs.sync(&mut dev).map_err(|e| format!("sync: {:?}", e))?;
    println!("[mkbin] /bin ready");
    Ok(())
}

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
        },
        "ls" => {
            if args.len() < 3 {
                eprintln!("Usage: morpheus-cli ls <disk-image> [path]");
                std::process::exit(1);
            }
            let disk = &args[2];
            let path = args.get(3).map(|s| s.as_str()).unwrap_or("/");
            cmd_ls(disk, path)
        },
        "mkbin" => {
            if args.len() < 3 {
                eprintln!("Usage: morpheus-cli mkbin <disk-image>");
                std::process::exit(1);
            }
            cmd_mkbin(&args[2])
        },
        "pack" => {
            if args.len() < 4 {
                eprintln!("Usage: morpheus-cli pack <disk-image> <output-image> [--max-mb N]");
                std::process::exit(1);
            }
            let disk = &args[2];
            let output = &args[3];
            let max_mb = args
                .windows(2)
                .find(|w| w[0] == "--max-mb")
                .and_then(|w| w[1].parse::<u64>().ok())
                .unwrap_or(512);
            cmd_pack(disk, output, max_mb)
        },
        "format" => {
            if args.len() < 3 {
                eprintln!("Usage: morpheus-cli format <disk-image>");
                std::process::exit(1);
            }
            cmd_format(&args[2])
        },
        _ => {
            usage();
            std::process::exit(1);
        },
    };

    if let Err(e) = result {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}
