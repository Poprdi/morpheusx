//! Boot tests (El Torito)

mod common;

use common::MemoryBlockDevice;
use iso9660::error::Iso9660Error;
use iso9660::{find_boot_image, mount};

fn create_bootable_iso() -> MemoryBlockDevice {
    let mut device = MemoryBlockDevice::create_minimal_iso();

    // Minimal ISO (from common) has:
    // 16: PVD
    // 17: Terminator
    // 18: Root Dir

    // We want to insert Boot Record at 17, pushing Terminator to 18?
    // MemoryBlockDevice has fixed data vector. We can just overwrite 17 with Boot,
    // and move Terminator to 18?
    // But Root is at 18. Move Root to 19?

    // Easiest: Overwrite 17 with Boot Record.
    // Overwrite 18 with Terminator.
    // Move Root to 19.
    // Update PVD to point to Root at 19.

    let boot_sector = 17;
    let term_sector = 18;
    let root_sector = 19;
    let catalog_sector = 20;

    // Update Root record in PVD (Sector 16, offset 156)
    // Update Extent Location (bytes 158-166) to 19
    let pvd_offset = 16 * 2048;
    device.data[pvd_offset + 158..pvd_offset + 162]
        .copy_from_slice(&(root_sector as u32).to_le_bytes());
    device.data[pvd_offset + 162..pvd_offset + 166]
        .copy_from_slice(&(root_sector as u32).to_be_bytes()); // BE

    // 1. Create Boot Record Volume Descriptor at 17
    let offset = boot_sector * 2048;
    // Clear sector first (was Terminator)
    device.data[offset..offset + 2048].fill(0);

    device.data[offset] = 0; // Type 0 = Boot Record
    device.data[offset + 1..offset + 6].copy_from_slice(b"CD001");
    device.data[offset + 6] = 1; // Version
    device.data[offset + 7..offset + 39]
        .copy_from_slice(b"EL TORITO SPECIFICATION\0\0\0\0\0\0\0\0\0");
    device.data[offset + 71..offset + 75].copy_from_slice(&(catalog_sector as u32).to_le_bytes()); // Catalog LBA

    // 2. Terminator at 18 (was Root)
    let term_offset = term_sector * 2048;
    device.data[term_offset..term_offset + 2048].fill(0);
    device.data[term_offset] = 255;
    device.data[term_offset + 1..term_offset + 6].copy_from_slice(b"CD001");
    device.data[term_offset + 6] = 1;

    // 3. Move Root Directory to 19
    // It was at 18. Copy 18 to 19.
    // But we just overwrote 18 with Terminator?
    // We should copy 18 to 19 BEFORE overwriting 18.
    // Or just recreate it. create_minimal_iso builds it simple.
    // "." and ".." entries need to point to 19 now.

    let root_offset = root_sector * 2048;
    device.data[root_offset] = 34; // Length
    device.data[root_offset + 2..root_offset + 6]
        .copy_from_slice(&(root_sector as u32).to_le_bytes());
    device.data[root_offset + 6..root_offset + 10]
        .copy_from_slice(&(root_sector as u32).to_be_bytes()); // BE
                                                               // ... data length etc same (2048)
    device.data[root_offset + 10..root_offset + 14].copy_from_slice(&2048u32.to_le_bytes());
    device.data[root_offset + 14..root_offset + 18].copy_from_slice(&2048u32.to_be_bytes());
    device.data[root_offset + 25] = 0x02; // Directory
    device.data[root_offset + 32] = 1; // Name len
    device.data[root_offset + 33] = 0x00; // "."

    // ".."
    let parent_offset = root_offset + 34;
    device.data[parent_offset] = 34;
    device.data[parent_offset + 2..parent_offset + 6]
        .copy_from_slice(&(root_sector as u32).to_le_bytes());
    device.data[parent_offset + 6..parent_offset + 10]
        .copy_from_slice(&(root_sector as u32).to_be_bytes()); // BE
    device.data[parent_offset + 10..parent_offset + 14].copy_from_slice(&2048u32.to_le_bytes());
    device.data[parent_offset + 14..parent_offset + 18].copy_from_slice(&2048u32.to_be_bytes());
    device.data[parent_offset + 25] = 0x02;
    device.data[parent_offset + 32] = 1;
    device.data[parent_offset + 33] = 0x01; // ".."

    // 4. Create Boot Catalog at sector 20 (Same as before)
    let cat_offset = catalog_sector * 2048;

    // Validation Entry (0x01, platform, reserved, ID, checksum, 0x55, 0xAA)
    device.data[cat_offset] = 0x01; // Header ID
    device.data[cat_offset + 1] = 0xEF; // Platform ID (EFI = 0xEF)
                                        // ID string...
    device.data[cat_offset + 4..cat_offset + 28].copy_from_slice(b"Morpheus Details        ");
    device.data[cat_offset + 30] = 0x55;
    device.data[cat_offset + 31] = 0xAA;

    // Calculate checksum
    let mut sum: u16 = 0;
    // Sum all words (first 32 bytes)
    for i in (0..32).step_by(2) {
        let word =
            u16::from_le_bytes([device.data[cat_offset + i], device.data[cat_offset + i + 1]]);
        sum = sum.wrapping_add(word);
    }
    let checksum = 0u16.wrapping_sub(sum);
    device.data[cat_offset + 28..cat_offset + 30].copy_from_slice(&checksum.to_le_bytes());

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

    assert!(
        volume.boot_catalog_lba.is_some(),
        "Should find boot catalog LBA"
    );

    let boot_image = find_boot_image(&mut device, &volume).expect("Should find boot image");

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
