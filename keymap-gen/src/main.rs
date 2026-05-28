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

    let layouts: [(&str, &str, fn() -> keymap::Keymap); 2] = [
        ("de.kmap", "German (DE)", keymap::german_default),
        ("us.kmap", "US (QWERTY)", keymap::us_default),
    ];

    for (file, name, build) in layouts {
        let mut buf = [0u8; keymap::KMAP_FILE_SIZE];
        build().serialize(name, &mut buf);
        let path = dir.join(file);
        if let Err(e) = std::fs::write(&path, &buf[..]) {
            eprintln!("keymap-gen: cannot write {}: {}", path.display(), e);
            exit(1);
        }
        println!("keymap-gen: wrote {} ({} bytes)", path.display(), buf.len());
    }
}
