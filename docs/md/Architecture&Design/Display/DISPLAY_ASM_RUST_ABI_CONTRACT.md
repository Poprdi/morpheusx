# MorpheusX Display ASM ↔ Rust ABI Contract v1.0

**Status**: FROZEN  
**Date**: 2026-01-12  
**Authority**: Derived from NETWORK_ASM_RUST_ABI_CONTRACT.md (v1.0 Frozen)  
**Scope**: Post-ExitBootServices bare-metal display runtime  
**Target**: x86_64-unknown-uefi  

---

## Document Structure

This contract defines the **frozen v1 ABI** between assembly (ASM) code and Rust code for the MorpheusX post-ExitBootServices display runtime.

---

# Chunk 1 — Contract Scope & Global Invariants

## 1.1 Purpose of This Contract

This document defines the **frozen v1 ABI** between ASM and Rust for display operations.

**This contract specifies:**
- What ASM exposes to Rust for display operations
- What Rust may assume about ASM behavior
- What ASM may assume about Rust behavior
- What constitutes undefined behavior (UB)
- Memory ownership and lifetime rules for GPU resources
- Calling conventions and register usage

## 1.2 System Context (Non-Negotiable)

| Property | Value | Source |
|----------|-------|--------|
| Architecture | x86_64 | Inherited from network contract |
| Execution mode | Long mode (64-bit) | Inherited |
| ABI | Microsoft x64 (UEFI-compatible) | Inherited |
| Runtime phase | Post-ExitBootServices | Inherited |
| Interrupt state | Disabled (IF=0) | Inherited |
| Core count | 1 (BSP only) | Inherited |
| Thread count | 1 | Inherited |

## 1.3 Trust Boundaries

### 1.3.1 ASM Owns

| Domain | Rationale |
|--------|-----------|
| All GPU hardware interaction | Direct MMIO |
| Command queue submission | Virtqueue manipulation |
| GPU device state | Register reads/writes |
| Memory barriers | sfence/lfence/mfence placement |
| Fence management | GPU synchronization primitives |

### 1.3.2 Rust Owns

| Domain | Rationale |
|--------|-----------|
| Frame composition | Building render commands |
| Resource management | Creating/destroying GPU resources |
| Display state machines | Mode set, vsync, frame pacing |
| Error handling policy | Retry vs. fatal decisions |
| Performance instrumentation | Latency metrics |

## 1.4 Global Invariants

### 1.4.1 Execution Invariants

| ID | Invariant |
|----|-----------|
| **EXEC-INV-1** | All function calls return in bounded time |
| **EXEC-INV-2** | No function may yield, await, or suspend |
| **EXEC-INV-3** | No function may loop waiting for GPU fence |
| **EXEC-INV-4** | Interrupts remain disabled (IF=0) |

### 1.4.2 GPU Resource Invariants

| ID | Invariant |
|----|-----------|
| **GPU-INV-1** | Resources are identity-mapped (phys == virt) |
| **GPU-INV-2** | GPU DMA region is UC or WC (not WB) |
| **GPU-INV-3** | bus_addr for device, cpu_ptr for software |
| **GPU-INV-4** | Resources have exactly one owner at any time |
| **GPU-INV-5** | DEVICE-OWNED resources never accessed by driver |

### 1.4.3 VirtIO-GPU Invariants

| ID | Invariant |
|----|-----------|
| **VGPU-INV-1** | Control queue for commands, cursor queue for cursor |
| **VGPU-INV-2** | Feature negotiation complete before queue setup |
| **VGPU-INV-3** | Queue size read from device, not hardcoded |
| **VGPU-INV-4** | Fence IDs monotonically increasing |

---

# Chunk 2 — Calling Convention & ABI Rules

## 2.1 Calling Convention: Microsoft x64 ABI

All ASM ↔ Rust boundary crossings use the **Microsoft x64 calling convention**.

### 2.1.1 Parameter Passing

| Parameter | Register |
|-----------|----------|
| 1st | RCX |
| 2nd | RDX |
| 3rd | R8 |
| 4th | R9 |
| 5th+ | Stack |

### 2.1.2 Return Values

| Type | Location |
|------|----------|
| Integer ≤ 64 bits | RAX |
| Integer 128 bits | RAX:RDX |

### 2.1.3 Register Classification

**Volatile (Caller-Saved):** RAX, RCX, RDX, R8, R9, R10, R11

**Non-Volatile (Callee-Saved):** RBX, RBP, RDI, RSI, RSP, R12-R15

### 2.1.4 Shadow Space

32 bytes shadow space required before every call.

### 2.1.5 Stack Alignment

16-byte aligned before `call` instruction.

---

# Chunk 3 — Canonical ASM Interface Table

## 3.1 Complete ASM Interface

### 3.1.1 Device Control Functions

#### `asm_gpu_reset`

```
Symbol:     asm_gpu_reset
Purpose:    Reset VirtIO-GPU device
Inputs:     
    RCX = mmio_base: u64
Outputs:    
    RAX = 0 on success
    RAX = 1 on timeout (100ms)
Clobbers:   RAX, RCX, RDX, R8, R9, R10, R11
Timeout:    100ms maximum
```

#### `asm_gpu_set_status`

```
Symbol:     asm_gpu_set_status
Purpose:    Write device status register
Inputs:     
    RCX = mmio_base: u64
    RDX = status: u8
Outputs:    None
Clobbers:   RAX, RCX, RDX
```

#### `asm_gpu_get_status`

```
Symbol:     asm_gpu_get_status
Purpose:    Read device status register
Inputs:     
    RCX = mmio_base: u64
Outputs:    
    RAX = status: u8
Clobbers:   RAX
```

#### `asm_gpu_read_features`

```
Symbol:     asm_gpu_read_features
Purpose:    Read device feature bits (64-bit)
Inputs:     
    RCX = mmio_base: u64
Outputs:    
    RAX = features: u64
Clobbers:   RAX, RCX, RDX
```

#### `asm_gpu_write_features`

```
Symbol:     asm_gpu_write_features
Purpose:    Write driver-accepted features
Inputs:     
    RCX = mmio_base: u64
    RDX = features: u64
Outputs:    None
Clobbers:   RAX, RCX, RDX
```

### 3.1.2 Command Queue Functions

#### `asm_gpu_submit_cmd`

```
Symbol:     asm_gpu_submit_cmd
Purpose:    Submit command to control queue
Inputs:     
    RCX = *mut CtrlQueueState
    RDX = *const GpuCommand
Outputs:    
    RAX = 0 on success
    RAX = 1 if queue full
Clobbers:   RAX, RCX, RDX, R8, R9, R10, R11
Barriers:   sfence after descriptor, mfence before notify
```

**Rust declaration:**
```rust
extern "win64" {
    fn asm_gpu_submit_cmd(
        ctrl_queue: *mut CtrlQueueState,
        cmd: *const GpuCommand,
    ) -> u32;
}
```

#### `asm_gpu_poll_response`

```
Symbol:     asm_gpu_poll_response
Purpose:    Poll for command response
Inputs:     
    RCX = *mut CtrlQueueState
Outputs:    
    RAX = 0 if no response
    RAX = 1 if response available
Clobbers:   RAX, RCX, RDX, R8, R9, R10, R11
Barriers:   lfence after reading used.idx
```

#### `asm_gpu_notify`

```
Symbol:     asm_gpu_notify
Purpose:    Notify device of pending commands
Inputs:     
    RCX = *mut CtrlQueueState
Outputs:    None
Clobbers:   RAX, RCX, RDX
Barriers:   mfence before MMIO write
```

### 3.1.3 Resource Functions

#### `asm_gpu_create_resource`

```
Symbol:     asm_gpu_create_resource
Purpose:    Create 2D resource
Inputs:     
    RCX = *mut CtrlQueueState
    RDX = *const CreateResourceCmd
Outputs:    
    RAX = 0 on success
    RAX = 1 if queue full
```

#### `asm_gpu_attach_backing`

```
Symbol:     asm_gpu_attach_backing
Purpose:    Attach backing pages to resource
Inputs:     
    RCX = *mut CtrlQueueState
    RDX = *const AttachBackingCmd
Outputs:    
    RAX = 0 on success
    RAX = 1 if queue full
```

#### `asm_gpu_set_scanout`

```
Symbol:     asm_gpu_set_scanout
Purpose:    Set scanout to display resource
Inputs:     
    RCX = *mut CtrlQueueState
    RDX = *const SetScanoutCmd
Outputs:    
    RAX = 0 on success
    RAX = 1 if queue full
```

#### `asm_gpu_transfer_to_host`

```
Symbol:     asm_gpu_transfer_to_host
Purpose:    Transfer resource data to host
Inputs:     
    RCX = *mut CtrlQueueState
    RDX = *const TransferToHostCmd
Outputs:    
    RAX = 0 on success
    RAX = 1 if queue full
```

#### `asm_gpu_resource_flush`

```
Symbol:     asm_gpu_resource_flush
Purpose:    Flush resource to display
Inputs:     
    RCX = *mut CtrlQueueState
    RDX = *const ResourceFlushCmd
Outputs:    
    RAX = 0 on success
    RAX = 1 if queue full
```

### 3.1.4 Fence Functions

#### `asm_gpu_create_fence`

```
Symbol:     asm_gpu_create_fence
Purpose:    Create synchronization fence
Inputs:     
    RCX = *mut CtrlQueueState
    RDX = fence_id: u64
Outputs:    
    RAX = 0 on success
    RAX = 1 if queue full
```

#### `asm_gpu_poll_fence`

```
Symbol:     asm_gpu_poll_fence
Purpose:    Poll fence status (non-blocking)
Inputs:     
    RCX = *mut CtrlQueueState
    RDX = fence_id: u64
Outputs:    
    RAX = 0 if pending
    RAX = 1 if signaled
```

### 3.1.5 3D Functions (virgl)

#### `asm_gpu_ctx_create`

```
Symbol:     asm_gpu_ctx_create
Purpose:    Create 3D rendering context
Inputs:     
    RCX = *mut CtrlQueueState
    RDX = ctx_id: u32
Outputs:    
    RAX = 0 on success
    RAX = 1 on error
```

#### `asm_gpu_submit_3d`

```
Symbol:     asm_gpu_submit_3d
Purpose:    Submit 3D command buffer
Inputs:     
    RCX = *mut CtrlQueueState
    RDX = *const Cmd3D
    R8  = length: u32
Outputs:    
    RAX = 0 on success
    RAX = 1 if queue full
```

## 3.2 Shared Data Structures

### 3.2.1 CtrlQueueState

```rust
#[repr(C)]
pub struct CtrlQueueState {
    pub desc_base: u64,
    pub avail_base: u64,
    pub used_base: u64,
    pub queue_size: u16,
    pub queue_index: u16,
    pub _pad: u32,
    pub notify_addr: u64,
    pub last_used_idx: u16,
    pub next_avail_idx: u16,
    pub _pad2: u32,
    pub cmd_buffer_cpu: u64,
    pub cmd_buffer_bus: u64,
    pub resp_buffer_cpu: u64,
    pub resp_buffer_bus: u64,
}
```

### 3.2.2 GpuCommand

```rust
#[repr(C)]
pub struct GpuCommand {
    pub hdr: GpuCtrlHdr,
    pub data: [u8; 256],  // Command-specific data
}

#[repr(C)]
pub struct GpuCtrlHdr {
    pub cmd_type: u32,
    pub flags: u32,
    pub fence_id: u64,
    pub ctx_id: u32,
    pub ring_idx: u8,
    pub padding: [u8; 3],
}
```

### 3.2.3 Command Types

```rust
#[repr(u32)]
pub enum GpuCmdType {
    GetDisplayInfo = 0x0100,
    ResourceCreate2D = 0x0101,
    ResourceUnref = 0x0102,
    SetScanout = 0x0103,
    ResourceFlush = 0x0104,
    TransferToHost2D = 0x0105,
    AttachBacking = 0x0106,
    DetachBacking = 0x0107,
    GetCapsetInfo = 0x0108,
    GetCapset = 0x0109,
    GetEdid = 0x010a,
    // 3D commands
    CtxCreate = 0x0200,
    CtxDestroy = 0x0201,
    CtxAttachResource = 0x0202,
    CtxDetachResource = 0x0203,
    ResourceCreate3D = 0x0204,
    TransferToHost3D = 0x0205,
    TransferFromHost3D = 0x0206,
    Submit3D = 0x0207,
}
```

---

# Chunk 4 — Ownership & Lifetime Semantics

## 4.1 GPU Resource Ownership Model

```
              ┌─────────┐
              │  FREE   │
              └────┬────┘
                   │ create_resource()
                   ▼
          ┌──────────────────┐
          │   DRIVER-OWNED   │  Rust may upload/modify
          └────────┬─────────┘
                   │ submit_command()
                   ▼
          ┌──────────────────┐
          │   DEVICE-OWNED   │  GPU processing
          └────────┬─────────┘
                   │ fence_signaled()
                   ▼
          ┌──────────────────┐
          │   DRIVER-OWNED   │
          └────────┬─────────┘
                   │ destroy_resource()
                   ▼
              ┌─────────┐
              │  FREE   │
              └─────────┘
```

## 4.2 Ownership Transfer Rules

### Rule OWN-1: Command Submission

```
PRECONDITION:
    Resource is DRIVER-OWNED
    Command is properly initialized
    
ACTION:
    Call asm_gpu_submit_cmd()
    
POSTCONDITION (success):
    Resource is DEVICE-OWNED until fence signals
```

### Rule OWN-2: Fence Completion

```
PRECONDITION:
    Resource is DEVICE-OWNED
    Fence was attached to command
    
ACTION:
    asm_gpu_poll_fence() returns 1 (signaled)
    
POSTCONDITION:
    Resource is DRIVER-OWNED
```

---

# Chunk 5 — Memory Ordering & DMA Visibility

## 5.1 Barrier Requirements

### Command Submit Sequence

| Operation | Before Barrier | After Barrier |
|-----------|----------------|---------------|
| Write descriptor | — | sfence |
| Write avail.ring | — | sfence |
| Write avail.idx | sfence | mfence |
| Write notify | mfence | — |

### Response Poll Sequence

| Operation | Before Barrier | After Barrier |
|-----------|----------------|---------------|
| Read used.idx | — | lfence |
| Read used.ring | lfence | lfence |
| Read response | lfence | — |

---

# Chunk 6 — Time & Progress Guarantees

## 6.1 Function Latency Bounds

| Function | Max Latency | May Block |
|----------|-------------|-----------|
| `asm_gpu_submit_cmd` | 200 cycles | No |
| `asm_gpu_poll_response` | 100 cycles | No |
| `asm_gpu_poll_fence` | 100 cycles | No |
| `asm_gpu_notify` | 500 cycles | No |
| `asm_gpu_reset` | 100ms | Yes (bounded) |

## 6.2 Frame Budget

```
Target frame time: 16.6ms (60 FPS)
Maximum jitter: 2ms

Budget allocation:
  - Command build:    2ms
  - Submit + notify:  1ms
  - GPU execution:   10ms
  - Fence wait:       2ms
  - Buffer swap:      1ms
  - Margin:         0.6ms
```

---

# Chunk 7 — Error Semantics

## 7.1 Error Return Convention

| Return Value | Meaning |
|--------------|---------|
| 0 | Success |
| 1 | Retryable error (queue full) |
| 0xFFFFFFFF | Sentinel for "no result" |

## 7.2 Error Handling

### Queue Full (Retryable)

```rust
match unsafe { asm_gpu_submit_cmd(queue, &cmd) } {
    0 => { /* success */ }
    1 => {
        // Queue full - collect responses and retry
        collect_responses();
        // Try again next frame
    }
    _ => unreachable!()
}
```

### Device Reset Timeout (Fatal)

```rust
if unsafe { asm_gpu_reset(mmio_base) } != 0 {
    // Device unresponsive - fatal error
    fatal_error("GPU reset timeout");
}
```

---

# Chunk 8 — Safety & Undefined Behavior

## 8.1 UB Conditions

| Violation | Consequence |
|-----------|-------------|
| Accessing DEVICE-OWNED resource | Data corruption |
| Missing barriers | GPU sees stale data |
| Wrong fence ID | Incorrect synchronization |
| Queue overflow | Command loss |
| Invalid command | Device error |

## 8.2 Required `unsafe` Documentation

```rust
// SAFETY:
// - ctrl_queue points to valid, initialized CtrlQueueState
// - cmd points to valid GpuCommand with correct type
// - Resource referenced by command is DRIVER-OWNED
// - Fence ID is unique and not reused
let result = unsafe { 
    asm_gpu_submit_cmd(ctrl_queue, &cmd) 
};
```

---

# Chunk 9 — Contract Freeze Declaration

```
╔═══════════════════════════════════════════════════════════════════════════════╗
║                                                                               ║
║                    CONTRACT FREEZE DECLARATION                                ║
║                                                                               ║
║  Document:    DISPLAY_ASM_RUST_ABI_CONTRACT.md                               ║
║  Version:     1.0                                                             ║
║  Date:        2026-01-12                                                      ║
║  Status:      FROZEN                                                          ║
║                                                                               ║
║  This document represents the frozen v1.0 ABI contract between ASM and       ║
║  Rust for the MorpheusX post-ExitBootServices display runtime.               ║
║                                                                               ║
╚═══════════════════════════════════════════════════════════════════════════════╝
```

---

*End of Contract Document*
