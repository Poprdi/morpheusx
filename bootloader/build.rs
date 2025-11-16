// Build script to compile assembly files

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let target = env::var("TARGET").unwrap();

    // Only build assembly if we're on a Unix-like system with nasm and ar.
    // The PE/COFF object format is specific to the x86_64-unknown-uefi target.
    if target.contains("uefi") {
        build_trampoline_for_uefi();
    }
}

fn build_trampoline_for_uefi() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    let obj_file = out_dir.join("trampoline32.obj");
    let lib_file = out_dir.join("libtrampoline32.a");

    // Clean old files to avoid mixing ELF and COFF
    let _ = std::fs::remove_file(&obj_file);
    let _ = std::fs::remove_file(&lib_file);
    let _ = std::fs::remove_file(out_dir.join("trampoline32.o"));

    // Use nasm to create COFF object file directly (compatible with PE/UEFI)
    let status = Command::new("nasm")
        .args([
            "-f",
            "win64", // Output COFF format for 64-bit Windows/UEFI
            "-o",
            obj_file.to_str().unwrap(),
            "src/boot/arch/x86_64/trampoline32.asm",
        ])
        .status()
        .expect("Failed to assemble trampoline32.asm with nasm");

    assert!(status.success(), "Assembly failed");

    // Create static library with ONLY the COFF object
    let status = Command::new("ar")
        .args([
            "crus",
            lib_file.to_str().unwrap(),
            obj_file.to_str().unwrap(),
        ])
        .status()
        .expect("Failed to create static library");

    assert!(status.success(), "Archive creation failed");

    // Tell Cargo to link the library
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=trampoline32");
    println!("cargo:rerun-if-changed=src/boot/arch/x86_64/trampoline32.asm");
}
