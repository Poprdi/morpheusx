# Network Stack Testing Guide

## Overview

This guide explains how to test the MorpheusX post-ExitBootServices network stack.

## Architecture

The network stack operates in two phases:

### Phase 1: UEFI Pre-Boot (existing)
- Uses UEFI protocols (SimpleNetwork, PCI I/O)
- Probes hardware, calibrates TSC, allocates DMA regions
- Populates `BootHandoff` structure
- Calls `ExitBootServices()`

### Phase 2: Bare-Metal Post-EBS (new)
- All UEFI services unavailable
- Uses raw ASM for hardware access
- VirtIO-net for networking
- VirtIO-blk for storage
- smoltcp for TCP/IP stack

## Test Configuration

### QEMU Network Setup
```
Guest IP:      10.0.2.15 (DHCP from QEMU user networking)
Gateway:       10.0.2.2 (host)
DNS:           10.0.2.3
HTTP Server:   http://10.0.2.2:8000/
```

### Required VirtIO Devices
- `virtio-net-pci` at PCI address `00:03.0`
- `virtio-blk-pci` at PCI address `00:04.0` (ESP)
- `virtio-blk-pci` at PCI address `00:05.0` (target disk)

## Running the Test

```bash
cd testing/
./test-network.sh
```

This will:
1. Create a test ISO image
2. Start a local HTTP server
3. Boot QEMU with VirtIO devices
4. The bootloader should:
   - Initialize network stack
   - Get IP via DHCP
   - Download the ISO
   - Write it to the target VirtIO-blk disk

## Manual Testing

### Start HTTP Server
```bash
python3 -m http.server 8000
```

### Create Test ISO
```bash
dd if=/dev/zero of=test.iso bs=1M count=50
```

### Run QEMU
```bash
qemu-system-x86_64 \
    -bios /usr/share/OVMF/x64/OVMF_CODE.4m.fd \
    -drive format=raw,file=esp.img \
    -device virtio-net-pci,netdev=net0 \
    -netdev user,id=net0 \
    -device virtio-blk-pci,drive=target \
    -drive format=raw,file=target.img,if=none,id=target \
    -serial mon:stdio \
    -m 4G
```

## Expected Boot Sequence

1. UEFI boot from ESP
2. MorpheusX logo displayed
3. "Initializing network stack..." 
4. "Network stack ready (IP: 10.0.2.15)"
5. "Downloading ISO from http://10.0.2.2:8000/test.iso..."
6. Progress updates
7. "Download complete. Writing to disk..."
8. "ISO written successfully"

## Debugging

### Serial Console
All status messages are output via UEFI console (pre-EBS) and serial (post-EBS).

### QEMU Monitor
Press `Ctrl+A c` to access QEMU monitor:
```
(qemu) info pci
(qemu) info network
```

### Memory Dump
```
(qemu) xp/8x 0x1000000
```

## State Machines

The download process uses these state machines:

```
IsoDownloadState:
  Init → WaitingForNetwork → Downloading → WritingToDisk → Verifying → Done
         ↓                    ↓             ↓              ↓
         Failed              Failed        Failed         Failed

DhcpState:
  Init → Discovering → Bound
         ↓
         Failed/Timeout

HttpDownloadState:
  Init → Resolving → Connecting → SendingRequest → ReceivingHeaders → ReceivingBody → Done
         ↓           ↓             ↓                ↓                  ↓
         Failed      Failed        Failed           Failed             Failed

DiskWriterState:
  Init → Ready → Writing → Flushing → Done
                 ↓         ↓
                 Failed    Failed
```

## API Entry Points

### Post-EBS Entry
```rust
// Called after ExitBootServices with BootHandoff pointer
morpheus_network::boot::init::post_ebs_init(handoff: &'static BootHandoff)
    -> Result<InitResult, InitError>
```

### Main Loop
```rust
// Never returns - processes network events forever
morpheus_network::mainloop::run(
    device: &mut impl NetworkDevice,
    iface: &mut Interface,
    sockets: &mut SocketSet,
    app: &mut IsoDownloadState,
)
```

### Download Orchestration
```rust
// Higher-level orchestrator for ISO download + disk write
morpheus_network::transfer::PersistenceOrchestrator::step(
    now_tsc: u64,
    timeout_config: &TimeoutConfig,
) -> OrchestratorResult
```

## Files

```
network/
├── asm/                    # Assembly drivers
│   ├── core/               # TSC, barriers, MMIO
│   ├── drivers/virtio/     # VirtIO-net, VirtIO-blk
│   └── pci/                # PCI config access
├── src/
│   ├── boot/               # Post-EBS initialization
│   │   ├── handoff.rs      # BootHandoff structure
│   │   └── init.rs         # post_ebs_init()
│   ├── driver/             # Rust driver orchestration
│   │   ├── virtio/         # VirtIO-net
│   │   └── virtio_blk.rs   # VirtIO-blk
│   ├── state/              # State machines
│   │   ├── dhcp.rs         # DHCP
│   │   ├── tcp.rs          # TCP connection
│   │   ├── http.rs         # HTTP download
│   │   ├── download.rs     # ISO download orchestration
│   │   └── disk_writer.rs  # Disk writing
│   ├── transfer/           # End-to-end orchestration
│   │   └── orchestrator.rs # PersistenceOrchestrator
│   └── mainloop/           # 5-phase poll loop
```

## Troubleshooting

### "Network initialization FAILED"
- Check QEMU is using `-device virtio-net-pci`
- Check PCI enumeration logs
- Verify DMA memory is properly allocated

### "DHCP timeout"
- QEMU user networking should provide DHCP automatically
- Check network adapter is connected

### "HTTP download failed"
- Verify HTTP server is running on host
- Check URL format (must use IP, not hostname for QEMU user networking)

### "Disk write failed"
- Verify VirtIO-blk device present
- Check target disk is large enough

## Integration with Bootloader

The bootloader currently uses the pre-ExitBootServices network initialization
in `morpheus_network::init::NetworkInit::initialize()`. 

For post-EBS operation, the bootloader would:

1. Call pre-EBS init to probe hardware and allocate DMA
2. Store BootHandoff at known location
3. Call ExitBootServices()
4. Switch to pre-allocated stack
5. Call `post_ebs_init(handoff)`
6. Enter main loop

This is not yet integrated - the current implementation uses pre-EBS
networking only.
