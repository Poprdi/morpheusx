//! Build script for morpheus-bootloader.
//!
//! Assembles bootloader-local ASM using NASM.
//! Pattern mirrors hwinit/build.rs exactly:
//!   NASM → .o → ar crs libbootloader_asm.a → rustc-link-lib=static

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const ASM_KEYBOARD: &[&str] = &["asm/keyboard/ps2.s"];

fn main() {
    let target = env::var("TARGET").unwrap_or_default();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    println!("cargo:rerun-if-changed=build.rs");

    if !target.contains("x86_64") || !target.contains("uefi") {
        println!("cargo:warning=Skipping keyboard ASM for target: {}", target);
        return;
    }

    // Microsoft x64 ABI — same as hwinit
    let obj_format = "win64";
    println!(
        "cargo:warning=Building keyboard ASM for: {} ({})",
        target, obj_format
    );

    let mut objects = Vec::new();

    for path in ASM_KEYBOARD {
        println!("cargo:rerun-if-changed={}", path);
        match assemble(path, &out_dir, obj_format) {
            Ok(obj) => objects.push(obj),
            Err(e) => panic!("NASM failed on {}: {}", path, e),
        }
    }

    if objects.is_empty() {
        return;
    }

    // Pack into libbootloader_asm.a
    let lib = out_dir.join("libbootloader_asm.a");
    let mut args: Vec<String> = vec!["crs".into(), lib.to_str().unwrap().into()];
    args.extend(objects.iter().map(|p| p.to_str().unwrap().into()));

    let out = Command::new("ar").args(&args).output().expect("ar failed");
    if !out.status.success() {
        panic!("ar failed: {}", String::from_utf8_lossy(&out.stderr));
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=bootloader_asm");
}

fn assemble(path: &str, out_dir: &Path, fmt: &str) -> Result<PathBuf, String> {
    let obj_name = path.replace('/', "_").replace(".s", ".o");
    let obj = out_dir.join(&obj_name);

    let out = Command::new("nasm")
        .args(["-f", fmt, "-o", obj.to_str().unwrap(), "-I", "asm/", path])
        .output()
        .map_err(|e| e.to_string())?;

    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into());
    }
    Ok(obj)
}
