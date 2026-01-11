# MorpheusX Display Stack

Display rendering and scanout for the bare-metal post-ExitBootServices environment.

### Design Principles

1. **Hardware Acceleration**: Prioritize virtio-gpu with virgl/3D; fall back to 2D
2. **No External Deps**: Everything built on primitives
3. **Error Handling**: Comprehensive error types for display operations
4. **Drop-in Drivers**: Stable ASM boundary for vendor GPUs

## Usage

### Basic Display Init

```rust
use morpheus_display::{Display, DisplayConfig, Mode};

// Create display driver from PCI discovery
let display = Display::init_from_pci(
    handoff.display_mmio_base,
    &mut dma_region,
    DisplayConfig::default(),
)?;

// Query capabilities
let caps = display.capabilities();
if caps.has_3d {
    log::info!("Hardware 3D acceleration available");
}
```

### Upload Frame

```rust
// Create 2D resource
let texture = display.create_resource(ResourceType::Texture2D {
    width: 1920,
    height: 1080,
    format: Format::BGRA8888,
})?;

// Upload pixel data
display.upload_resource(&texture, &pixel_data)?;

// Attach to scanout and display
display.attach_scanout(0, &texture, Rect::full())?;
display.flush()?;
```

### Fencing and Synchronization

```rust
let fence = display.create_fence()?;
display.submit_with_fence(cmds, &fence)?;
fence.wait_timeout(FRAME_BUDGET)?;
```

## Implementation 

### See /docs/md/Architecture&Design/Display/

## Error Handling

All operations return `Result<T, DisplayError>`:

```rust
match display.set_mode(requested_mode) {
    Ok(()) => { /* success */ },
    Err(DisplayError::ModeNotSupported(mode)) => {
        // Fall back to different mode
    },
    Err(DisplayError::No3DSupport) => {
        // Run in degraded mode
    },
    Err(e) => {
        // Other error
    }
}
```

## Testing

Test in QEMU with virtio-gpu:
```bash
cd testing
./run-display-test.sh
```

Ensure QEMU has display configured:
```bash
-device virtio-gpu-pci,virgl=on \
-display gtk,gl=on
```

## Future: ARM64 Support

When adding ARM64 support:
- Same VirtIO-GPU driver works on ARM64
- ASM layer uses DMB instead of MFENCE
- May need arch-specific optimizations for large transfers

## References

- VirtIO Specification v1.2 (Section 5.7 - GPU Device)
- Virgl Protocol Documentation
- UEFI Specification 2.10, Section 12.9 (Graphics Output Protocol)
