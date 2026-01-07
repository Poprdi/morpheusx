//! Build script for morpheus-network.
//!
//! Assembles the PCI I/O assembly file for x86_64 targets.

use std::env;
use std::process::Command;
use std::path::PathBuf;

fn main() {
    let target = env::var("TARGET").unwrap_or_default();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Always rerun if build.rs changes
    println!("cargo:rerun-if-changed=build.rs");

    // Only build assembly for x86_64 targets
    if target.contains("x86_64") {
        println!("cargo:rerun-if-changed=src/device/pci_io.S");
        println!("cargo:warning=Building PCI I/O assembly for target: {}", target);

        let asm_path = "src/device/pci_io.S";
        let obj_path = out_dir.join("pci_io.o");
        let lib_path = out_dir.join("libpci_io.a");

        // Determine output format based on target
        // UEFI uses PE/COFF format (win64)
        let obj_format = if target.contains("windows") || target.contains("uefi") {
            "win64"  // PE/COFF format for UEFI
        } else {
            "elf64"
        };

        println!("cargo:warning=Using object format: {}", obj_format);

        // Assemble with nasm
        let nasm_output = Command::new("nasm")
            .args([
                "-f", obj_format,
                "-o", obj_path.to_str().unwrap(),
                asm_path,
            ])
            .output()
            .expect("Failed to run nasm. Is nasm installed?");

        if !nasm_output.status.success() {
            let stderr = String::from_utf8_lossy(&nasm_output.stderr);
            panic!("nasm failed to assemble pci_io.S: {}", stderr);
        }

        println!("cargo:warning=Assembled {} -> {}", asm_path, obj_path.display());

        // Use regular ar - it handles COFF files fine
        // (llvm-ar would be more "correct" but ar works and is more available)
        let ar_output = Command::new("ar")
            .args([
                "crs",
                lib_path.to_str().unwrap(),
                obj_path.to_str().unwrap(),
            ])
            .output()
            .expect("Failed to run ar. Is binutils installed?");

        if !ar_output.status.success() {
            let stderr = String::from_utf8_lossy(&ar_output.stderr);
            panic!("ar failed to create libpci_io.a: {}", stderr);
        }

        println!("cargo:warning=Created static library: {}", lib_path.display());

        // Tell cargo to link the library
        println!("cargo:rustc-link-search=native={}", out_dir.display());
        println!("cargo:rustc-link-lib=static=pci_io");
    } else {
        println!("cargo:warning=Skipping PCI I/O assembly for non-x86_64 target: {}", target);
    }
}
