//! Assembles morpheus-xhci ASM (`asm/usb/*.s`) into a single static archive
//! linked into the lib. Mirrors the nasm + ar pattern used by hwinit/network.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

// USB host-controller primitives. Previously lived in network/asm/drivers/usb/
// and were silently imported by hwinit via `extern "win64"` — the cross-crate
// link-time dependency is what motivated this crate.
const ASM_USB: &[&str] = &["asm/usb/init.s"];

fn main() {
    let target = env::var("TARGET").unwrap_or_default();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=asm/");

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
        "cargo:warning=Building morpheus-xhci ASM for: {} ({})",
        target, obj_format
    );

    let mut objects = Vec::new();

    for path in ASM_USB.iter() {
        if Path::new(path).exists() {
            println!("cargo:rerun-if-changed={}", path);
            match assemble(path, &out_dir, obj_format) {
                Ok(obj) => objects.push(obj),
                Err(e) => println!("cargo:warning=ASM failed {}: {}", path, e),
            }
        }
    }

    if objects.is_empty() {
        println!("cargo:warning=No ASM files assembled");
        return;
    }

    let lib = out_dir.join("libmorpheus_xhci_asm.a");
    let mut args: Vec<String> = vec!["crs".into(), lib.to_str().unwrap().into()];
    args.extend(objects.iter().map(|p| p.to_str().unwrap().into()));

    let out = Command::new("ar").args(&args).output().expect("ar failed");
    if !out.status.success() {
        panic!("ar failed: {}", String::from_utf8_lossy(&out.stderr));
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=morpheus_xhci_asm");
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
