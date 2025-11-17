# Network Stack Architecture

## Module Structure

```
network/src/
├── lib.rs                      - Public API exports
├── error.rs                    - Error types
├── types.rs                    - Common types (HttpMethod, callbacks)
├── protocol/                   - UEFI protocol layer
│   ├── mod.rs
│   └── uefi/
│       ├── mod.rs
│       ├── bindings.rs         - Protocol bindings (refs bootloader)
│       └── manager.rs          - Protocol lifecycle management
├── http/                       - HTTP message handling
│   ├── mod.rs
│   ├── request.rs              - HTTP request representation
│   ├── response.rs             - HTTP response representation
│   └── headers.rs              - Header management
├── url/                        - URL parsing
│   ├── mod.rs
│   └── parser.rs               - URL parser
├── transfer/                   - Data transfer
│   ├── mod.rs
│   ├── chunked.rs              - Chunked encoding
│   └── streaming.rs            - Streaming downloads
├── client/                     - HTTP client interface
│   ├── mod.rs                  - HttpClient trait
│   └── uefi/
│       ├── mod.rs
│       ├── client.rs           - UEFI HTTP client impl
│       └── downloader.rs       - High-level download API
└── utils/                      - Utilities
    ├── mod.rs
    ├── string.rs               - String conversion
    └── buffer.rs               - Buffer management
```

## Layer Responsibilities

### 1. Protocol Layer (`protocol/`)
- Locate UEFI protocols (HTTP, ServiceBinding)
- Create/destroy protocol instances
- Manage protocol handles and lifecycle
- Configure UEFI HTTP settings

### 2. HTTP Layer (`http/`)
- Build HTTP request messages
- Parse HTTP response messages
- Manage headers (add, get, remove)
- Handle status codes

### 3. URL Layer (`url/`)
- Parse URLs: `scheme://host[:port]/path[?query]`
- Validate HTTP/HTTPS schemes
- Extract URL components
- Default ports (80/443)

### 4. Transfer Layer (`transfer/`)
- Handle chunked transfer encoding
- Stream large downloads
- Progress tracking
- Buffer management

### 5. Client Layer (`client/`)
- High-level HTTP client interface (trait)
- UEFI implementation
- Request/response lifecycle
- Download manager

### 6. Utils Layer (`utils/`)
- String conversions (ASCII/UTF-16 for UEFI)
- Buffer pools
- Hex parsing
- Common utilities

## Data Flow

```
User Code
    ↓
Downloader::download(url)
    ↓
UefiHttpClient::request()
    ↓
ProtocolManager (UEFI protocols)
    ↓
UEFI HTTP Protocol
    ↓
Firmware Network Stack
    ↓
Hardware
```

## Implementation Phases

### Phase 1: Foundation ✅
- [x] Module structure
- [x] Error types
- [x] Basic types
- [x] Stubs with TODOs

### Phase 2: URL & HTTP Parsing
- [ ] URL parser implementation
- [ ] HTTP request builder
- [ ] HTTP response parser
- [ ] Header management

### Phase 3: UEFI Protocol Integration  
- [ ] Protocol manager
- [ ] Locate HTTP protocol
- [ ] Create/configure instances
- [ ] Handle lifecycle

### Phase 4: Client Implementation
- [ ] UEFI HTTP client
- [ ] Request execution
- [ ] Response handling
- [ ] Sync over async UEFI events

### Phase 5: Transfer Handling
- [ ] Chunked encoding decoder
- [ ] Streaming with progress
- [ ] Buffer management

### Phase 6: High-Level API
- [ ] Downloader implementation
- [ ] Progress callbacks
- [ ] Error handling
- [ ] Integration with bootloader TUI

## Key Design Decisions

1. **Trait-based abstraction**: `HttpClient` trait allows future implementations
2. **No external dependencies**: Pure `no_std` implementation
3. **UEFI protocols in bootloader**: Network crate references bootloader's UEFI bindings
4. **Modular design**: Each layer has clear responsibilities
5. **Progress tracking**: Built-in support for download progress

## Testing Strategy

1. **Unit tests**: Test URL parsing, header management (when possible in no_std)
2. **Integration tests**: Test in QEMU with OVMF
3. **Real hardware**: Test on physical machines with different UEFI implementations

## Usage Example (Future)

```rust
use morpheus_network::{UefiHttpClient, client::uefi::Downloader};

// Initialize
let mut client = UefiHttpClient::new(boot_services)?;
let mut downloader = Downloader::new(&mut client);

// Download with progress
let iso = downloader.download_with_progress(
    "http://releases.ubuntu.com/24.04/ubuntu.iso",
    |bytes, total| {
        if let Some(total) = total {
            let percent = (bytes * 100) / total;
            println!("{}%", percent);
        }
    }
)?;

// Save to ESP
save_file("/EFI/morpheus/isos/ubuntu.iso", &iso)?;
```

## TODOs by Priority

1. **URL Parser** - Need this first for everything else
2. **HTTP Message Builder** - Build requests, parse responses
3. **Protocol Manager** - Interface with UEFI
4. **UEFI HTTP Client** - Core implementation
5. **Transfer Handling** - Chunked/streaming
6. **Utils** - String conversion, buffers
7. **Downloader** - High-level API
8. **TUI Integration** - Progress display in bootloader

## Notes

- All protocol bindings live in `bootloader/src/uefi/`
- Network crate is pure logic, no FFI
- Focus on HTTP first, HTTPS later (needs TLS)
- ARM64 support will use same UEFI protocols
