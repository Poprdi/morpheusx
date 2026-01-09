# MorpheusX Network Stack

HTTP client for downloading ISOs and other files in the UEFI bootloader environment.



### Design Principles

1. **Platform Abstraction**: `HttpClient` trait allows multiple implementations
3. **No External Deps**: Everything built on primitives
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

## Implementation 

### See /docs/

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
