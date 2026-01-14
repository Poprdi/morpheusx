//! Build script for morpheus-network.
//!
//! Assembles all ASM files for x86_64 targets:
//! - Legacy pci_io.S (existing)
//! - New ASM layer in asm/ (core, pci, drivers, phy)

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// All ASM source files organized by category
const ASM_CORE: &[&str] = &[
    "asm/core/tsc.s",
    "asm/core/barriers.s",
    "asm/core/mmio.s",
    "asm/core/pio.s",
    "asm/core/cache.s",
    "asm/core/delay.s",
];

const ASM_PCI: &[&str] = &[
    "asm/pci/legacy.s",
    "asm/pci/ecam.s",
    "asm/pci/bar.s",
    "asm/pci/capability.s",
    "asm/pci/virtio_cap.s",
];

const ASM_VIRTIO: &[&str] = &[
    "asm/drivers/virtio/init.s",
    "asm/drivers/virtio/queue.s",
    "asm/drivers/virtio/tx.s",
    "asm/drivers/virtio/rx.s",
    "asm/drivers/virtio/notify.s",
    "asm/drivers/virtio/blk.s",
    "asm/drivers/virtio/pci_modern.s",
];

const ASM_INTEL: &[&str] = &[
    "asm/drivers/intel/init.s",
    "asm/drivers/intel/tx.s",
    "asm/drivers/intel/rx.s",
    "asm/drivers/intel/phy.s",
    "asm/drivers/intel/ulp.s", // I218/PCH LPT ULP management (CRITICAL for real hardware)
];

const ASM_AHCI: &[&str] = &[
    "asm/drivers/ahci/init.s",
    "asm/drivers/ahci/port.s",
    "asm/drivers/ahci/cmd.s",
    "asm/drivers/ahci/identify.s",
    "asm/drivers/ahci/io.s",
];

const ASM_PHY: &[&str] = &["asm/phy/mdio.s", "asm/phy/mii.s", "asm/phy/link.s"];

fn main() {
    let target = env::var("TARGET").unwrap_or_default();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Always rerun if build.rs changes
    println!("cargo:rerun-if-changed=build.rs");

    // Only build assembly for x86_64 targets
    if !target.contains("x86_64") {
        println!(
            "cargo:warning=Skipping assembly for non-x86_64 target: {}",
            target
        );
        return;
    }

    // Only build ASM for UEFI target - host builds don't need it
    // The ASM is for bare-metal post-ExitBootServices execution
    if !target.contains("uefi") {
        println!(
            "cargo:warning=Skipping assembly for non-UEFI target: {} (ASM only needed for bare-metal)",
            target
        );
        return;
    }

    // Determine output format based on target
    // UEFI uses PE/COFF format (win64)
    let obj_format = if target.contains("windows") || target.contains("uefi") {
        "win64" // PE/COFF format for UEFI
    } else {
        "elf64"
    };

    println!(
        "cargo:warning=Building ASM for target: {} (format: {})",
        target, obj_format
    );

    let mut all_objects = Vec::new();

    // =========================================================================
    // Build legacy pci_io.S (existing)
    // =========================================================================
    let legacy_asm = "src/device/pci_io.S";
    if Path::new(legacy_asm).exists() {
        println!("cargo:rerun-if-changed={}", legacy_asm);
        let obj = assemble_file(legacy_asm, &out_dir, obj_format);
        all_objects.push(obj);
    }

    // =========================================================================
    // Build new ASM layer (if feature enabled or files exist)
    // =========================================================================
    let asm_categories = [
        ("core", ASM_CORE),
        ("pci", ASM_PCI),
        ("virtio", ASM_VIRTIO),
        ("intel", ASM_INTEL),
        ("ahci", ASM_AHCI),
        ("phy", ASM_PHY),
    ];

    for (category, files) in asm_categories {
        for asm_path in files.iter() {
            if Path::new(asm_path).exists() {
                println!("cargo:rerun-if-changed={}", asm_path);
                match assemble_file_checked(asm_path, &out_dir, obj_format) {
                    Ok(obj) => {
                        all_objects.push(obj);
                    }
                    Err(e) => {
                        println!(
                            "cargo:warning=Failed to assemble {} ({}): {}",
                            asm_path, category, e
                        );
                        // Continue with other files
                    }
                }
            }
        }
    }

    // =========================================================================
    // Create static library from all objects
    // =========================================================================
    if all_objects.is_empty() {
        println!("cargo:warning=No ASM files assembled!");
        return;
    }

    let lib_path = out_dir.join("libnetwork_asm.a");

    // Use ar to create archive
    let mut ar_args = vec!["crs".to_string(), lib_path.to_str().unwrap().to_string()];
    for obj in &all_objects {
        ar_args.push(obj.to_str().unwrap().to_string());
    }

    let ar_output = Command::new("ar")
        .args(&ar_args)
        .output()
        .expect("Failed to run ar. Is binutils installed?");

    if !ar_output.status.success() {
        let stderr = String::from_utf8_lossy(&ar_output.stderr);
        panic!("ar failed to create libnetwork_asm.a: {}", stderr);
    }

    println!(
        "cargo:warning=Created static library with {} objects: {}",
        all_objects.len(),
        lib_path.display()
    );

    // Tell cargo to link the library
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=network_asm");
}

/// Assemble a single file, panic on failure
fn assemble_file(asm_path: &str, out_dir: &Path, obj_format: &str) -> PathBuf {
    assemble_file_checked(asm_path, out_dir, obj_format)
        .unwrap_or_else(|e| panic!("Failed to assemble {}: {}", asm_path, e))
}

/// Assemble a single file, return Result
fn assemble_file_checked(
    asm_path: &str,
    out_dir: &Path,
    obj_format: &str,
) -> Result<PathBuf, String> {
    let _stem = Path::new(asm_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("Invalid path: {}", asm_path))?;

    // Create unique object name to avoid collisions
    let obj_name = asm_path
        .replace('/', "_")
        .replace('\\', "_")
        .replace(".s", ".o")
        .replace(".S", ".o");

    let obj_path = out_dir.join(&obj_name);

    // Get include paths for ASM files that use external references
    let mut nasm_args = vec![
        "-f".to_string(),
        obj_format.to_string(),
        "-o".to_string(),
        obj_path.to_str().unwrap().to_string(),
    ];

    // Add include path for cross-file references
    if asm_path.contains("asm/") {
        nasm_args.push("-I".to_string());
        nasm_args.push("asm/".to_string());
    }

    nasm_args.push(asm_path.to_string());

    let nasm_output = Command::new("nasm")
        .args(&nasm_args)
        .output()
        .map_err(|e| format!("Failed to run nasm: {}", e))?;

    if !nasm_output.status.success() {
        let stderr = String::from_utf8_lossy(&nasm_output.stderr);
        return Err(format!("nasm error: {}", stderr));
    }

    Ok(obj_path)
}
