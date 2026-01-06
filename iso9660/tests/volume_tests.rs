//! Volume descriptor parsing tests

mod common;

use common::MemoryBlockDevice;
use iso9660::volume::mount;
use iso9660::error::Iso9660Error;

#[test]
fn test_mount_minimal_iso() {
    let mut device = MemoryBlockDevice::create_minimal_iso();
    
    let result = mount(&mut device, 0);
    assert!(result.is_ok(), "Should successfully mount minimal ISO");
    
    let volume = result.unwrap();
    assert_eq!(volume.logical_block_size, 2048);
    assert_eq!(volume.volume_space_size, 64);
    assert_eq!(volume.root_extent_lba, 18);
}

#[test]
fn test_mount_invalid_signature() {
    let mut device = MemoryBlockDevice::new(vec![0u8; 64 * 2048]);
    // No valid volume descriptor - should fail
    
    let result = mount(&mut device, 0);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), Iso9660Error::InvalidSignature);
}

#[test]
fn test_mount_empty_device() {
    let mut device = MemoryBlockDevice::new(vec![0u8; 10 * 2048]);
    // Device too small for proper volume descriptors
    
    let result = mount(&mut device, 0);
    assert!(result.is_err());
}

#[test]
fn test_mount_with_offset() {
    let mut device = MemoryBlockDevice::create_minimal_iso();
    
    // Should work at offset 0
    let result = mount(&mut device, 0);
    assert!(result.is_ok());
}

#[test]
fn test_volume_info_fields() {
    let mut device = MemoryBlockDevice::create_minimal_iso();
    let volume = mount(&mut device, 0).expect("mount should succeed");
    
    // Check expected values from minimal ISO
    assert_eq!(volume.logical_block_size, 2048, "Block size should be 2048");
    assert_eq!(volume.volume_space_size, 64, "Volume should have 64 sectors");
    assert_eq!(volume.root_extent_lba, 18, "Root should be at sector 18");
    assert_eq!(volume.root_extent_len, 2048, "Root extent should be 2048 bytes");
    assert!(!volume.has_joliet, "Minimal ISO has no Joliet");
    assert!(!volume.has_rock_ridge, "Minimal ISO has no Rock Ridge");
}

#[test]
fn test_mount_read_only() {
    let mut device = MemoryBlockDevice::create_minimal_iso();
    
    // Mount should not modify device
    let data_before = device.data.clone();
    let _ = mount(&mut device, 0);
    assert_eq!(device.data, data_before, "Mount should not modify device");
}
