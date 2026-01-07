//! Build script for morpheus-network.
//!
//! Assembles the PCI I/O assembly file for x86_64 targets.

use std::env;
use std::process::Command;
use std::path::PathBuf;

fn main() {
    let target = env::var("TARGET").unwrap_or_default();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Only build assembly for x86_64 targets
    if target.contains("x86_64") {
        println!("cargo:rerun-if-changed=src/device/pci_io.S");

        let asm_path = "src/device/pci_io.S";
        let obj_path = out_dir.join("pci_io.o");
        let lib_path = out_dir.join("libpci_io.a");

        // Determine output format based on target
        let obj_format = if target.contains("windows") || target.contains("uefi") {
            "win64"  // PE/COFF format for UEFI
        } else {
            "elf64"
        };

        // Assemble with nasm
        let nasm_status = Command::new("nasm")
            .args([
                "-f", obj_format,
                "-o", obj_path.to_str().unwrap(),
                asm_path,
            ])
            .status()
            .expect("Failed to run nasm. Is nasm installed?");

        if !nasm_status.success() {
            panic!("nasm failed to assemble pci_io.S");
        }

        // Create static library with ar
        let ar_status = Command::new("ar")
            .args([
                "crs",
                lib_path.to_str().unwrap(),
                obj_path.to_str().unwrap(),
            ])
            .status()
            .expect("Failed to run ar. Is binutils installed?");

        if !ar_status.success() {
            panic!("ar failed to create libpci_io.a");
        }

        // Tell cargo to link the library
        println!("cargo:rustc-link-search=native={}", out_dir.display());
        println!("cargo:rustc-link-lib=static=pci_io");
    }
}
