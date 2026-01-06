# Network Stack Architecture V2
## Universal Bare Metal Design

## Core Philosophy

**Goal**: 90% machine coverage with zero firmware dependencies
**Strategy**: Multi-backend architecture with runtime fallback chain
**Constraint**: Pure `no_std`, works before and after ExitBootServices

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                     Public API Layer                        │
│  download(url) → auto-selects best backend                 │
└─────────────────────────────────────────────────────────────┘
                            ↓
┌─────────────────────────────────────────────────────────────┐
│                  Backend Abstraction (Trait)                │
│  NetworkBackend trait: download(), connect(), etc.         │
└─────────────────────────────────────────────────────────────┘
                            ↓
        ┌───────────────────┴───────────────────┐
        ↓                                       ↓
┌──────────────────┐                  ┌──────────────────┐
│ UEFI HTTP        │                  │ Raw Stack        │
│ (Firmware deps)  │                  │ (Universal)      │
│ - Try first      │                  │ - Fallback       │
│ - ~60% coverage  │                  │ - ~90% coverage  │
└──────────────────┘                  └──────────────────┘
                                               ↓
                        ┌──────────────────────┴──────────────────────┐
                        ↓                                             ↓
              ┌─────────────────┐                          ┌─────────────────┐
              │  HTTP Layer     │                          │  smoltcp Stack  │
              │  (Application)  │                          │  (Transport)    │
              └─────────────────┘                          └─────────────────┘
                        ↓                                             ↓
              ┌─────────────────┐                          ┌─────────────────┐
              │  TCP Socket     │                          │  TCP/UDP/IP     │
              │                 │                          │  DHCP/DNS/ARP   │
              └─────────────────┘                          └─────────────────┘
                                                                     ↓
                                                          ┌─────────────────┐
                                                          │  Device Layer   │
                                                          │  (NIC drivers)  │
                                                          └─────────────────┘
                                                                     ↓
                                                ┌────────────────────┼────────────────────┐
                                                ↓                    ↓                    ↓
                                        ┌──────────────┐   ┌──────────────┐   ┌──────────────┐
                                        │ VirtIO-net   │   │ Intel e1000  │   │ Realtek 8169 │
                                        │ (QEMU/KVM)   │   │ (80% h/w)    │   │ (cheap h/w)  │
                                        └──────────────┘   └──────────────┘   └──────────────┘
```

---

## Module Structure (30 files, ~8,000 lines)

```
network/
├── Cargo.toml                          # Dependencies: smoltcp (no_std)
├── ARCHITECTURE_V2.md                  # This file
├── README.md                           # User-facing docs
│
├── src/
│   ├── lib.rs                          # Public API exports (100 lines)
│   ├── error.rs                        # Error types (100 lines)
│   ├── types.rs                        # Common types (100 lines)
│   │
│   ├── backend/                        # Backend abstraction layer
│   │   ├── mod.rs                      # NetworkBackend trait (150 lines)
│   │   ├── selector.rs                 # Auto-select backend (200 lines)
│   │   ├── uefi_http.rs                # UEFI HTTP backend (400 lines)
│   │   └── raw_stack.rs                # Raw socket backend (400 lines)
│   │
│   ├── http/                           # HTTP protocol layer
│   │   ├── mod.rs                      # Public API (50 lines)
│   │   ├── request.rs                  # HTTP request builder (300 lines)
│   │   ├── response.rs                 # HTTP response parser (350 lines)
│   │   ├── headers.rs                  # Header management (250 lines)
│   │   ├── status.rs                   # Status code constants (100 lines)
│   │   └── chunked.rs                  # Chunked transfer encoding (200 lines)
│   │
│   ├── tcp/                            # TCP abstraction (wraps smoltcp)
│   │   ├── mod.rs                      # Public API (100 lines)
│   │   ├── socket.rs                   # TCP socket wrapper (400 lines)
│   │   └── stream.rs                   # Streaming I/O (300 lines)
│   │
│   ├── url/                            # URL parsing
│   │   ├── mod.rs                      # Public API (50 lines)
│   │   └── parser.rs                   # URL parser (400 lines)
│   │
│   ├── dns/                            # DNS client
│   │   ├── mod.rs                      # Public API (100 lines)
│   │   ├── resolver.rs                 # DNS resolver (350 lines)
│   │   └── cache.rs                    # Simple DNS cache (150 lines)
│   │
│   ├── stack/                          # Network stack integration
│   │   ├── mod.rs                      # Stack manager (200 lines)
│   │   ├── smoltcp_wrapper.rs          # smoltcp integration (400 lines)
│   │   ├── dhcp.rs                     # DHCP client (300 lines)
│   │   └── config.rs                   # Network configuration (150 lines)
│   │
│   ├── device/                         # NIC device abstraction
│   │   ├── mod.rs                      # Device trait (150 lines)
│   │   ├── probe.rs                    # PCI device enumeration (400 lines)
│   │   │
│   │   ├── virtio.rs                   # VirtIO-net driver (450 lines)
│   │   │
│   │   ├── realtek/                    # Realtek drivers (40% market)
│   │   │   ├── mod.rs                  # Common Realtek code (200 lines)
│   │   │   ├── rtl8111.rs              # RTL8111/8168 (consumer, 450 lines)
│   │   │   └── rtl8125.rs              # RTL8125 2.5GbE (450 lines)
│   │   │
│   │   ├── intel/                      # Intel drivers (45% market)
│   │   │   ├── mod.rs                  # Common Intel code (200 lines)
│   │   │   ├── e1000.rs                # Legacy e1000 (450 lines)
│   │   │   ├── e1000e.rs               # Modern e1000e (450 lines)
│   │   │   └── i219.rs                 # i219/i225/i226 (450 lines)
│   │   │
│   │   ├── broadcom/                   # Broadcom drivers (10% market)
│   │   │   ├── mod.rs                  # Common Broadcom code (200 lines)
│   │   │   ├── tg3.rs                  # NetXtreme (450 lines)
│   │   │   └── bnx2.rs                 # NetXtreme II (450 lines)
│   │   │
│   │   └── registers/                  # Hardware register definitions
│   │       ├── mod.rs                  # Re-exports (100 lines)
│   │       ├── realtek_regs.rs         # Realtek registers (400 lines)
│   │       ├── intel_regs.rs           # Intel registers (400 lines)
│   │       └── broadcom_regs.rs        # Broadcom registers (400 lines)
│   │
│   ├── pci/                            # PCI enumeration
│   │   ├── mod.rs                      # PCI abstraction (200 lines)
│   │   ├── config_space.rs             # PCI config access (250 lines)
│   │   ├── bar.rs                      # BAR parsing (200 lines)
│   │   └── ids.rs                      # Vendor/device IDs (100 lines)
│   │
│   ├── buffer/                         # Buffer management
│   │   ├── mod.rs                      # Public API (100 lines)
│   │   ├── ring.rs                     # Ring buffer (300 lines)
│   │   └── pool.rs                     # Buffer pool (250 lines)
│   │
│   └── utils/                          # Utilities
│       ├── mod.rs                      # Re-exports (50 lines)
│       ├── checksum.rs                 # Network checksums (150 lines)
│       ├── endian.rs                   # Endianness conversion (100 lines)
│       └── time.rs                     # Timing abstraction (100 lines)
│
└── examples/
    ├── download_iso.rs                 # Example: download ISO
    └── simple_http.rs                  # Example: simple HTTP GET
```

**Total: ~40 files, ~8,500 lines** (no file > 500 lines)

---

## Layer Responsibilities

### 1. Backend Layer (`backend/`)
**Purpose**: Abstraction over different network implementations

```rust
pub trait NetworkBackend {
    fn is_available() -> bool;
    fn download(&mut self, url: &str) -> Result<Vec<u8>>;
    fn download_with_progress(&mut self, url: &str, callback: ProgressCallback) -> Result<Vec<u8>>;
}
```

**Implementations**:
- `UefiHttpBackend` - Uses UEFI HTTP protocol (optional, requires UEFI)
- `RawStackBackend` - Uses smoltcp + custom drivers (universal)

**Selector logic**:
```rust
pub fn create_network_backend() -> Box<dyn NetworkBackend> {
    // Try UEFI first (faster if available)
    if UefiHttpBackend::is_available() {
        Box::new(UefiHttpBackend::new())
    } else {
        // Fallback to raw stack
        Box::new(RawStackBackend::new())
    }
}
```

### 2. HTTP Layer (`http/`)
**Purpose**: HTTP/1.1 protocol implementation

**Modules**:
- `request.rs` - Build HTTP requests (GET, POST, headers)
- `response.rs` - Parse HTTP responses (status, headers, body)
- `headers.rs` - Header manipulation
- `chunked.rs` - Decode chunked transfer encoding

**No external dependencies**, pure parsing logic.

### 3. TCP Layer (`tcp/`)
**Purpose**: Thin wrapper over smoltcp TCP sockets

**Provides**:
- Connection management (connect, close)
- Send/receive with buffering
- Non-blocking I/O abstraction
- Timeout handling

### 4. URL Layer (`url/`)
**Purpose**: Parse HTTP URLs

```rust
pub struct Url {
    pub scheme: Scheme,      // http, https
    pub host: Host,          // domain or IP
    pub port: u16,           // default 80/443
    pub path: &str,
}
```

### 5. DNS Layer (`dns/`)
**Purpose**: Resolve hostnames to IP addresses

**Features**:
- UDP-based DNS queries
- Simple cache (avoid repeated lookups)
- Fallback to common DNS servers (8.8.8.8, 1.1.1.1)

### 6. Stack Layer (`stack/`)
**Purpose**: Initialize and manage smoltcp network stack

**Responsibilities**:
- Create smoltcp `Interface`
- Configure IP address (DHCP or static)
- Poll loop integration
- ARP table management

### 7. Device Layer (`device/`)
**Purpose**: NIC driver abstraction and implementations

**Trait**:
```rust
pub trait Device {
    fn mac_address(&self) -> [u8; 6];
    fn transmit(&mut self, packet: &[u8]) -> Result<()>;
    fn receive(&mut self) -> Option<&[u8]>;
}
```

**Drivers** (priority order for 95%+ coverage):
1. **VirtIO-net** - QEMU/KVM/VirtualBox (testing/VM)
2. **Realtek RTL8111/8168** - Most consumer boards (~35% market)
3. **Intel e1000e** - Modern Intel boards (~20% market)
4. **Intel e1000** - Legacy Intel (~10% market)
5. **Realtek RTL8125** - 2.5GbE consumer boards (~5% market)
6. **Broadcom NetXtreme** - Enterprise servers/workstations (~10% market)
7. **Intel i219/i225** - Recent Intel chipsets (~15% market)

**Probe logic**:
```rust
pub fn probe_devices() -> Option<Box<dyn Device>> {
    // Try in order of likelihood
    if let Some(d) = VirtioDevice::probe() { return Some(Box::new(d)); }
    if let Some(d) = Rtl8111Device::probe() { return Some(Box::new(d)); }  // Most common
    if let Some(d) = E1000eDevice::probe() { return Some(Box::new(d)); }
    if let Some(d) = E1000Device::probe() { return Some(Box::new(d)); }
    if let Some(d) = Rtl8125Device::probe() { return Some(Box::new(d)); }
    if let Some(d) = BroadcomDevice::probe() { return Some(Box::new(d)); }
    if let Some(d) = IntelI219Device::probe() { return Some(Box::new(d)); }
    None
}
```

### 8. PCI Layer (`pci/`)
**Purpose**: PCI device enumeration (scan for NICs)

**Features**:
- Enumerate PCI bus
- Read config space
- Parse BARs (MMIO/Port I/O addresses)
- Match vendor/device IDs

**No UEFI dependency** - direct I/O port access (0xCF8/0xCFC).

### 9. Buffer Layer (`buffer/`)
**Purpose**: DMA buffer management

**Features**:
- Ring buffers for TX/RX
- Buffer pooling (reduce allocations)
- Alignment helpers (for DMA)

### 10. Utils Layer (`utils/`)
**Purpose**: Shared utilities

- Checksum calculation (TCP/IP/UDP)
- Endianness conversions (network byte order)
- Timing abstraction (UEFI vs bare metal)

---

## Dependencies

```toml
[dependencies]
# Core network stack (no_std, requires alloc)
smoltcp = { version = "0.11", default-features = false, features = [
    "proto-ipv4",
    "proto-dhcpv4", 
    "proto-dns",
    "socket-tcp",
    "socket-udp",
    "socket-icmp",
] }

# No other external dependencies!
# PCI access, drivers, HTTP - all custom
```

**Note**: smoltcp requires `alloc` crate, which is fine:
- UEFI: Boot services provide allocator
- Bare metal: Can use static allocator (buddy/bump allocator)

---

## Memory Model

### Option 1: UEFI (Before ExitBootServices)
```rust
extern crate alloc;

// Use heap allocations
let buffers = vec![0u8; 8192];
```

### Option 2: Bare Metal (After ExitBootServices)
```rust
#![no_std]
extern crate alloc; // Custom allocator

use linked_list_allocator::LockedHeap;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

// Initialize with static memory region
unsafe {
    ALLOCATOR.lock().init(HEAP_START, HEAP_SIZE);
}
```

**Both work with smoltcp.**

---

## Hardware Coverage Analysis

### Consumer/Desktop Market
| NIC Driver | Market Share | Complexity | Priority |
|------------|--------------|------------|----------|
| **Realtek RTL8111/8168** | ~35% | Medium | ⭐⭐⭐ Critical |
| **Intel e1000e** | ~20% | Medium | ⭐⭐⭐ Critical |
| **Intel i219/i225/i226** | ~15% | Medium-High | ⭐⭐ Important |
| **Intel e1000** | ~10% | Medium | ⭐ Legacy |
| **Realtek RTL8125** | ~5% | Medium | ⭐ 2.5GbE |
| **Broadcom (consumer)** | ~2% | High | Optional |

### Server/Enterprise Market
| NIC Driver | Market Share | Complexity | Priority |
|------------|--------------|------------|----------|
| **Intel (all)** | ~50% | Medium | ⭐⭐⭐ Critical |
| **Broadcom NetXtreme** | ~35% | High | ⭐⭐ Important |
| **Mellanox** | ~10% | Very High | ❌ Skip |

### Virtual Machines
| Driver | Coverage | Complexity | Priority |
|--------|----------|------------|----------|
| **VirtIO-net** | 100% KVM/QEMU | Low | ⭐⭐⭐ Critical |

### Coverage Tiers

**Tier 1: Essential (95% coverage)**
```
VirtIO-net    - All VMs
RTL8111/8168  - 35% consumer
e1000e        - 20% modern Intel  
i219/i225     - 15% recent Intel
e1000         - 10% legacy Intel
RTL8125       - 5% 2.5GbE
Broadcom TG3  - 10% servers
```
**Total: 7 drivers = 95%+ coverage**

**Tier 2: Extended (98% coverage)**
```
+ Broadcom bnx2  - 3% older servers
+ Mellanox       - 2% high-end
```
**Total: 9 drivers = 98% coverage**

**Trade-off Decision**: Implement Tier 1 (7 drivers) for 95% coverage. Mellanox complexity not worth 2% gain.

---

## Runtime Behavior

### Boot Sequence
```rust
// 1. Initialize network backend
let mut network = network::init()?;

// 2. Auto-select best backend
// - Tries UEFI HTTP first
// - Falls back to raw stack + driver probe

// 3. Download ISO
let iso_data = network.download(
    "http://releases.ubuntu.com/24.04/ubuntu-24.04-live-server-amd64.iso"
)?;

// 4. Mount with iso9660
let volume = iso9660::mount(&iso_data, 0)?;

// 5. Extract kernel
let kernel = iso9660::find_file(&volume, "/casper/vmlinuz")?;

// 6. Boot it
boot_linux_kernel(kernel)?;
```

---

## Error Handling Strategy

```rust
pub enum NetworkError {
    // Hardware errors
    NoNicFound,
    NicInitFailed,
    DmaError,
    
    // Stack errors
    NoIpAddress,
    DhcpTimeout,
    DnsLookupFailed,
    
    // Protocol errors
    ConnectionRefused,
    ConnectionTimeout,
    HttpError(u16),
    InvalidResponse,
    
    // UEFI errors (optional backend)
    UefiProtocolNotFound,
    UefiOperationFailed,
}
```

**Graceful degradation**: If UEFI fails, try raw. If raw fails, clear error.

---

## Testing Strategy

### QEMU Testing
```bash
# Test VirtIO-net driver
qemu-system-x86_64 -device virtio-net-pci,netdev=net0 \
    -netdev user,id=net0,hostfwd=tcp::8080-:80

# Test e1000 driver  
qemu-system-x86_64 -device e1000,netdev=net0 \
    -netdev user,id=net0
```

### Real Hardware Testing
- Test on Intel motherboard (e1000e)
- Test on cheap Realtek board (rtl8169)
- Test on server with e1000

### Fallback Testing
- Disable UEFI HTTP in firmware settings
- Verify raw stack takes over
- Measure performance difference

---

## Performance Targets

| Operation | Target | Notes |
|-----------|--------|-------|
| Driver init | < 500ms | NIC reset + DHCP |
| DNS lookup | < 100ms | Cached: < 1ms |
| TCP connect | < 50ms | Local network |
| Download 10MB | < 5s | Gigabit LAN |
| Download 1GB ISO | < 2min | Gigabit LAN |

**Bottleneck**: Network bandwidth, not stack performance.

---

## Implementation Phases

### Phase 1: Foundation (Week 1)
- [ ] Backend trait + selector
- [ ] Error types
- [ ] URL parser
- [ ] PCI enumeration
- [ ] Basic HTTP request/response

### Phase 2: UEFI Backend (Week 2)
- [ ] UEFI HTTP protocol wrapper
- [ ] Simple download function
- [ ] Test in QEMU with OVMF

### Phase 3: VirtIO Driver (Week 3)
- [ ] VirtIO PCI device probe
- [ ] Virtqueue management
- [ ] TX/RX implementation
- [ ] Test with smoltcp

### Phase 4: smoltcp Integration (Week 4)
- [ ] Stack initialization
- [ ] DHCP client
- [ ] DNS resolver
- [ ] TCP socket wrapper

### Phase 5: HTTP over TCP (Week 5)
- [ ] HTTP client on smoltcp
- [ ] Chunked encoding
- [ ] Progress callbacks
- [ ] End-to-end ISO download

### Phase 6: Realtek RTL8111 Driver (Week 6-7) - CRITICAL
- [ ] PCI probe (Realtek vendor ID 0x10EC)
- [ ] Register initialization
- [ ] RX/TX ring buffers
- [ ] DMA setup
- [ ] Test on consumer motherboard

### Phase 7: Intel e1000e Driver (Week 8)
- [ ] Reuse Intel common code
- [ ] Test on modern Intel boards

### Phase 8: Intel i219/i225 Driver (Week 9)
- [ ] PCIe variant handling
- [ ] Test on recent motherboards

### Phase 9: Broadcom TG3 Driver (Week 10-11) - Server Coverage
- [ ] PCI probe (Broadcom vendor ID 0x14E4)
- [ ] More complex initialization
- [ ] Test on server hardware

### Phase 10: Additional Drivers (Week 12+) - Optional
- [ ] Realtek RTL8125 (2.5GbE)
- [ ] Intel e1000 (legacy)
- [ ] Broadcom bnx2 (optional)

---

## Code Quality Rules

1. **No file > 500 lines** (enforced)
2. **Every module has docs** (enforced)
3. **Public API has examples** (enforced)
4. **Unsafe code has safety comments** (enforced)
5. **Error types are descriptive** (enforced)
6. **No unwrap() in drivers** (enforced)

---

## Success Criteria

- ✅ Download 1GB ISO from internet in QEMU
- ✅ Download 1GB ISO on real Intel hardware
- ✅ Download 1GB ISO on real Realtek hardware
- ✅ Graceful fallback when UEFI HTTP unavailable
- ✅ Works before and after ExitBootServices
- ✅ 90%+ machine coverage
- ✅ No file exceeds 500 lines
- ✅ Full documentation coverage

---

## Why This Design?

### Modularity
- Each layer is independent
- Can test layers in isolation
- Easy to add new drivers
- Clear separation of concerns

### Portability
- No UEFI dependency in core stack
- Works on bare metal x86_64
- ARM64 ready (add ARM drivers later)
- Firmware-agnostic

### Maintainability
- Small, focused files
- Clear responsibility boundaries
- Easy to understand flow
- Well-documented

### Performance
- Zero-copy where possible
- Efficient buffer management
- Minimal allocations in hot paths
- Direct hardware access (no firmware overhead)

---

**Status**: Architecture complete, ready for implementation
**Next**: Implement Phase 1 (foundation layer)
