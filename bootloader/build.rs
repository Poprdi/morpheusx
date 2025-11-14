// Build script to compile assembly files

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    
    // Compile 32-bit trampoline assembly
    let status = Command::new("as")
        .args(&[
            "--64",  // Assemble for x86-64
            "-o",
            out_dir.join("trampoline32.o").to_str().unwrap(),
            "src/boot/arch/x86_64/trampoline32.s",
        ])
        .status()
        .expect("Failed to assemble trampoline32.s");
    
    assert!(status.success(), "Assembly failed");
    
    // Link the object file into a static library
    let status = Command::new("ar")
        .args(&[
            "crus",
            out_dir.join("libtrampoline32.a").to_str().unwrap(),
            out_dir.join("trampoline32.o").to_str().unwrap(),
        ])
        .status()
        .expect("Failed to create static library");
    
    assert!(status.success(), "Archive creation failed");
    
    // Tell Cargo to link the library
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=trampoline32");
    println!("cargo:rerun-if-changed=src/boot/arch/x86_64/trampoline32.s");
}
