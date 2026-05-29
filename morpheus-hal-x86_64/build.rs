//! Assembles hal-x86_64 CPU + PCI ASM. Bare primitives live in `morpheus-x86-asm`.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const ASM_CPU: &[&str] = &[
    // PIT-driven TSC calibration; co-located with `cpu/ap_boot.rs`.
    "asm/cpu/calibrate.s",
    // Context-switch + LAPIC timer ISR shim; syscall MSR entry.
    "asm/cpu/context_switch.s",
    "asm/cpu/syscall.s",
];

const ASM_PCI: &[&str] = &[
    "asm/pci/legacy.s",
    "asm/pci/ecam.s",
    "asm/pci/bar.s",
    "asm/pci/capability.s",
    "asm/pci/virtio_cap.s",
];

const ASM_FB: &[&str] = &[
    // Differential framebuffer present, SSE2-accelerated.
    "asm/fb/present.s",
];

fn main() {
    let target = env::var("TARGET").unwrap_or_default();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    println!("cargo:rerun-if-changed=build.rs");

    // The AP trampoline is `include_bytes!`d unconditionally when the `smp`
    // feature is on, so the file must exist for EVERY target — including the
    // host target used by `cargo clippy`/`check --all-features`, which type-
    // checks the code but never runs it. Real UEFI builds assemble the flat
    // binary; other targets get an empty placeholder (=> AP_TRAMPOLINE_BIN is
    // empty => smp bring-up no-ops, matching the pre-smp-feature behavior).
    provision_trampoline(&target, &out_dir);

    if !target.contains("x86_64") {
        println!(
            "cargo:warning=Skipping ASM for non-x86_64 target: {}",
            target
        );
        return;
    }

    if !target.contains("uefi") {
        println!("cargo:warning=Skipping ASM for non-UEFI target: {}", target);
        return;
    }

    let obj_format = if target.contains("windows") || target.contains("uefi") {
        "win64"
    } else {
        "elf64"
    };

    println!(
        "cargo:warning=Building morpheus-hal-x86_64 ASM for: {} ({})",
        target, obj_format
    );

    let mut objects = Vec::new();

    for files in [ASM_CPU, ASM_PCI, ASM_FB] {
        for path in files.iter() {
            if Path::new(path).exists() {
                println!("cargo:rerun-if-changed={}", path);
                match assemble(path, &out_dir, obj_format) {
                    Ok(obj) => objects.push(obj),
                    Err(e) => println!("cargo:warning=ASM failed {}: {}", path, e),
                }
            }
        }
    }

    if objects.is_empty() {
        println!("cargo:warning=No ASM files assembled");
        return;
    }

    let lib = out_dir.join("libmorpheus_hal_x86_64_asm.a");
    let mut args: Vec<String> = vec!["crs".into(), lib.to_str().unwrap().into()];
    args.extend(objects.iter().map(|p| p.to_str().unwrap().into()));

    let out = Command::new("ar").args(&args).output().expect("ar failed");
    if !out.status.success() {
        panic!("ar failed: {}", String::from_utf8_lossy(&out.stderr));
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=morpheus_hal_x86_64_asm");
}

/// Ensure `ap_trampoline.bin` exists in OUT_DIR. Assembles the flat binary on
/// real x86_64-UEFI targets; writes an empty placeholder elsewhere so the
/// unconditional `include_bytes!` in `cpu/ap_boot.rs` always resolves.
fn provision_trampoline(target: &str, out_dir: &Path) {
    let trampoline_bin = out_dir.join("ap_trampoline.bin");
    let trampoline_src = "asm/cpu/ap_trampoline.s";

    let is_uefi_x86 = target.contains("x86_64") && target.contains("uefi");
    if is_uefi_x86 && Path::new(trampoline_src).exists() {
        println!("cargo:rerun-if-changed={}", trampoline_src);

        let out = Command::new("nasm")
            .args([
                "-f",
                "bin",
                "-o",
                trampoline_bin.to_str().unwrap(),
                trampoline_src,
            ])
            .output()
            .expect("nasm failed for AP trampoline");

        if !out.status.success() {
            panic!(
                "AP trampoline assembly failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        println!(
            "cargo:warning=Built AP trampoline ({} bytes)",
            std::fs::metadata(&trampoline_bin)
                .map(|m| m.len())
                .unwrap_or(0)
        );
    } else {
        std::fs::write(&trampoline_bin, []).expect("write empty ap_trampoline.bin placeholder");
    }
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
