---
name: testing-validation
description: |
  Test and validate exokernel code with QEMU, OVMF, and integration tests.
  Cover boot sequences, hardware initialization, and syscall interfaces.
author: MorpheusX Architecture Team
version: 2026.1
---

## Inputs Required
- Test type (unit, integration, boot, stress)
- Target hardware (QEMU virtual, hardware)
- Serial output expectations

## Test Infrastructure

### Unit Tests
```rust
// In code:
#[cfg(test)]
mod tests {
    #[test]
    fn test_allocator_order() {
        // ...
    }
}
```

### Integration Tests (`testing/`)
- `run.sh`: Boot MorpheusX in QEMU
- `test-boot.exp`: Automated expect script
- Install scripts: Arch/Tails/Ubuntu live

### Serial Console
```bash
# Connect to QEMU serial
nc -localhost 1234
# Or via monitor
info chardev
```

## Process

### Step 1: Build Verification
```bash
cargo build --release --target x86_64-unknown-uefi
```

### Step 2: Run in QEMU
```bash
cd testing && ./run.sh
```

### Step 3: Capture Serial Output
```
# In another terminal
nc localhost 1234
```

### Step 4: Verify Boot Phases
1. UEFI entry
2. Memory ownership
3. CPU state (GDT/IDT)
4. Interrupt routing
5. Heap allocation
6. DMA region
7. PCI discovery
8. Paging
9. Scheduler
10. Syscall interface
11. HelixFS mount

## Common Issues
- OVMF path wrong: update in `testing/run.sh`
- ESP too small: check directory size + 50MB
- No serial output: verify -serial configuration
- Boot hang: enable GDB via `-s` flag

## Debugging
```bash
./debug.sh          # GDB connection to QEMU :1234
```

## References
- QEMU documentation
- OVMF/UEFI firmware docs