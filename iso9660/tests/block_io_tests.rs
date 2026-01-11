//! Block I/O implementation tests

mod common;

use common::MemoryBlockDevice;
use gpt_disk_io::BlockIo;
use gpt_disk_types::Lba;

#[test]
fn test_memory_block_device_creation() {
    let data = vec![0u8; 10 * 2048];
    let mut device = MemoryBlockDevice::new(data);

    // BlockSize might not have to_u32 in all versions, checking if u32::from works
    // Or check just the numeric value if we can unwrap it
    // The previous error suggested to_u32, so let's stick with that.
    assert_eq!(device.block_size().to_u32(), 2048);
    assert_eq!(device.num_blocks().unwrap(), 10);
}

#[test]
fn test_read_single_block() {
    let mut data = vec![0u8; 10 * 2048];
    // Write pattern to block 3
    for i in 0..2048 {
        data[3 * 2048 + i] = (i % 256) as u8;
    }

    let mut device = MemoryBlockDevice::new(data);
    let mut buffer = [0u8; 2048];

    device
        .read_blocks(Lba(3), &mut buffer)
        .expect("read should succeed");

    // Verify pattern
    for i in 0..2048 {
        assert_eq!(buffer[i], (i % 256) as u8);
    }
}

#[test]
fn test_read_multiple_blocks() {
    let mut data = vec![0u8; 10 * 2048];
    // Write patterns to blocks 2, 3, 4
    for block in 2..5 {
        for i in 0..2048 {
            data[block * 2048 + i] = block as u8;
        }
    }

    let mut device = MemoryBlockDevice::new(data);
    let mut buffer = vec![0u8; 3 * 2048];

    device
        .read_blocks(Lba(2), &mut buffer)
        .expect("read should succeed");

    // Verify each block
    for block in 0..3 {
        for i in 0..2048 {
            assert_eq!(buffer[block * 2048 + i], (block + 2) as u8);
        }
    }
}

#[test]
fn test_read_out_of_bounds() {
    let data = vec![0u8; 10 * 2048];
    let mut device = MemoryBlockDevice::new(data);
    let mut buffer = [0u8; 2048];

    // Try to read beyond device
    let result = device.read_blocks(Lba(10), &mut buffer);
    assert!(result.is_err(), "Should fail reading beyond device");
}

#[test]
fn test_write_and_read_back() {
    let data = vec![0u8; 10 * 2048];
    let mut device = MemoryBlockDevice::new(data);

    // Write pattern
    let mut write_buffer = [0u8; 2048];
    for i in 0..2048 {
        write_buffer[i] = (i % 256) as u8;
    }

    device
        .write_blocks(Lba(5), &write_buffer)
        .expect("write should succeed");

    // Read back
    let mut read_buffer = [0u8; 2048];
    device
        .read_blocks(Lba(5), &mut read_buffer)
        .expect("read should succeed");

    assert_eq!(write_buffer, read_buffer);
}

#[test]
fn test_partial_block_read() {
    let mut data = vec![0u8; 10 * 2048];
    // Write pattern
    for i in 0..2048 {
        data[2 * 2048 + i] = (i % 256) as u8;
    }

    let mut device = MemoryBlockDevice::new(data);

    // Read only 512 bytes (partial block)
    let mut buffer = [0u8; 512];
    device
        .read_blocks(Lba(2), &mut buffer)
        .expect("read should succeed");

    // Verify we got the first 512 bytes
    for i in 0..512 {
        assert_eq!(buffer[i], (i % 256) as u8);
    }
}

#[test]
fn test_unaligned_buffer_sizes() {
    let data = vec![0u8; 10 * 2048];
    let mut device = MemoryBlockDevice::new(data);

    // Test various buffer sizes
    let sizes = vec![1, 512, 1024, 2047, 2048, 2049, 4096, 8192];

    for size in sizes {
        let mut buffer = vec![0u8; size];
        let result = device.read_blocks(Lba(0), &mut buffer);

        if size <= 10 * 2048 {
            assert!(result.is_ok(), "Read of {} bytes should succeed", size);
        }
    }
}
