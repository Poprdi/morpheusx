//! Host tool — serialize the built-in keyboard layouts to `.kmap` files so
//! `setup-dev.sh` can provision them into the HelixFS image.
//!
//! Usage: `keymap-gen <output-dir>` → writes `<output-dir>/{de,us}.kmap`.

use std::path::Path;
use std::process::exit;

fn main() {
    let out_dir = match std::env::args().nth(1) {
        Some(d) => d,
        None => {
            eprintln!("usage: keymap-gen <output-dir>");
            exit(2);
        },
    };
    let dir = Path::new(&out_dir);
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("keymap-gen: cannot create {}: {}", out_dir, e);
        exit(1);
    }

    write_layout(dir, "de.kmap", "German (DE)", keymap::german_default());
    write_layout(dir, "us.kmap", "US (QWERTY)", keymap::us_default());
}

/// Serialize one layout to `<dir>/<file>`; exits the process on I/O error.
fn write_layout(dir: &Path, file: &str, name: &str, km: keymap::Keymap) {
    let mut buf = [0u8; keymap::KMAP_FILE_SIZE];
    km.serialize(name, &mut buf);
    let path = dir.join(file);
    if let Err(e) = std::fs::write(&path, &buf[..]) {
        eprintln!("keymap-gen: cannot write {}: {}", path.display(), e);
        exit(1);
    }
    println!("keymap-gen: wrote {} ({} bytes)", path.display(), buf.len());
}
