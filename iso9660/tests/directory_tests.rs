//! Directory navigation and file finding tests

mod common;

use common::MemoryBlockDevice;
use iso9660::error::Iso9660Error;
use iso9660::{find_file, mount};

#[test]
fn test_find_root_directory() {
    let mut device = MemoryBlockDevice::create_minimal_iso();
    let volume = mount(&mut device, 0).expect("mount should succeed");

    // Root directory is at the volume's root_extent_lba
    assert_eq!(volume.root_extent_lba, 18);
}

#[test]
fn test_find_nonexistent_file() {
    let mut device = MemoryBlockDevice::create_minimal_iso();
    let volume = mount(&mut device, 0).expect("mount should succeed");

    let result = find_file(&mut device, &volume, "/nonexistent.txt");
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), Iso9660Error::NotFound);
}

#[test]
fn test_root_paths() {
    let mut device = MemoryBlockDevice::create_minimal_iso();
    let volume = mount(&mut device, 0).expect("mount should succeed");

    // These should all return the root directory
    let root_paths = vec!["", "/", "//", "/./"];

    for path in root_paths {
        let entry = find_file(&mut device, &volume, path)
            .expect(&format!("Path '{}' should resolve to root", path));

        assert_eq!(entry.extent_lba, volume.root_extent_lba);
        assert!(entry.flags.directory);
    }
}

#[test]
fn test_path_depth_limit() {
    let mut device = MemoryBlockDevice::create_minimal_iso();
    let volume = mount(&mut device, 0).expect("mount should succeed");

    // ISO9660 has max depth 8. Test deep path.
    // We construct a dummy deep path
    let mut deep_path = String::new();
    for _ in 0..10 {
        deep_path.push_str("/level");
    }

    let result = find_file(&mut device, &volume, &deep_path);
    // Since levels don't exist, it might fail with NotFound before PathTooLong
    // But find_file checks depth upfront: `if components.len() > MAX_DIRECTORY_DEPTH`

    assert_eq!(result.unwrap_err(), Iso9660Error::PathTooLong);
}

#[test]
fn test_case_sensitivity() {
    let mut device = MemoryBlockDevice::create_minimal_iso();
    let volume = mount(&mut device, 0).expect("mount should succeed");

    // ISO9660 is case-insensitive by spec
    // If we add a file "TEST.TXT", both "/test.txt" and "/TEST.TXT" should find it
    // TODO: Add actual file and test
}
