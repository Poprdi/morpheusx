# MorpheusX Network Stack

Bare-metal HTTP client for post-ExitBootServices execution.

## Architecture

```
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

## Usage

```rust
use morpheus_network::mainloop::{download_with_config, DownloadConfig, DownloadResult};
use morpheus_network::boot::probe::{probe_and_create_driver, ProbeResult};

// 1. Probe PCI and create driver (brutal reset happens during driver::new())
let driver = match probe_and_create_driver(&dma, tsc_freq)? {
    ProbeResult::Intel(d) => UnifiedNetworkDriver::Intel(d),
    ProbeResult::VirtIO(d) => UnifiedNetworkDriver::VirtIO(d),
};

// 2. Configure download
let config = DownloadConfig::full(
    "http://example.com/image.iso",
    start_sector,
    0,  // offset
    esp_lba,
    partition_uuid,
    "image.iso",
);

// 3. Execute download with optional disk write
let result = download_with_config(&mut driver, config, Some(blk_device), tsc_freq);

match result {
    DownloadResult::Success { bytes_downloaded, bytes_written } => { /* done */ }
    DownloadResult::Failed { reason } => { /* handle error */ }
}
```

## Preconditions

Before calling network functions:

1. **ExitBootServices completed** - No UEFI runtime
2. **hwinit has run** - Platform normalized (bus mastering, DMA, cache coherency)
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
| `boot` | Device probing, driver creation helpers |
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
