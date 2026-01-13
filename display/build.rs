//! Build script for morpheus-display.
//!
//! Assembles framebuffer ASM primitives using NASM.
//! Pattern matches network/build.rs.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const ASM_FILES: &[&str] = &["asm/fb.s"];

fn main() {
    let target = env::var("TARGET").unwrap_or_default();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    println!("cargo:rerun-if-changed=build.rs");
    for asm in ASM_FILES {
        println!("cargo:rerun-if-changed={}", asm);
    }

    // Only build ASM for x86_64 UEFI target
    if !target.contains("x86_64") || !target.contains("uefi") {
        println!("cargo:warning=Skipping ASM for target: {}", target);
        return;
    }

    let obj_format = "win64"; // PE/COFF for UEFI
    let mut objects = Vec::new();

    for asm_path in ASM_FILES {
        if Path::new(asm_path).exists() {
            let obj_name = Path::new(asm_path).file_stem().unwrap().to_str().unwrap();
            let obj_path = out_dir.join(format!("{}.o", obj_name));

            let status = Command::new("nasm")
                .args(&["-f", obj_format, "-o", obj_path.to_str().unwrap(), asm_path])
                .status()
                .expect("Failed to run nasm");

            if !status.success() {
                panic!("NASM failed for {}", asm_path);
            }
            objects.push(obj_path);
        }
    }

    if objects.is_empty() {
        return;
    }

    // Create static library
    let lib_path = out_dir.join("libdisplay_asm.a");
    let mut ar_args = vec!["crs".to_string(), lib_path.to_str().unwrap().to_string()];
    for obj in &objects {
        ar_args.push(obj.to_str().unwrap().to_string());
    }

    let status = Command::new("ar")
        .args(&ar_args)
        .status()
        .expect("Failed to run ar");
    if !status.success() {
        panic!("ar failed");
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=display_asm");
}
