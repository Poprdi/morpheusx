//! File reading tests

mod common;

#[allow(unused_imports)]
use common::{MemoryBlockDevice, IsoBuilder};
#[allow(unused_imports)]
use iso9660::{mount, find_file, read_file};

#[test]
fn test_read_file_content() {
    let mut builder = IsoBuilder::new();
    builder.add_file("TEST.TXT", b"Hello ISO9660");
    let mut device = builder.build();
    
    let volume = mount(&mut device, 0).expect("mount");
    let file = find_file(&mut device, &volume, "/TEST.TXT").expect("find");
    
    let mut content = vec![0u8; file.size as usize];
    read_file(&mut device, &file, &mut content).expect("read");
    
    assert_eq!(content, b"Hello ISO9660");
}

#[test]
fn test_read_file_crossing_sectors() {
    let mut builder = IsoBuilder::new();
    // Create content that spans 2.5 sectors (2048 * 2 + 1024 = 5120 bytes)
    let mut expected_content = Vec::new();
    for i in 0..5120 {
        expected_content.push((i % 256) as u8);
    }
    
    builder.add_file("LARGE.DAT", &expected_content);
    let mut device = builder.build();
    
    let volume = mount(&mut device, 0).expect("mount");
    let file = find_file(&mut device, &volume, "/LARGE.DAT").expect("find");
    
    assert_eq!(file.size, 5120);
    
    let mut content = vec![0u8; file.size as usize];
    read_file(&mut device, &file, &mut content).expect("read");
    
    assert_eq!(content, expected_content);
}

#[test]
fn test_read_partial_last_sector() {
    let mut builder = IsoBuilder::new();
    // 2050 bytes = 1 sector + 2 bytes
    let mut expected_content = vec![0xAA; 2050];
    
    builder.add_file("PARTIAL.DAT", &expected_content);
    let mut device = builder.build();
    
    let volume = mount(&mut device, 0).expect("mount");
    let file = find_file(&mut device, &volume, "/PARTIAL.DAT").expect("find");
    
    let mut content = vec![0u8; file.size as usize];
    read_file(&mut device, &file, &mut content).expect("read partial");
    
    assert_eq!(content, expected_content);
}
