# MorpheusX Network Stack

HTTP client for downloading ISOs and other files in the UEFI bootloader environment.

## Architecture

### Core Abstractions (`src/`)

```
network/
├── client.rs        - HttpClient trait (platform-agnostic interface)
├── types.rs         - HTTP types (Request, Response, Method, etc.)
├── error.rs         - Error types and Result alias
├── download.rs      - High-level DownloadManager
└── uefi_impl/       - UEFI-specific implementation
    └── uefi_client.rs - UefiHttpClient (implements HttpClient)
```

### Design Principles

1. **Platform Abstraction**: `HttpClient` trait allows multiple implementations
2. **UEFI-First**: Primary implementation uses UEFI HTTP protocols
3. **No External Deps**: Everything built on UEFI primitives
4. **Progress Tracking**: Built-in support for download progress callbacks
5. **Error Handling**: Comprehensive error types for network operations

## Usage

### Basic Download

```rust
use morpheus_network::{UefiHttpClient, DownloadManager};

// Create UEFI HTTP client
let mut client = UefiHttpClient::new(boot_services)?;

// Create download manager
let mut downloader = DownloadManager::new(&mut client);

// Download a file
let data = downloader.download_to_memory("http://example.com/file.iso")?;
```

### Download with Progress

```rust
fn progress_callback(downloaded: usize, total: Option<usize>) {
    if let Some(total) = total {
        let percent = (downloaded * 100) / total;
        println!("Progress: {}%", percent);
    }
}

let data = downloader.download_with_progress(
    "http://example.com/large-file.iso",
    progress_callback
)?;
```

### Check File Size

```rust
if let Some(size) = downloader.get_file_size("http://example.com/file.iso")? {
    println!("File size: {} bytes", size);
}
```

## UEFI Protocol Usage

The UEFI implementation uses:

- `EFI_HTTP_PROTOCOL` - HTTP/HTTPS requests
- `EFI_SERVICE_BINDING_PROTOCOL` - Create/destroy HTTP instances
- `EFI_DHCP4_PROTOCOL` - Auto network configuration (future)

### Protocol Bindings

UEFI protocol definitions are in `bootloader/src/uefi/`:
- `http.rs` - EFI_HTTP_PROTOCOL structures and constants
- `service_binding.rs` - EFI_SERVICE_BINDING_PROTOCOL

## Implementation Status

### ✅ Completed
- [x] Core trait definitions
- [x] HTTP request/response types
- [x] Error handling
- [x] Download manager API
- [x] UEFI protocol bindings

### 🚧 In Progress
- [ ] UefiHttpClient implementation
- [ ] URL parsing
- [ ] Header handling
- [ ] Progress tracking

### 📋 Planned
- [ ] HTTPS/TLS support
- [ ] Resume/range requests
- [ ] Connection pooling
- [ ] DNS caching
- [ ] Redirect following
- [ ] Timeout handling

## Error Handling

All operations return `Result<T, NetworkError>`:

```rust
match downloader.download_to_memory(url) {
    Ok(data) => { /* success */ },
    Err(NetworkError::ProtocolNotAvailable) => {
        // UEFI firmware doesn't support HTTP
    },
    Err(NetworkError::HttpError(404)) => {
        // File not found
    },
    Err(e) => {
        // Other error
    }
}
```

## Testing

Test in QEMU with OVMF UEFI firmware:
```bash
cd testing
./run.sh
```

Ensure QEMU has network configured:
```bash
-netdev user,id=net0 \
-device e1000,netdev=net0
```

## Future: ARM Support

When adding ARM64 support:
- Same UEFI protocols work on ARM
- No code changes needed in abstraction layer
- May need arch-specific optimizations for large transfers

## References

- UEFI Specification 2.10, Section 28.7 (EFI HTTP Protocol)
- UEFI Specification 2.10, Section 11.1 (EFI Service Binding Protocol)
