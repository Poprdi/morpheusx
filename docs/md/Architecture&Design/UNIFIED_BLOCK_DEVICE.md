# Unified Block Device Architecture

MorpheusX provides a unified block device abstraction that enables seamless operation
across both virtualized environments (QEMU with VirtIO-blk) and real hardware
(ThinkPad T450s with Intel AHCI SATA controller).

## Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                 Filesystem Layer (FAT32, ISO9660)                       │
│                    gpt_disk_io::BlockIo trait                           │
└─────────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                    UnifiedBlockIo Adapter                               │
│              Synchronous wrapper with DMA buffer                        │
│      (network/src/driver/unified_block_io.rs)                          │
└─────────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                    UnifiedBlockDevice                                   │
│                  BlockDriver trait impl                                 │
│       (network/src/device/mod.rs)                                      │
├─────────────────────────────────┬───────────────────────────────────────┤
│         VirtIO-blk              │              AHCI SATA                │
│       (QEMU, VMs)               │     (ThinkPad T450s, real HW)         │
└─────────────────────────────────┴───────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                      ASM Layer (Hardware Access)                        │
│        All MMIO, DMA, PCI via hand-written x86_64 assembly             │
│                   Microsoft x64 ABI (UEFI)                             │
└─────────────────────────────────────────────────────────────────────────┘
```

## Target Hardware

### AHCI (Real Hardware)
- **Intel Wildcat Point-LP SATA Controller**
  - Vendor: `0x8086` (Intel)
  - Device: `0x9C83`
  - Class: `0x010601` (Mass Storage → SATA → AHCI)
- Found in ThinkPad T450s and similar Haswell/Broadwell era laptops

### VirtIO-blk (Virtualization)
- **VirtIO Block Device**
  - Vendor: `0x1AF4` (Red Hat)
  - Device: `0x1001` (transitional) or `0x1042` (modern)
- QEMU `-drive file=disk.img,if=virtio`

## DMA Memory Requirements

The block device drivers require specific DMA memory allocations:

### AHCI Requirements
| Structure       | Size        | Alignment  | Description                      |
|----------------|-------------|------------|----------------------------------|
| Command List   | 1 KB        | 1 KB       | 32 × 32-byte command headers     |
| FIS Receive    | 256 bytes   | 256 bytes  | Frame Information Structure      |
| Command Tables | 8 KB        | 128 bytes  | 32 × 256-byte per slot           |
| IDENTIFY       | 512 bytes   | 2 bytes    | Device identification data       |
| I/O Buffer     | 64+ KB      | 4 KB       | Read/write transfer buffer       |

### VirtIO-blk Requirements
| Structure       | Size        | Alignment  | Description                      |
|----------------|-------------|------------|----------------------------------|
| Virtqueue      | ~16 KB      | 4 KB       | Descriptor rings                 |
| I/O Buffer     | 64+ KB      | 512 bytes  | Read/write transfer buffer       |

## Usage Examples

### 1. Probe and Create Unified Device

```rust
use morpheus_network::{
    UnifiedBlockDevice, UnifiedBlockIo, BlockDmaConfig,
    probe_unified_block_device,
};

// DMA configuration (pre-allocated aligned buffers)
let config = BlockDmaConfig {
    // AHCI structures
    cmd_list_cpu: cmd_list_ptr,
    cmd_list_phys: cmd_list_bus_addr,
    fis_cpu: fis_ptr,
    fis_phys: fis_bus_addr,
    cmd_tables_cpu: tables_ptr,
    cmd_tables_phys: tables_bus_addr,
    identify_cpu: identify_ptr,
    identify_phys: identify_bus_addr,
    // VirtIO structures  
    virtqueue_cpu: vq_ptr,
    virtqueue_phys: vq_bus_addr,
    // Common
    tsc_freq: calibrated_tsc,
};

// Probe for block device (auto-detects AHCI or VirtIO)
let mut device = unsafe { probe_unified_block_device(&config)? };

// Check what was detected
println!("Block device: {}", device.driver_type()); // "AHCI SATA" or "VirtIO-blk"
```

### 2. Create BlockIo Adapter for Filesystem Access

```rust
// Create synchronous BlockIo wrapper
let mut bio = UnifiedBlockIo::new(
    &mut device,
    io_buffer,           // DMA buffer for transfers
    io_buffer_phys,      // Physical address of buffer
    tsc_freq * 5,        // 5 second timeout
)?;

// Now use with gpt_disk_io for filesystem operations
let block_size = bio.block_size();
let total_blocks = bio.num_blocks()?;

// Read boot sector
let mut mbr = [0u8; 512];
bio.read_blocks(Lba(0), &mut mbr)?;

// Write to disk
bio.write_blocks(Lba(100), &data)?;
bio.flush()?;
```

### 3. Direct Driver Access (Advanced)

```rust
use morpheus_network::{AhciDriver, AhciConfig, BlockDriver};

// Direct AHCI access (bypassing UnifiedBlockDevice)
let config = AhciConfig {
    cmd_list_cpu: ...,
    cmd_list_phys: ...,
    // ...
};

let mut ahci = unsafe { AhciDriver::new(abar, config)? };

// Get device info
let info = ahci.info();
println!("Sectors: {}, Size: {}B", info.total_sectors, info.sector_size);

// Submit async read
ahci.submit_read(sector, buffer_phys, count, request_id)?;
ahci.notify();  // AHCI: no-op, VirtIO: kicks queue

// Poll for completion
loop {
    if let Some(completion) = ahci.poll_completion() {
        if completion.request_id == request_id {
            // Read complete
            break;
        }
    }
}
```

## ASM Layer Files

The AHCI driver uses the following assembly files in `network/asm/drivers/ahci/`:

| File          | Purpose                                        |
|---------------|------------------------------------------------|
| `regs.s`      | AHCI register definitions and constants        |
| `init.s`      | HBA reset, enable, capability reading          |
| `port.s`      | Port management (stop, start, setup, detect)   |
| `cmd.s`       | Command header and FIS building                |
| `identify.s`  | IDENTIFY DEVICE command                        |
| `io.s`        | READ/WRITE DMA EXT operations                  |

All assembly follows Microsoft x64 ABI (RCX, RDX, R8, R9, stack) for UEFI compatibility.

## Error Handling

```rust
use morpheus_network::{BlockError, UnifiedBlockIoError};

// BlockDriver errors
match device.submit_read(...) {
    Err(BlockError::InvalidSector) => /* out of bounds */,
    Err(BlockError::QueueFull) => /* retry later */,
    Err(BlockError::ReadOnly) => /* write to RO device */,
    Err(BlockError::DeviceError) => /* hardware failure */,
    Err(BlockError::Timeout) => /* operation timed out */,
    _ => {}
}

// BlockIo adapter errors  
match bio.read_blocks(...) {
    Err(UnifiedBlockIoError::Timeout) => /* I/O timeout */,
    Err(UnifiedBlockIoError::BufferAlignment) => /* buffer issue */,
    Err(UnifiedBlockIoError::DeviceNotReady) => /* link down */,
    _ => {}
}
```

## AHCI Specification Compliance

The driver implements AHCI 1.3.1 with:

- ✅ HBA reset and enable sequence (GHC.AE, GHC.HR)
- ✅ Port multiplier support detection
- ✅ Native Command Queuing (NCQ) capable (32 slots)
- ✅ 64-bit DMA addressing
- ✅ FIS-based switching
- ✅ READ DMA EXT (0x25) / WRITE DMA EXT (0x35)
- ✅ FLUSH CACHE EXT (0xEA)
- ✅ IDENTIFY DEVICE (0xEC)
- ✅ Polling-based completion (no interrupts)

## Performance Notes

1. **No interrupts** - All completion is poll-based for simplicity in bare-metal
2. **Single-command depth** - While NCQ supports 32, current impl uses one at a time
3. **64KB max transfer** - Conservative limit to avoid PRDT complexity
4. **Synchronous flush** - `flush()` blocks until cache is committed

## Future Enhancements

- [ ] NCQ multi-command queueing for parallelism
- [ ] TRIM/UNMAP support for SSDs
- [ ] Port multiplier support
- [ ] Hot-plug detection
- [ ] Power management (DIPM, HIPM)
