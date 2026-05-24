//! Boot tests (El Torito)

mod common;

use common::MemoryBlockDevice;
use iso9660::error::Iso9660Error;
use iso9660::{find_boot_image, mount};

fn create_bootable_iso() -> MemoryBlockDevice {
    // Layout: 16=PVD, 17=Boot Record, 18=Terminator, 19=Root, 20=Catalog, 21=Image.
    // The minimal ISO has 16=PVD, 17=Terminator, 18=Root; we overwrite and
    // repoint the root extent so the new layout is consistent.
    let mut device = MemoryBlockDevice::create_minimal_iso();

    let boot_sector = 17;
    let term_sector = 18;
    let root_sector = 19;
    let catalog_sector = 20;

    // Repoint PVD root directory record (PVD offset 156, both-endian LBA at +2/+6).
    let pvd_offset = 16 * 2048;
    device.data[pvd_offset + 158..pvd_offset + 162]
        .copy_from_slice(&(root_sector as u32).to_le_bytes());
    device.data[pvd_offset + 162..pvd_offset + 166]
        .copy_from_slice(&(root_sector as u32).to_be_bytes());

    // Boot Record VD.
    let offset = boot_sector * 2048;
    device.data[offset..offset + 2048].fill(0);
    device.data[offset] = 0;
    device.data[offset + 1..offset + 6].copy_from_slice(b"CD001");
    device.data[offset + 6] = 1;
    device.data[offset + 7..offset + 39]
        .copy_from_slice(b"EL TORITO SPECIFICATION\0\0\0\0\0\0\0\0\0");
    device.data[offset + 71..offset + 75].copy_from_slice(&(catalog_sector as u32).to_le_bytes());

    // Set terminator.
    let term_offset = term_sector * 2048;
    device.data[term_offset..term_offset + 2048].fill(0);
    device.data[term_offset] = 255;
    device.data[term_offset + 1..term_offset + 6].copy_from_slice(b"CD001");
    device.data[term_offset + 6] = 1;

    // Root directory: "." and ".." both pointing at root_sector.
    let root_offset = root_sector * 2048;
    device.data[root_offset] = 34;
    device.data[root_offset + 2..root_offset + 6]
        .copy_from_slice(&(root_sector as u32).to_le_bytes());
    device.data[root_offset + 6..root_offset + 10]
        .copy_from_slice(&(root_sector as u32).to_be_bytes());
    device.data[root_offset + 10..root_offset + 14].copy_from_slice(&2048u32.to_le_bytes());
    device.data[root_offset + 14..root_offset + 18].copy_from_slice(&2048u32.to_be_bytes());
    device.data[root_offset + 25] = 0x02;
    device.data[root_offset + 32] = 1;
    device.data[root_offset + 33] = 0x00;

    let parent_offset = root_offset + 34;
    device.data[parent_offset] = 34;
    device.data[parent_offset + 2..parent_offset + 6]
        .copy_from_slice(&(root_sector as u32).to_le_bytes());
    device.data[parent_offset + 6..parent_offset + 10]
        .copy_from_slice(&(root_sector as u32).to_be_bytes());
    device.data[parent_offset + 10..parent_offset + 14].copy_from_slice(&2048u32.to_le_bytes());
    device.data[parent_offset + 14..parent_offset + 18].copy_from_slice(&2048u32.to_be_bytes());
    device.data[parent_offset + 25] = 0x02;
    device.data[parent_offset + 32] = 1;
    device.data[parent_offset + 33] = 0x01;

    // Boot catalog validation entry.
    let cat_offset = catalog_sector * 2048;
    device.data[cat_offset] = 0x01;
    device.data[cat_offset + 1] = 0xEF;
    device.data[cat_offset + 4..cat_offset + 28].copy_from_slice(b"Morpheus Details        ");
    device.data[cat_offset + 30] = 0x55;
    device.data[cat_offset + 31] = 0xAA;

    // Validation entry must sum to zero across its sixteen 16-bit words.
    let mut sum: u16 = 0;
    for i in (0..32).step_by(2) {
        let word =
            u16::from_le_bytes([device.data[cat_offset + i], device.data[cat_offset + i + 1]]);
        sum = sum.wrapping_add(word);
    }
    let checksum = 0u16.wrapping_sub(sum);
    device.data[cat_offset + 28..cat_offset + 30].copy_from_slice(&checksum.to_le_bytes());

    // Initial/default boot entry pointing at sector 21.
    let entry_offset = cat_offset + 32;
    device.data[entry_offset] = 0x88;
    device.data[entry_offset + 1] = 0;
    device.data[entry_offset + 6] = 4;
    device.data[entry_offset + 8..entry_offset + 12].copy_from_slice(&(21u32).to_le_bytes());

    // Boot image with an x86 short JMP and a 0xAA55 signature.
    let image_offset = 21 * 2048;
    device.data[image_offset] = 0xEB;
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
