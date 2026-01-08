# morpheus-ping

A standalone, `no_std` ICMP ping utility for MorpheusX bootloader connectivity testing.

## Features

- **no_std compatible**: Works in bare-metal environments
- **Zero dependencies**: Self-contained implementation
- **Minimal footprint**: Designed for firmware/bootloader use
- **RFC-compliant**: Follows RFC 792 (ICMP) and RFC 791 (IPv4)

## Usage

```rust
use morpheus_ping::{Pinger, PingConfig, Ipv4Addr};

// Create pinger with default config
let config = PingConfig::default();
let mut pinger = Pinger::new(config);

// Build ping request
let src = Ipv4Addr::new(192, 168, 1, 100);
let dst = Ipv4Addr::CLOUDFLARE_DNS; // 1.1.1.1
let mut buffer = [0u8; 128];

let len = pinger.build_request(src, dst, &mut buffer)?;

// Send packet via your network driver...
// Receive reply...

// Parse reply
let result = pinger.parse_reply(&reply_buffer)?;
println!("RTT: {} ms", result.rtt_ms);
```

## Default Targets

The crate provides well-known reliable targets for connectivity testing:

- `1.1.1.1` - Cloudflare DNS (recommended, fastest)
- `8.8.8.8` - Google DNS
- `9.9.9.9` - Quad9 DNS

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
