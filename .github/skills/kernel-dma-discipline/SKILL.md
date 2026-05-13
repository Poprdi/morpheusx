---
name: kernel-dma-discipline
description: 'DMA correctness for bare-metal drivers: cache coherency, write-combining, ownership transfer between CPU and device, posted-write flushing, dma_addr vs virt_addr distinction, IOMMU domains, identity vs translated DMA, no allocation in IRQ for DMA buffers, ring buffer descriptor ordering, doorbell write barriers. Based on Linux Documentation/core-api/dma-api.rst and dma-api-howto.rst.'
argument-hint: "DMA path or buffer ownership question"
---

# Kernel DMA Discipline

## When to Use
- Allocating a buffer that a device will read/write via DMA
- Building or modifying a ring descriptor (NIC TX/RX ring, NVMe queue, etc.)
- Debugging silent data corruption in DMA paths
- Adding a new bus-mastering device driver
- Reasoning about cache coherency on the boundary

## The DMA Mental Model

A DMA buffer has three distinct addresses:

| Address | Who uses it | Translation |
|---------|-------------|-------------|
| Virtual | CPU code | MMU page tables |
| Physical | CPU bus, identity-mapped pages | None on identity map |
| DMA / Bus | Device | IOMMU (if present) or = physical |

**The CPU and the device never use the same address type.** `dma_addr_t` (or your kernel's equivalent) is what you give the device. The CPU pointer is for CPU access.

In `morpheusx`, `hwinit::dma` is the single source of truth. Drivers must not bypass it.

## Ownership Transfer

A DMA buffer is owned by exactly one party at a time:

```
CPU writes data
   │
   ▼
[transfer ownership to device]   <- cache flush, memory barrier, then doorbell write
   │
   ▼
Device reads / writes
   │
   ▼
[device signals completion via interrupt / status flag]
   │
   ▼
[transfer ownership to CPU]      <- cache invalidate (if non-coherent), memory barrier
   │
   ▼
CPU reads result
```

**Touching a buffer the device owns is a race.** You will see corruption or stale data.

## Coherent vs Streaming DMA

| Type | Use case | Cost |
|------|----------|------|
| Coherent | Long-lived, frequently exchanged (rings, doorbells) | Allocated as uncached/write-combining |
| Streaming | Single-shot transfer (a packet, a disk block) | Cached, requires explicit sync at ownership transfer |

For x86 with cache-coherent PCIe (the common case), streaming sync is often a no-op — but the API calls must still be there for portability and for the platforms where it matters.

## The Three Sins

1. **Reading a DMA buffer while the device may still be writing it.** Always check the descriptor "done" bit *with proper acquire ordering* before reading.
2. **Writing to a descriptor without flushing before doorbell.** The device may see the doorbell before your write reaches memory.
3. **Allocating DMA memory in IRQ context.** DMA-coherent allocators are not interrupt-safe.

## The Doorbell Pattern

```rust
// 1. Build descriptor (CPU side, cached writes are fine)
desc.addr = buf_dma_addr;
desc.len = len;
desc.flags = DESC_VALID;

// 2. Make sure descriptor reaches memory before device sees doorbell
core::sync::atomic::fence(Ordering::Release);
// (on weakly-ordered arches, may also need wmb-equivalent)

// 3. Ring the doorbell — MMIO write tells device "go look at the ring"
unsafe { write_volatile(doorbell_reg, ring_index); }

// 4. Posted-write flush — read back to ensure the doorbell write reached the device
let _ = unsafe { read_volatile(doorbell_reg); };
```

## Cache Coherency Caveats

x86 PCIe is cache-coherent — the chipset snoops CPU caches for DMA reads. You generally don't need explicit flushes.

**However**:

- Memory mapped with the wrong page attributes (uncached, write-combining) bypasses the cache hierarchy
- Some embedded x86 platforms have non-coherent DMA on certain buses
- Other architectures (ARM, RISC-V) commonly require explicit cache maintenance

Code that assumes coherency without using the DMA API is non-portable kernel code. Use the DMA API even on x86 — the abstractions are zero-cost when coherency is implicit.

## DMA Buffer Allocation Rules

- Allocate DMA buffers at init or from process context — never from IRQ
- Buffers must be physically contiguous unless using scatter-gather
- Alignment must satisfy device requirements (often 64 or 4096 bytes)
- Address must satisfy the device's DMA mask (e.g., 32-bit-only devices need buffers below 4 GiB)
- Free buffers only after the device has been quiesced

## DMA Mask

A device with a 32-bit DMA mask cannot reach addresses above 4 GiB. If you allocate above the mask, you either:

- Crash (modern PCIe with no DMAR)
- Get silently corrupted data (legacy PCI without remapping)
- Use bounce buffers via SWIOTLB (Linux's fallback)

Set the DMA mask at device probe; refuse to attach if the mask cannot be satisfied.

## IOMMU / DMAR

If an IOMMU is enabled (Intel VT-d, AMD-Vi):

- The device's view of memory is translated; `dma_addr` no longer equals physical
- A misbehaving device cannot scribble outside its allowed regions (security win)
- DMA setup involves mapping into an IOMMU domain — slower than identity DMA
- Driver code is the same; the DMA API hides the difference

`hwinit/src/dma/` should expose a single `map`/`unmap` API that drivers use; whether it's identity or IOMMU-translated is invisible to the driver.

## Common Failure Modes

| Symptom | Cause |
|---------|-------|
| Device reads zeros / stale data | No barrier before doorbell; descriptor not flushed |
| CPU reads wrong RX data | Read before checking "done" bit; missing Acquire on done check |
| Random corruption under load | Buffer reuse before device finished previous DMA |
| Works on QEMU, fails on real PCIe | Cache coherency assumption that's only true in QEMU |
| 32-bit device gets 64-bit address | DMA mask not enforced; buffer allocated above 4 GiB |
| IOMMU fault / DMAR error | DMA address not mapped, or mapped read-only when device wants to write |

## Procedure

1. Use `hwinit::dma` for every DMA buffer — never roll your own.
2. Always pair "make descriptor visible to device" with a memory barrier and posted-write flush.
3. Always check the "done" bit with `Acquire` ordering before reading device-written data.
4. Never allocate DMA memory in IRQ context.
5. On real hardware: test under load; cache coherency bugs are rare-but-fatal.

## References
- `Documentation/core-api/dma-api.rst`
- `Documentation/core-api/dma-api-howto.rst`
- `Documentation/driver-api/pci/pci.rst`
- PCIe Base Spec § 2.4 (transaction ordering)
