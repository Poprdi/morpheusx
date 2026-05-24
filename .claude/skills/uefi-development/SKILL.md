---
name: uefi-development
description: |
  Implement UEFI firmware interaction following best practices.
  Handle ExitBootServices, GOP framebuffer, and proper error propagation.
author: MorpheusX Architecture Team
version: 2026.1
---

## Inputs Required
- Boot phase (pre/post ExitBootServices)
- UEFI protocol being used (GOP, SimpleText, BlockIO, etc.)
- Error handling strategy

## Process

### Step 1: UEFI Conventions
1. Use `uefi` crate for protocol interfaces
2. Convert UEFI statuses to custom error types
3. All pointers are valid or checked before dereference
4. Handle memory allocation via UEFI boot services only

### Step 2: ExitBootServices Handoff
```rust
// GOOD: Proper ExitBootServices sequence
pub fn exit_boot_services<'a>(
    handle: Handle,
    systab: &'a SystemTable<Boot>,
) -> Result<LoadedImageHandle, &'static str> {
    // 1. Locate and store any protocols needed post-boot
    // 2. Disable watchdog timer
    // 3. Call ExitBootServices
    // 4. Verify we own the memory map
}
```

### Step 3: Framebuffer (GOP)
1. Query GOP protocol from UEFI
2. Extract mode info (resolution, pitch, format)
3. Store framebuffer base and size
4. After ExitBootServices: direct MMIO access

### Step 4: Error Handling Patterns
```rust
// GOOD: Propagate UEFI errors properly
fn open_protocol<T>(&self) -> Result<&'static T, UefiError> {
    let ptr = self.boot_services.open_protocol::<T>(...)?;
    NonNull::new(ptr as *mut T).ok_or(UefiError::ProtocolNotFound)
}
```

## Critical Rules
- NO UEFI runtime services after ExitBootServices
- Watchdog timer must be disabled before handoff
- Memory map must be captured before ExitBootServices
- All protocol pointers validated before use

## References
- UEFI Specification 2.10
- `uefi` crate documentation