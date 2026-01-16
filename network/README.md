# MorpheusX Network Stack

Bare-metal HTTP client for post-ExitBootServices execution.

## Architecture (with hwinit split)

```
┌──────────────────────────────────────────────────────────────┐
│                    Boot Sequence                             │
│  UEFI → ExitBootServices → hwinit → [driver init] → network  │
└──────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌──────────────────────────────────────────────────────────────┐
│                    Entry Point                               │
│  mainloop::download_with_config(&mut driver, config, ...)    │
└──────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌──────────────────────────────────────────────────────────────┐
│                    State Machine                             │
│  Init → GptPrep → LinkWait → DHCP → DNS → Connect → HTTP     │
└──────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌──────────────────────────────────────────────────────────────┐
│                    NetworkDriver Trait                       │
│  transmit(), receive(), mac_address(), link_up()             │
└──────────────────────────────────────────────────────────────┘
                             │
              ┌──────────────┴──────────────┐
              ▼                             ▼
┌─────────────────────────┐   ┌─────────────────────────┐
│     VirtioNetDriver     │   │     E1000eDriver        │
│     (QEMU, KVM)         │   │     (Real Hardware)     │
└─────────────────────────┘   └─────────────────────────┘
              │                             │
              └──────────────┬──────────────┘
                             ▼
┌──────────────────────────────────────────────────────────────┐
│                    ASM Layer (network/asm/)                  │
│  MMIO read/write, TSC, barriers, cache ops                   │
└──────────────────────────────────────────────────────────────┘
```

## Usage (NEW - with hwinit)

```rust
use morpheus_hwinit::{platform_init, PlatformConfig, NetDeviceType};
use morpheus_network::driver::virtio::{VirtioConfig, VirtioNetDriver};
use morpheus_network::driver::intel::{E1000eConfig, E1000eDriver};
use morpheus_network::mainloop::{download_with_config, DownloadConfig, DownloadResult};

// 1. Run hwinit (after ExitBootServices)
let platform_config = PlatformConfig {
    dma_base,
    dma_bus,
    dma_size: 2 * 1024 * 1024,
    tsc_freq,
};
let platform = unsafe { platform_init(platform_config)? };

// 2. Find network device from hwinit result
let net_dev = platform.net_devices.iter()
    .find_map(|d| *d)
    .ok_or("no network device")?;

// 3. Create driver (brutal reset happens here)
let mut driver = match net_dev.device_type {
    NetDeviceType::VirtIO => {
        let cfg = VirtioConfig { dma_cpu_base: dma_base, ... };
        VirtioNetDriver::new(net_dev.mmio_base, cfg)?
    }
    NetDeviceType::IntelE1000e => {
        let cfg = E1000eConfig::new(dma_base, dma_bus, tsc_freq);
        E1000eDriver::new(net_dev.mmio_base, cfg)?
    }
};

// 4. Configure and execute download
let config = DownloadConfig::download_only("http://example.com/image.iso");
let result = download_with_config(&mut driver, config, None, tsc_freq);
```

## Preconditions

Before calling network functions:

1. **ExitBootServices completed** - No UEFI runtime
2. **hwinit::platform_init() called** - PCI scanned, bus mastering enabled
3. **DMA region allocated** - Identity-mapped, cache-coherent

## Driver Reset Contract

All drivers perform **brutal reset** on init:

- Mask and clear all interrupts
- Disable RX/TX with quiescence polling
- Full device reset with timeout
- Wait for EEPROM auto-read
- Clear all descriptor pointers
- Disable loopback explicitly
- Rebuild queues from scratch

See `driver/RESET_CONTRACT.md` for details.

## Modules

| Module | Purpose |
|--------|---------|
| `mainloop` | State machine orchestration, entry point |
| `driver` | NetworkDriver trait, VirtIO, Intel e1000e |
| `boot` | Legacy handoff support (deprecated) |
| `asm` | Assembly bindings (MMIO, PIO, TSC) |
| `dma` | DMA buffer management |
| `time` | TSC-based timing |

## State Machine

| State | Description |
|-------|-------------|
| Init | Initialize smoltcp interface |
| GptPrep | Prepare GPT if writing to disk |
| LinkWait | Wait for link up |
| DHCP | Obtain IP address |
| DNS | Resolve hostname |
| Connect | TCP connection |
| HTTP | HTTP GET and streaming receive |
| Manifest | Write manifest to disk |
| Done | Reboot |
- UEFI Specification 2.10, Section 11.1 (EFI Service Binding Protocol)
