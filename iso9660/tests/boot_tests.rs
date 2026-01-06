//! Boot tests (El Torito)

mod common;

use common::MemoryBlockDevice;
use iso9660::{mount, find_boot_image};
use iso9660::error::Iso9660Error;

fn create_bootable_iso() -> MemoryBlockDevice {
    let mut device = MemoryBlockDevice::create_minimal_iso();
    
    // Add Boot Record at sector 19 (after Root)
    // Actually create_minimal_iso puts Root at 18.
    // Let's put Boot Record at 19.
    let boot_record_sector = 19;
    let catalog_sector = 20;
    
    // 1. Create Boot Record Volume Descriptor
    let offset = boot_record_sector * 2048;
    device.data[offset] = 0; // Type 0 = Boot Record
    device.data[offset + 1..offset + 6].copy_from_slice(b"CD001");
    device.data[offset + 6] = 1; // Version
    device.data[offset + 7..offset + 39].copy_from_slice(b"EL TORITO SPECIFICATION\0\0\0\0\0\0\0\0\0");
    device.data[offset + 71..offset + 75].copy_from_slice(&(catalog_sector as u32).to_le_bytes()); // Catalog LBA
    
    // 2. Create Boot Catalog at sector 20
    let cat_offset = catalog_sector * 2048;
    
    // Validation Entry (0x01, platform, reserved, ID, checksum, 0x55, 0xAA)
    device.data[cat_offset] = 0x01; // Header ID
    device.data[cat_offset + 1] = 0xEF; // Platform ID (EFI = 0xEF)
    // ID string...
    device.data[cat_offset + 4..cat_offset + 28].copy_from_slice(b"Morpheus Details        ");
    // Checksum... (simplified, might need real calculation if crate validates strict)
    // Key bytes
    device.data[cat_offset + 30] = 0x55;
    device.data[cat_offset + 31] = 0xAA;
    
    // Initial/Default Entry (bootable=0x88, media=0, load_seg=0, sys_type=0, count=4, lba=21)
    let entry_offset = cat_offset + 32;
    device.data[entry_offset] = 0x88; // Bootable indicator
    device.data[entry_offset + 1] = 0; // No Emulation
    device.data[entry_offset + 6] = 4; // Sector count (2 sectors) -> wait, it's u16
    device.data[entry_offset + 8..entry_offset + 12].copy_from_slice(&(21u32).to_le_bytes()); // Load RBA = 21
    
    // 3. Boot Image content at sector 21
    let image_offset = 21 * 2048;
    device.data[image_offset] = 0xEB; // JMP instruction (x86 boot)
    device.data[image_offset + 1] = 0x3C;
    device.data[image_offset + 510] = 0x55;
    device.data[image_offset + 511] = 0xAA;
    
    device
}

#[test]
fn test_find_boot_image() {
    let mut device = create_bootable_iso();
    let volume = mount(&mut device, 0).expect("mount success");
    
    assert!(volume.boot_catalog_lba.is_some(), "Should find boot catalog LBA");
    
    let boot_image = find_boot_image(&mut device, &volume)
        .expect("Should find boot image");
        
    assert!(boot_image.bootable);
    assert_eq!(boot_image.load_rba, 21);
}

#[test]
fn test_no_boot_catalog() {
    let mut device = MemoryBlockDevice::create_minimal_iso();
    let volume = mount(&mut device, 0).expect("mount success");
    
    // Minimal ISO has no boot catalog
    let result = find_boot_image(&mut device, &volume);
    assert_eq!(result.err(), Some(Iso9660Error::NoBootCatalog));
}

#[test]
fn test_invalid_boot_catalog_signature() {
    let mut device = create_bootable_iso();
    let catalog_sector = 20;
    
    // Corrupt validation signature
    device.data[catalog_sector * 2048 + 30] = 0x00;
    
    let volume = mount(&mut device, 0).expect("mount success");
    let result = find_boot_image(&mut device, &volume);
    assert_eq!(result.err(), Some(Iso9660Error::InvalidBootCatalog));
}
