//! Integration tests with real ISO images

mod common;

use common::MemoryBlockDevice;
use iso9660::{find_file, mount, read_file};
use std::path::Path;

/// Test with a real Tails ISO if available
#[test]
#[ignore] // Only run when explicitly requested
fn test_real_tails_iso() {
    let iso_path = "../testing/esp/.iso/tails-amd64-7.3.1.iso";

    if !Path::new(iso_path).exists() {
        eprintln!("Skipping test: Tails ISO not found at {}", iso_path);
        return;
    }

    let mut device = MemoryBlockDevice::from_file(iso_path).expect("Should load ISO file");

    println!(
        "ISO size: {} bytes ({} sectors)",
        device.data.len(),
        device.data.len() / 2048
    );

    // 1. Mount the volume
    let volume = mount(&mut device, 0).expect("Should mount Tails ISO");

    println!(
        "Volume ID: {:?}",
        String::from_utf8_lossy(&volume.volume_id)
    );
    println!("Block size: {}", volume.logical_block_size);
    println!("Volume size: {} blocks", volume.volume_space_size);
    println!(
        "Root extent: LBA {}, length {}",
        volume.root_extent_lba, volume.root_extent_len
    );

    // 2. Find kernel (Tails uses /live/vmlinuz)
    let kernel_paths = ["/live/vmlinuz", "/casper/vmlinuz"];

    let mut kernel_found = false;
    for path in &kernel_paths {
        if let Ok(kernel) = find_file(&mut device, &volume, path) {
            println!("Found kernel at {}: {} bytes", path, kernel.size);
            kernel_found = true;

            // 3. Try to read kernel (first 4KB to verify)
            let mut buffer = vec![0u8; 4096.min(kernel.size as usize)];
            read_file(&mut device, &kernel, &mut buffer).expect("Should read kernel");

            // Check for Linux kernel magic
            // ELF: 0x7F 'E' 'L' 'F'
            // or bzImage: 'M' 'Z' (for x86 boot sector)
            println!(
                "Kernel header: {:02X} {:02X} {:02X} {:02X}",
                buffer[0], buffer[1], buffer[2], buffer[3]
            );

            break;
        }
    }

    assert!(kernel_found, "Should find kernel in Tails ISO");

    // 4. Find initrd
    let initrd_paths = ["/live/initrd.img", "/casper/initrd"];

    for path in &initrd_paths {
        if let Ok(initrd) = find_file(&mut device, &volume, path) {
            println!("Found initrd at {}: {} bytes", path, initrd.size);
            break;
        }
    }
}

/// Test with any ISO in test-data directory
#[test]
#[ignore]
fn test_custom_test_iso() {
    let iso_path = "test-data/test.iso";

    if !Path::new(iso_path).exists() {
        eprintln!("Skipping test: No test ISO at {}", iso_path);
        eprintln!("Create one with: genisoimage -o test-data/test.iso -r test-data/files/");
        return;
    }

    let mut device = MemoryBlockDevice::from_file(iso_path).expect("Should load test ISO");

    let volume = mount(&mut device, 0).expect("Should mount test ISO");

    println!(
        "Mounted test ISO: {:?}",
        String::from_utf8_lossy(&volume.volume_id)
    );
}

/// Create a minimal test ISO using genisoimage if available
#[test]
#[ignore]
fn create_test_iso() {
    use std::fs;
    use std::process::Command;

    let test_dir = "test-data/source";
    let iso_file = "test-data/minimal.iso";

    // Create test files
    fs::create_dir_all(test_dir).expect("Should create test directory");
    fs::write(format!("{}/hello.txt", test_dir), b"Hello, World!").expect("Should write test file");
    fs::write(format!("{}/test.dat", test_dir), &[0u8; 8192]).expect("Should write test file");

    // Try to create ISO
    let result = Command::new("genisoimage")
        .args(&[
            "-o", iso_file, "-r", // Rock Ridge extensions
            "-J", // Joliet extensions
            "-V", "TEST", // Volume ID
            test_dir,
        ])
        .output();

    match result {
        Ok(output) if output.status.success() => {
            println!("Created test ISO at {}", iso_file);

            // Verify we can mount it
            let mut device =
                MemoryBlockDevice::from_file(iso_file).expect("Should load created ISO");
            let volume = mount(&mut device, 0).expect("Should mount created ISO");

            println!("Volume: {:?}", String::from_utf8_lossy(&volume.volume_id));

            // Try to find our test file
            if let Ok(file) = find_file(&mut device, &volume, "/hello.txt") {
                println!("Found hello.txt: {} bytes", file.size);

                let mut content = vec![0u8; file.size as usize];
                read_file(&mut device, &file, &mut content).expect("Should read file");

                assert_eq!(&content, b"Hello, World!");
            }
        }
        Ok(output) => {
            eprintln!("genisoimage failed:");
            eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        }
        Err(e) => {
            eprintln!("genisoimage not available: {}", e);
            eprintln!("Install with: apt-get install genisoimage");
        }
    }
}
