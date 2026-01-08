# MorpheusX Single-Core Bare-Metal Network Stack Redesign

## Document Status
- **Version**: 0.1 (Draft)
- **Date**: 2026-01-08
- **Scope**: Research & Architecture Design (No Implementation)

---

## Section 1: Problem Statement

### 1.1 The Fundamental Deadlock

The current MorpheusX network stack has a **fatal architectural flaw**: it attempts to use blocking patterns in a single-threaded, cooperative environment without a scheduler.

**Current TX Path (Broken)**:
```
smoltcp.poll()
  → wants to send DHCP DISCOVER
  → calls DeviceAdapter::transmit()
  → calls TxToken::consume()
  → calls VirtioNetDevice::transmit()
  → BLOCKS: loop { poll_transmit(); tsc_delay_us(10); }
  → Waits for VirtIO device to process packet
  → BUT: Device completion may require US to poll RX
  → DEADLOCK: We never return to poll RX
```

**Why This Happens**:
1. VirtIO is designed for interrupt-driven operation
2. Guest submits buffer → hypervisor processes → hypervisor signals completion
3. Without interrupts, we must poll for completion
4. But polling inside `transmit()` blocks the entire system
5. No other work can happen while we spin

### 1.2 The UEFI Interference Problem

Before `ExitBootServices()`, UEFI firmware:
- Maintains its own memory pools and DMA mappings
- May intercept PCI/MMIO accesses
- Handles hardware interrupts
- Can move memory regions unpredictably

**Consequence**: Any NIC driver running while UEFI is active risks:
- Conflicting DMA mappings (double-mapped buffers)
- Race conditions on device registers
- Unexpected device state changes
- Memory corruption

**Solution**: Full device control requires `ExitBootServices()` FIRST.

### 1.3 The Rust Compiler Problem

The `virtio-drivers` Rust crate and compiler optimizations introduce:
- **Instruction reordering**: Compiler may reorder MMIO accesses
- **Hidden temporaries**: Stack spills at unpredictable times
- **Timing variations**: Different optimization levels = different timing
- **Opaque memory barriers**: `volatile` isn't sufficient for all cases

**For deterministic NIC operation**: Critical paths must be ASM.

### 1.4 Current Blocking Violations

| Location | Pattern | Severity |
|----------|---------|----------|
| `virtio.rs:322` | `loop { poll_transmit(); delay(); }` | CRITICAL |
| `native.rs:202` | `while !has_ip() { poll(); delay(); }` | CRITICAL |
| `native.rs:261` | DNS resolution loop | CRITICAL |
| `native.rs:352` | TCP connect wait | CRITICAL |
| `native.rs:380` | `send_all()` loop | CRITICAL |
| `native.rs:407` | `recv()` loop | CRITICAL |
| `init.rs:205` | DHCP wait loop | HIGH |
| `pci.rs:252` | `tsc_delay_us()` spin | CRITICAL (root cause) |

**Root Cause**: `tsc_delay_us()` is the blocking primitive enabling all violations.

---

## Section 2: Design Constraints

### 2.1 Hardware Constraints

| Constraint | Description |
|------------|-------------|
| Single CPU core | Initial implementation targets one core only |
| No preemption | No OS scheduler, no timer interrupts |
| No FPU | `no_std`, soft-float only |
| Identity-mapped memory | Physical == Virtual addresses |
| Polling-only | No interrupt handlers installed |

### 2.2 Execution Model Constraints

| Constraint | Description |
|------------|-------------|
| All functions must return | No function may block indefinitely |
| Bounded execution time | Every call completes in finite cycles |
| Cooperative scheduling | Progress happens between calls, not within |
| Explicit state machines | All multi-step operations use state enums |

### 2.3 UEFI Lifecycle Constraints

| Phase | What's Available | What's Forbidden |
|-------|------------------|------------------|
| Pre-ExitBootServices | Memory allocation, console, file I/O | Full NIC control, DMA ownership |
| Post-ExitBootServices | Raw hardware access, identity map | Any UEFI call except Runtime Services |

**Critical Invariant**: NIC driver initialization MUST occur AFTER `ExitBootServices()`.

### 2.4 Timing Constraints

| Constraint | Description |
|------------|-------------|
| No OS timers | No `sleep()`, no scheduler tick |
| TSC-relative only | All timing via `rdtsc` instruction |
| Polling budgets | Each subsystem gets cycle allocation |
| Deterministic delays | ASM `nop` sequences for precise timing |

---

## Section 3: Architecture Overview

### 3.1 Layer Stack

```
┌─────────────────────────────────────────────────────────────────┐
│                    Application Layer (Rust)                     │
│         HTTP client, download manager, boot orchestrator        │
│                  - All non-blocking state machines -            │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Protocol Layer (smoltcp)                     │
│            TCP, UDP, ICMP, DHCP, DNS, ARP, IPv4                │
│              - Architecture-agnostic, no_std -                  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                   Device Adapter (Rust)                         │
│     Implements smoltcp Device trait, wraps ASM primitives      │
│        - Translates poll() → asm_poll_rx/asm_poll_tx -         │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    ASM Driver Layer (x86_64)                    │
│      Deterministic NIC control: virtqueue, DMA, MMIO           │
│                  - Full temporal control -                      │
├─────────────────────────────────────────────────────────────────┤
│  Exports:                                                       │
│    asm_nic_init(base_addr) → status                            │
│    asm_poll_rx(buf_ptr, buf_len) → bytes_read | 0              │
│    asm_poll_tx(buf_ptr, buf_len) → success | failure           │
│    asm_get_mac(out_ptr) → void                                 │
│    asm_read_tsc() → u64                                        │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Hardware (VirtIO/Intel/etc)                  │
│                   PCI MMIO, DMA rings, MAC                      │
└─────────────────────────────────────────────────────────────────┘
```

### 3.2 The Main Loop (Single Entry Point)

All network activity flows through ONE loop. No nested polling.

```
┌──────────────────────────────────────────────────────────────┐
│                     MAIN POLL LOOP                           │
│                                                              │
│  loop {                                                      │
│      // Phase 1: RX polling (fixed cycle budget)            │
│      for _ in 0..RX_BUDGET {                                │
│          if let Some(pkt) = asm_poll_rx() {                 │
│              rx_queue.push(pkt);                            │
│          }                                                   │
│      }                                                       │
│                                                              │
│      // Phase 2: Protocol processing                         │
│      let now = asm_read_tsc();                              │
│      smoltcp_iface.poll(now, &mut device, &mut sockets);    │
│                                                              │
│      // Phase 3: TX drain (from smoltcp's queue)            │
│      for _ in 0..TX_BUDGET {                                │
│          if let Some(pkt) = tx_queue.pop() {                │
│              asm_poll_tx(pkt);                              │
│          }                                                   │
│      }                                                       │
│                                                              │
│      // Phase 4: Application state machine step             │
│      app_state.step();                                      │
│                                                              │
│      // Phase 5: TX completion collection                    │
│      asm_collect_tx_completions();                          │
│  }                                                           │
└──────────────────────────────────────────────────────────────┘
```

### 3.3 Key Invariants

1. **No function may loop waiting for external state**
   - Loops bounded by input size only
   - External conditions checked, not waited for

2. **TX is fire-and-forget**
   - Submit buffer, return immediately
   - Completion collected opportunistically

3. **RX is non-blocking poll**
   - Returns `Some(packet)` or `None`
   - Never waits

4. **Time is observation, not control**
   - TSC checked for timeout decisions
   - Never used to delay execution

5. **State machines, not loops**
   - Multi-step operations encoded as enum states
   - One step per main loop iteration

---

## Section 4: ASM Driver Interface Specification

### 4.1 Why ASM for NIC Drivers?

| Rust Problem | ASM Solution |
|--------------|--------------|
| Compiler may reorder MMIO writes | Explicit instruction ordering |
| Hidden temporaries on stack | Full register control |
| Optimization-dependent timing | Exact cycle counts |
| `volatile` not always sufficient | Direct hardware access |
| Implicit memory barriers | Explicit `mfence`/`sfence` |

**Critical Insight**: For VirtIO virtqueues, the order of:
1. Write descriptor to available ring
2. Memory barrier
3. Write available index
4. Notify device (MMIO write)

...must be **exactly** as specified. Rust compiler can break this.

### 4.2 Complete ASM Responsibilities Post-ExitBootServices

After ExitBootServices(), the ASM layer owns **everything** below smoltcp:

```
┌─────────────────────────────────────────────────────────────────┐
│                    WHAT ASM MUST HANDLE                         │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  1. DMA MEMORY MANAGEMENT                                       │
│     - Pre-allocated region (from UEFI phase)                   │
│     - Buffer allocation within region                          │
│     - Physical address calculation (identity map)              │
│     - Alignment enforcement (page/cache-line)                  │
│                                                                 │
│  2. PHY INITIALIZATION (for real NICs, not VirtIO)             │
│     - MDIO/MDC bus access                                      │
│     - Auto-negotiation or forced speed/duplex                  │
│     - Link status detection                                    │
│     - PHY reset sequences                                      │
│                                                                 │
│  3. MAC INITIALIZATION                                          │
│     - Register reset sequence                                  │
│     - MAC address programming                                  │
│     - Interrupt masking (disable all)                          │
│     - Feature negotiation                                      │
│                                                                 │
│  4. PACKET QUEUE MANAGEMENT                                     │
│     - Descriptor ring setup (RX + TX)                          │
│     - Buffer submission to hardware                            │
│     - Completion detection (used ring polling)                 │
│     - Ring wrap-around handling                                │
│                                                                 │
│  5. TIMING-CRITICAL OPERATIONS                                  │
│     - Device notification (doorbell writes)                    │
│     - Memory barriers between operations                       │
│     - Hardware delay sequences                                 │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 4.3 DMA Memory Architecture

**Critical**: DMA region must be allocated BEFORE ExitBootServices, then managed by ASM.

```
┌─────────────────────────────────────────────────────────────────┐
│                   DMA REGION LAYOUT (2MB)                       │
│            Allocated by UEFI, Managed by ASM                    │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │ 0x000000: DMA Control Block (4KB)                         │  │
│  │   - free_bitmap[256]   (tracks 64KB chunks)              │  │
│  │   - next_free_index                                       │  │
│  │   - total_allocated                                       │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                 │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │ 0x001000: VirtIO Structures (64KB)                        │  │
│  │   - RX Descriptors (16 × 16B = 256B)                     │  │
│  │   - RX Available Ring (6B + 16 × 2B = 38B)               │  │
│  │   - RX Used Ring (6B + 16 × 8B = 134B)                   │  │
│  │   - TX Descriptors (16 × 16B = 256B)                     │  │
│  │   - TX Available Ring (38B)                               │  │
│  │   - TX Used Ring (134B)                                   │  │
│  │   - [padding to 4KB alignment]                           │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                 │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │ 0x011000: RX Packet Buffers (512KB)                       │  │
│  │   - 256 × 2KB buffers                                     │  │
│  │   - Each buffer: VirtIO header (12B) + packet (1514B)    │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                 │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │ 0x091000: TX Packet Buffers (512KB)                       │  │
│  │   - 256 × 2KB buffers                                     │  │
│  │   - Each buffer: VirtIO header (12B) + packet (1514B)    │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                 │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │ 0x111000: Scratch / Future Use (~900KB)                   │  │
│  │   - Additional buffers if needed                         │  │
│  │   - Intel/Realtek descriptor rings                       │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 4.4 ASM DMA Allocator

```nasm
; =============================================================================
; DMA Buffer Allocator (ASM-managed, fixed pool)
; =============================================================================

section .data
    dma_base:       dq 0        ; Set by asm_dma_init
    dma_size:       dq 0
    rx_buf_base:    dq 0        ; Start of RX buffer region
    tx_buf_base:    dq 0        ; Start of TX buffer region
    rx_buf_bitmap:  times 32 db 0   ; 256 bits = 256 buffers
    tx_buf_bitmap:  times 32 db 0

section .text

; -----------------------------------------------------------------------------
; asm_dma_init - Initialize DMA region (called once after EBS)
;
; Input:
;   RCX: DMA region base (physical = virtual, identity mapped)
;   RDX: DMA region size (must be >= 2MB)
;
; Output:
;   RAX: 0 = success, 1 = region too small
; -----------------------------------------------------------------------------
global asm_dma_init
asm_dma_init:
    ; Verify minimum size
    cmp rdx, 0x200000           ; 2MB minimum
    jb .too_small
    
    ; Store base and size
    mov [dma_base], rcx
    mov [dma_size], rdx
    
    ; Calculate buffer region bases
    lea rax, [rcx + 0x11000]    ; RX buffers at offset 0x11000
    mov [rx_buf_base], rax
    lea rax, [rcx + 0x91000]    ; TX buffers at offset 0x91000
    mov [tx_buf_base], rax
    
    ; Clear bitmaps (all buffers free)
    xor eax, eax
    mov rdi, rx_buf_bitmap
    mov rcx, 32
    rep stosb
    mov rdi, tx_buf_bitmap
    mov rcx, 32
    rep stosb
    
    xor eax, eax                ; Return success
    ret
    
.too_small:
    mov eax, 1
    ret

; -----------------------------------------------------------------------------
; asm_alloc_rx_buffer - Allocate RX buffer from pool
;
; Output:
;   RAX: Buffer physical address (0 if none available)
;   RDX: Buffer index (for later free)
; -----------------------------------------------------------------------------
global asm_alloc_rx_buffer
asm_alloc_rx_buffer:
    ; Find first free bit in rx_buf_bitmap
    mov rdi, rx_buf_bitmap
    mov rcx, 256                ; 256 buffers
    xor rdx, rdx                ; Buffer index
    
.find_loop:
    mov al, [rdi]
    cmp al, 0xFF                ; All 8 bits set?
    jne .found_byte
    inc rdi
    add rdx, 8
    cmp rdx, 256
    jb .find_loop
    
    ; No free buffers
    xor eax, eax
    ret
    
.found_byte:
    ; Find first zero bit in AL
    xor ecx, ecx
.find_bit:
    bt eax, ecx
    jnc .found_bit
    inc ecx
    cmp ecx, 8
    jb .find_bit
    
.found_bit:
    ; Set bit (mark allocated)
    bts dword [rdi], ecx
    
    ; Calculate buffer index
    add rdx, rcx
    
    ; Calculate buffer address: base + (index * 2048)
    mov rax, rdx
    shl rax, 11                 ; × 2048
    add rax, [rx_buf_base]
    
    ret
```

### 4.5 PHY Initialization (For Real NICs)

VirtIO doesn't have a PHY, but Intel/Realtek do:

```nasm
; =============================================================================
; PHY Layer (Intel e1000 example)
; =============================================================================

; PHY registers (accessed via MDIO)
PHY_CTRL        equ 0x00        ; Control register
PHY_STATUS      equ 0x01        ; Status register  
PHY_ID1         equ 0x02        ; PHY ID high
PHY_ID2         equ 0x03        ; PHY ID low
PHY_AUTONEG_ADV equ 0x04        ; Auto-neg advertisement
PHY_LINK_PART   equ 0x05        ; Link partner ability

; Intel NIC MDIC register (for PHY access)
E1000_MDIC      equ 0x0020
MDIC_DATA_MASK  equ 0x0000FFFF
MDIC_REG_SHIFT  equ 16
MDIC_PHY_SHIFT  equ 21
MDIC_OP_READ    equ 0x08000000
MDIC_OP_WRITE   equ 0x04000000
MDIC_READY      equ 0x10000000
MDIC_ERROR      equ 0x40000000

; -----------------------------------------------------------------------------
; asm_phy_read - Read PHY register via MDIO
;
; Input:
;   RCX: NIC MMIO base
;   DL:  PHY register address
;
; Output:
;   AX:  Register value (0xFFFF on error)
; -----------------------------------------------------------------------------
global asm_phy_read
asm_phy_read:
    push rbx
    
    ; Build MDIC command: read, PHY 1, register DL
    movzx eax, dl
    shl eax, MDIC_REG_SHIFT
    or eax, (1 << MDIC_PHY_SHIFT)   ; PHY address 1
    or eax, MDIC_OP_READ
    
    ; Write to MDIC register
    mov [rcx + E1000_MDIC], eax
    
    ; Wait for ready (with timeout)
    mov ebx, 10000              ; Timeout counter
.wait_ready:
    mov eax, [rcx + E1000_MDIC]
    test eax, MDIC_READY
    jnz .ready
    dec ebx
    jnz .wait_ready
    
    ; Timeout - return error
    mov ax, 0xFFFF
    pop rbx
    ret
    
.ready:
    ; Check for error
    test eax, MDIC_ERROR
    jnz .error
    
    ; Extract data
    and eax, MDIC_DATA_MASK
    pop rbx
    ret
    
.error:
    mov ax, 0xFFFF
    pop rbx
    ret

; -----------------------------------------------------------------------------
; asm_phy_init - Initialize PHY, auto-negotiate link
;
; Input:
;   RCX: NIC MMIO base
;
; Output:
;   RAX: 0 = success, link up
;        1 = timeout, no link
;        2 = PHY error
; -----------------------------------------------------------------------------
global asm_phy_init
asm_phy_init:
    push rbx
    push r12
    mov r12, rcx                ; Save MMIO base
    
    ; Reset PHY
    mov dl, PHY_CTRL
    call asm_phy_read
    or ax, 0x8000               ; Set reset bit
    mov dl, PHY_CTRL
    ; (would call asm_phy_write here)
    
    ; Wait for reset complete (~500ms worst case)
    mov ebx, 50                 ; 50 × 10ms = 500ms
.wait_reset:
    ; Delay 10ms
    mov ecx, 25000000           ; ~10ms at 2.5GHz
.delay:
    dec ecx
    jnz .delay
    
    mov rcx, r12
    mov dl, PHY_CTRL
    call asm_phy_read
    test ax, 0x8000             ; Reset bit cleared?
    jz .reset_done
    dec ebx
    jnz .wait_reset
    
    ; Reset timeout
    mov eax, 2
    jmp .exit
    
.reset_done:
    ; Enable auto-negotiation
    mov rcx, r12
    mov dl, PHY_CTRL
    call asm_phy_read
    or ax, 0x1200               ; Auto-neg enable + restart
    ; (write back)
    
    ; Wait for link (up to 5 seconds)
    mov ebx, 500                ; 500 × 10ms = 5s
.wait_link:
    mov ecx, 25000000
.delay2:
    dec ecx
    jnz .delay2
    
    mov rcx, r12
    mov dl, PHY_STATUS
    call asm_phy_read
    test ax, 0x0004             ; Link status bit
    jnz .link_up
    dec ebx
    jnz .wait_link
    
    ; No link
    mov eax, 1
    jmp .exit
    
.link_up:
    xor eax, eax                ; Success
    
.exit:
    pop r12
    pop rbx
    ret
```

### 4.6 Packet Queue Ring Management (VirtIO)

```nasm
; =============================================================================
; VirtIO Virtqueue Management
; =============================================================================

; Virtqueue state (per-queue)
struc VIRTQ_STATE
    .desc_addr:     resq 1      ; Descriptor table physical address
    .avail_addr:    resq 1      ; Available ring physical address
    .used_addr:     resq 1      ; Used ring physical address
    .num_entries:   resd 1      ; Queue size (power of 2)
    .free_head:     resw 1      ; First free descriptor
    .num_free:      resw 1      ; Number of free descriptors
    .last_used:     resw 1      ; Last seen used index
    .padding:       resw 1
endstruc

; Descriptor structure
struc VIRTQ_DESC
    .addr:          resq 1      ; Buffer physical address
    .len:           resd 1      ; Buffer length
    .flags:         resw 1      ; NEXT, WRITE, INDIRECT
    .next:          resw 1      ; Next descriptor (if NEXT set)
endstruc

VIRTQ_DESC_F_NEXT     equ 1
VIRTQ_DESC_F_WRITE    equ 2     ; Buffer is write-only (device writes)

section .data
    align 64
    rx_queue:   times VIRTQ_STATE_size db 0
    tx_queue:   times VIRTQ_STATE_size db 0

section .text

; -----------------------------------------------------------------------------
; asm_virtq_init - Initialize a virtqueue
;
; Input:
;   RCX: Queue state pointer
;   RDX: Descriptor ring physical address
;   R8:  Available ring physical address
;   R9:  Used ring physical address
;   R10D: Number of entries
; -----------------------------------------------------------------------------
global asm_virtq_init
asm_virtq_init:
    mov [rcx + VIRTQ_STATE.desc_addr], rdx
    mov [rcx + VIRTQ_STATE.avail_addr], r8
    mov [rcx + VIRTQ_STATE.used_addr], r9
    mov [rcx + VIRTQ_STATE.num_entries], r10d
    
    ; Initialize free list (chain all descriptors)
    xor eax, eax
    mov [rcx + VIRTQ_STATE.free_head], ax
    mov ax, r10w
    mov [rcx + VIRTQ_STATE.num_free], ax
    xor eax, eax
    mov [rcx + VIRTQ_STATE.last_used], ax
    
    ; Chain descriptors: 0→1→2→...→N-1
    mov rdi, rdx                ; Descriptor table
    xor esi, esi                ; Index
.chain_loop:
    lea eax, [esi + 1]          ; Next index
    mov [rdi + VIRTQ_DESC.next], ax
    add rdi, VIRTQ_DESC_size
    inc esi
    cmp esi, r10d
    jb .chain_loop
    
    ret

; -----------------------------------------------------------------------------
; asm_virtq_add_buf - Add buffer to virtqueue (for TX or RX submission)
;
; Input:
;   RCX: Queue state pointer
;   RDX: Buffer physical address
;   R8D: Buffer length
;   R9D: Flags (WRITE for RX buffers)
;
; Output:
;   RAX: Descriptor index used (0xFFFF if queue full)
; -----------------------------------------------------------------------------
global asm_virtq_add_buf
asm_virtq_add_buf:
    ; Check if queue has free descriptors
    movzx eax, word [rcx + VIRTQ_STATE.num_free]
    test eax, eax
    jz .full
    
    ; Get free descriptor
    movzx eax, word [rcx + VIRTQ_STATE.free_head]
    push rax                    ; Save descriptor index for return
    
    ; Calculate descriptor address
    mov rdi, [rcx + VIRTQ_STATE.desc_addr]
    imul r10d, eax, VIRTQ_DESC_size
    add rdi, r10
    
    ; Update free list head
    movzx r10d, word [rdi + VIRTQ_DESC.next]
    mov [rcx + VIRTQ_STATE.free_head], r10w
    dec word [rcx + VIRTQ_STATE.num_free]
    
    ; Fill descriptor
    mov [rdi + VIRTQ_DESC.addr], rdx
    mov [rdi + VIRTQ_DESC.len], r8d
    mov [rdi + VIRTQ_DESC.flags], r9w
    
    ; Add to available ring
    mov rdi, [rcx + VIRTQ_STATE.avail_addr]
    movzx r10d, word [rdi + 2]  ; avail->idx
    mov r11d, [rcx + VIRTQ_STATE.num_entries]
    dec r11d                    ; mask = num - 1
    and r10d, r11d              ; ring index = idx & mask
    
    pop rax                     ; Descriptor index
    mov [rdi + 4 + r10*2], ax   ; avail->ring[idx & mask] = desc
    
    ; Memory barrier before updating index
    mfence
    
    ; Increment available index
    inc word [rdi + 2]
    
    ret
    
.full:
    mov eax, 0xFFFF
    ret

; -----------------------------------------------------------------------------
; asm_virtq_get_used - Check for completed buffers in used ring
;
; Input:
;   RCX: Queue state pointer
;
; Output:
;   RAX: Descriptor index (0xFFFF if none)
;   RDX: Length written by device
; -----------------------------------------------------------------------------
global asm_virtq_get_used
asm_virtq_get_used:
    mov rdi, [rcx + VIRTQ_STATE.used_addr]
    
    ; Check if used ring advanced
    movzx eax, word [rdi + 2]   ; used->idx
    movzx edx, word [rcx + VIRTQ_STATE.last_used]
    cmp ax, dx
    je .empty
    
    ; Get used entry
    mov r8d, [rcx + VIRTQ_STATE.num_entries]
    dec r8d                     ; mask
    and edx, r8d                ; ring index
    
    ; used->ring[idx] is 8 bytes: id (4) + len (4)
    mov eax, [rdi + 4 + rdx*8]      ; Descriptor index
    mov edx, [rdi + 4 + rdx*8 + 4]  ; Length
    
    ; Update last_used
    inc word [rcx + VIRTQ_STATE.last_used]
    
    ; Return descriptor to free list
    push rax
    push rdx
    
    ; ... (return to free list logic)
    
    pop rdx
    pop rax
    ret
    
.empty:
    mov eax, 0xFFFF
    xor edx, edx
    ret
```

### 4.7 Complete ASM Interface (All Functions)

```nasm
; =============================================================================
; NIC Driver ASM Interface (x86_64, Microsoft x64 ABI)
; Complete function list post-ExitBootServices
; =============================================================================

; --- DMA Memory Management ---
global asm_dma_init             ; Initialize DMA region
global asm_alloc_rx_buffer      ; Allocate RX buffer
global asm_alloc_tx_buffer      ; Allocate TX buffer
global asm_free_rx_buffer       ; Free RX buffer
global asm_free_tx_buffer       ; Free TX buffer

; --- PHY Layer (Intel/Realtek only) ---
global asm_phy_read             ; Read PHY register
global asm_phy_write            ; Write PHY register
global asm_phy_init             ; Initialize PHY, auto-negotiate
global asm_phy_get_link         ; Check link status

; --- MAC Layer ---
global asm_mac_reset            ; Reset MAC controller
global asm_mac_init             ; Initialize MAC (after PHY)
global asm_mac_set_addr         ; Program MAC address
global asm_mac_enable           ; Enable RX/TX

; --- Virtqueue Management ---
global asm_virtq_init           ; Initialize virtqueue structures
global asm_virtq_add_buf        ; Add buffer to available ring
global asm_virtq_get_used       ; Poll used ring for completions
global asm_virtq_kick           ; Notify device (doorbell)

; --- High-Level Packet Interface ---
global asm_nic_init             ; Full NIC initialization sequence
global asm_poll_rx              ; Non-blocking receive (returns packet or 0)
global asm_poll_tx              ; Non-blocking transmit (fire-and-forget)
global asm_collect_tx           ; Collect TX completions
global asm_get_mac              ; Get MAC address

; --- Timing ---
global asm_read_tsc             ; Read timestamp counter
global asm_delay_cycles         ; Spin for N cycles (for HW delays only)
```

### 4.8 Memory Layout for DMA (Detailed)

```
DMA Region Layout (must be page-aligned, identity-mapped):

┌─────────────────────────────────────────────────────────────┐
│ Offset 0x0000: VirtIO Net Header Template (16 bytes)       │
├─────────────────────────────────────────────────────────────┤
│ Offset 0x0010: RX Descriptor Ring (16 entries × 16 bytes)  │
├─────────────────────────────────────────────────────────────┤
│ Offset 0x0110: RX Available Ring (header + 16 entries)     │
├─────────────────────────────────────────────────────────────┤
│ Offset 0x0140: RX Used Ring (header + 16 entries)          │
├─────────────────────────────────────────────────────────────┤
│ Offset 0x0200: TX Descriptor Ring (16 entries × 16 bytes)  │
├─────────────────────────────────────────────────────────────┤
│ Offset 0x0310: TX Available Ring (header + 16 entries)     │
├─────────────────────────────────────────────────────────────┤
│ Offset 0x0340: TX Used Ring (header + 16 entries)          │
├─────────────────────────────────────────────────────────────┤
│ Offset 0x0400: RX Buffers (16 × 2KB = 32KB)                │
├─────────────────────────────────────────────────────────────┤
│ Offset 0x8400: TX Buffers (16 × 2KB = 32KB)                │
├─────────────────────────────────────────────────────────────┤
│ Total: ~66KB (fits in 17 pages)                            │
└─────────────────────────────────────────────────────────────┘
```

### 4.9 VirtIO Notification Sequence (ASM Critical Section)

```
DMA Region Layout (must be page-aligned, identity-mapped):

┌─────────────────────────────────────────────────────────────┐
│ Offset 0x0000: VirtIO Net Header Template (16 bytes)       │
├─────────────────────────────────────────────────────────────┤
│ Offset 0x0010: RX Descriptor Ring (16 entries × 16 bytes)  │
├─────────────────────────────────────────────────────────────┤
│ Offset 0x0110: RX Available Ring (header + 16 entries)     │
├─────────────────────────────────────────────────────────────┤
│ Offset 0x0140: RX Used Ring (header + 16 entries)          │
├─────────────────────────────────────────────────────────────┤
│ Offset 0x0200: TX Descriptor Ring (16 entries × 16 bytes)  │
├─────────────────────────────────────────────────────────────┤
│ Offset 0x0310: TX Available Ring (header + 16 entries)     │
├─────────────────────────────────────────────────────────────┤
│ Offset 0x0340: TX Used Ring (header + 16 entries)          │
├─────────────────────────────────────────────────────────────┤
│ Offset 0x0400: RX Buffers (16 × 2KB = 32KB)                │
├─────────────────────────────────────────────────────────────┤
│ Offset 0x8400: TX Buffers (16 × 2KB = 32KB)                │
├─────────────────────────────────────────────────────────────┤
│ Total: ~66KB (fits in 17 pages)                            │
└─────────────────────────────────────────────────────────────┘
```

### 4.4 VirtIO Notification Sequence (ASM Critical Section)

```nasm
; Critical TX submit sequence - order MUST NOT change
submit_tx_packet:
    ; Step 1: Write descriptor (already done by caller)
    
    ; Step 2: Memory barrier - ensure descriptor visible before index
    mfence
    
    ; Step 3: Increment available index
    mov ax, [avail_idx]
    inc ax
    mov [avail_idx], ax
    
    ; Step 4: Memory barrier - ensure index visible before notify
    mfence
    
    ; Step 5: Check if notification needed
    mov ax, [used_flags]
    test ax, VIRTQ_USED_F_NO_NOTIFY
    jnz .skip_notify
    
    ; Step 6: Notify device (MMIO write to queue notify register)
    mov eax, TX_QUEUE_INDEX
    mov [notify_reg], eax
    
.skip_notify:
    ret
```

---

## Section 5: Boot Sequence & ExitBootServices Boundary

### 5.1 The Two-Phase Boot Model

```
┌─────────────────────────────────────────────────────────────────┐
│                    PHASE 1: UEFI ACTIVE                         │
│                 (Boot Services Available)                       │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  1. UEFI loads bootloader.efi                                  │
│  2. Bootloader allocates memory for:                           │
│     - Kernel/initrd buffers                                    │
│     - DMA region (page-aligned, reserved)                      │
│     - Stack for post-EBS execution                             │
│  3. Bootloader scans PCI for NICs                              │
│     - Records MMIO base addresses                              │
│     - Records MAC addresses (optional)                         │
│     - Does NOT initialize device drivers                       │
│  4. Bootloader prepares memory map                             │
│  5. Bootloader sets up identity page tables (if needed)        │
│                                                                 │
│  ─────────── POINT OF NO RETURN ───────────                    │
│                                                                 │
│  6. Call ExitBootServices(image_handle, map_key)               │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                   PHASE 2: BARE METAL                           │
│              (Boot Services UNAVAILABLE)                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  7. Initialize serial/debug output (optional)                  │
│  8. Call asm_nic_init(mmio_base, dma_base, dma_size)          │
│     - Device reset, feature negotiation                        │
│     - Virtqueue setup, RX buffer submission                    │
│  9. Create smoltcp Interface with ASM device adapter           │
│ 10. Enter main poll loop:                                      │
│     - DHCP discovery/negotiation                               │
│     - DNS resolution                                           │
│     - TCP connections                                          │
│     - HTTP requests                                            │
│     - ISO download                                             │
│ 11. Prepare kernel boot (load kernel, initrd)                  │
│ 12. Jump to kernel entry point                                 │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 5.2 Pre-ExitBootServices Checklist

**MUST DO before calling ExitBootServices()**:

| Task | Why |
|------|-----|
| Allocate DMA region | No allocator after EBS |
| Record NIC MMIO base | PCI config access may fail after EBS |
| Get memory map | Required for EBS call |
| Set up page tables | If using virtual memory |
| Reserve stack | Current stack may be in reclaimed region |
| Store ACPI tables pointer | May need for hardware discovery |

**MUST NOT DO before calling ExitBootServices()**:

| Action | Why Not |
|--------|---------|
| Initialize NIC driver | UEFI may conflict |
| Submit DMA buffers | UEFI owns device state |
| Write to device MMIO | Undefined behavior |
| Set up virtqueues | Device state unknown |

### 5.3 Post-ExitBootServices Constraints

After `ExitBootServices()` returns successfully:

| Available | Not Available |
|-----------|---------------|
| CPU registers | AllocatePool/FreePool |
| Identity-mapped memory | Console output (GOP) |
| PCI MMIO access | File system access |
| TSC instruction | Timer services |
| Raw port I/O | Memory map changes |
| Pre-allocated DMA | Runtime services (limited) |

### 5.4 Error Recovery

**If ExitBootServices() fails** (map_key mismatch):
1. Get new memory map
2. Try again with new map_key
3. If fails 3 times: fatal error, halt

**If NIC init fails after ExitBootServices()**:
1. No recovery possible (no allocator)
2. Log to serial if available
3. Attempt alternate NIC if discovered
4. If all fail: boot without network

### 5.5 Handoff Data Structure

```rust
/// Data passed from UEFI phase to bare-metal phase
#[repr(C)]
pub struct BootHandoff {
    /// MMIO base address for primary NIC
    pub nic_mmio_base: u64,
    /// DMA region base (page-aligned)
    pub dma_base: u64,
    /// DMA region size in bytes
    pub dma_size: u64,
    /// MAC address (6 bytes, may be zero if unknown)
    pub mac_address: [u8; 6],
    /// NIC type (0=VirtIO, 1=Intel, 2=Realtek, etc.)
    pub nic_type: u16,
    /// ACPI RSDP address (for future use)
    pub acpi_rsdp: u64,
    /// Stack top for post-EBS execution
    pub stack_top: u64,
    /// Framebuffer base (for debug output)
    pub framebuffer_base: u64,
    /// Framebuffer size
    pub framebuffer_size: u64,
}
```

---

## Section 6: smoltcp Integration

### 6.1 The Device Trait Bridge

smoltcp requires implementing its `Device` trait. The key insight:
**smoltcp's `poll()` must never block, and neither can our device**.

```rust
/// ASM-backed network device for smoltcp
pub struct AsmNetDevice {
    /// MMIO base (passed to ASM)
    mmio_base: u64,
    /// Pending RX packet (pre-fetched)
    rx_pending: Option<RxPacket>,
    /// TX queue status
    tx_ready: bool,
}

impl smoltcp::phy::Device for AsmNetDevice {
    type RxToken<'a> = AsmRxToken;
    type TxToken<'a> = AsmTxToken;

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = 1514;
        caps.medium = Medium::Ethernet;
        caps
    }

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        // Pre-poll RX in main loop, check result here
        if let Some(pkt) = self.rx_pending.take() {
            Some((
                AsmRxToken { data: pkt },
                AsmTxToken { device: self },
            ))
        } else {
            None
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        // Check if TX queue has space (cached from last poll)
        if self.tx_ready {
            Some(AsmTxToken { device: self })
        } else {
            None
        }
    }
}
```

### 6.2 Token Implementation

```rust
pub struct AsmRxToken {
    data: RxPacket,
}

impl smoltcp::phy::RxToken for AsmRxToken {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        // Data already in buffer, just call consumer
        f(&mut self.data.buffer[..self.data.len])
    }
}

pub struct AsmTxToken<'a> {
    device: &'a mut AsmNetDevice,
}

impl<'a> smoltcp::phy::TxToken for AsmTxToken<'a> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        // Allocate TX buffer
        let mut buffer = [0u8; 1514];
        let result = f(&mut buffer[..len]);
        
        // Submit via ASM - NON-BLOCKING
        // Returns immediately, completion tracked separately
        unsafe {
            asm_poll_tx(buffer.as_ptr(), len);
        }
        
        result
    }
}
```

### 6.3 The Critical Difference

**WRONG (current implementation)**:
```rust
fn transmit(&mut self, packet: &[u8]) -> Result<()> {
    let token = self.inner.transmit_begin(&tx_buf)?;
    loop {  // ← BLOCKING LOOP
        if let Some(t) = self.inner.poll_transmit() {
            if t == token { return Ok(()); }
        }
        tsc_delay_us(10);  // ← BUSY WAIT
    }
}
```

**CORRECT (new design)**:
```rust
fn transmit(&mut self, packet: &[u8]) -> Result<()> {
    // Just submit, don't wait
    unsafe { asm_poll_tx(packet.as_ptr(), packet.len()) };
    Ok(())
    // Completion will be collected in main loop's Phase 5
}
```

### 6.4 Timestamp Handling

smoltcp needs timestamps for timeouts. We use TSC-derived milliseconds:

```rust
/// Convert TSC to milliseconds (assuming ~2.5GHz CPU)
fn tsc_to_millis(tsc: u64) -> i64 {
    const TSC_PER_MS: u64 = 2_500_000; // 2.5GHz / 1000
    (tsc / TSC_PER_MS) as i64
}

/// Get current timestamp for smoltcp
fn now() -> Instant {
    let tsc = unsafe { asm_read_tsc() };
    Instant::from_millis(tsc_to_millis(tsc))
}
```

### 6.5 Socket State Machine Integration

smoltcp sockets must be polled repeatedly. The pattern:

```rust
pub enum DhcpState {
    Discovering { start_tsc: u64 },
    Requesting { start_tsc: u64 },
    Bound { ip: Ipv4Addr, lease_start: u64 },
    Failed(DhcpError),
}

impl DhcpState {
    /// Advance state by one step. Returns true if terminal.
    pub fn step(&mut self, iface: &mut Interface, now_tsc: u64) -> bool {
        match self {
            DhcpState::Discovering { start_tsc } => {
                // Check timeout
                if now_tsc - *start_tsc > DHCP_DISCOVER_TIMEOUT_TSC {
                    *self = DhcpState::Failed(DhcpError::DiscoverTimeout);
                    return true;
                }
                
                // Check if DHCP socket got offer
                if let Some(config) = iface.dhcp_poll() {
                    *self = DhcpState::Bound { 
                        ip: config.address, 
                        lease_start: now_tsc 
                    };
                    return true;
                }
                
                false // Still discovering
            }
            DhcpState::Bound { .. } => true,  // Terminal success
            DhcpState::Failed(_) => true,      // Terminal failure
            _ => false,
        }
    }
}
```

---

## Section 7: Timing, Polling Budgets & Determinism

### 7.1 The Polling Budget Model

Each main loop iteration has a **fixed cycle budget**:

```
┌──────────────────────────────────────────────────────────────────┐
│                    MAIN LOOP CYCLE BUDGET                        │
│                   (Target: 1ms per iteration)                    │
├─────────────────────────────┬────────────────────────────────────┤
│ Phase                       │ Budget (cycles @ 2.5GHz)          │
├─────────────────────────────┼────────────────────────────────────┤
│ 1. RX Poll (16 checks)      │ ~50,000 cycles (20µs)             │
│ 2. smoltcp poll()           │ ~500,000 cycles (200µs)           │
│ 3. TX Drain (16 packets)    │ ~100,000 cycles (40µs)            │
│ 4. App state step           │ ~1,000,000 cycles (400µs)         │
│ 5. TX completion collect    │ ~50,000 cycles (20µs)             │
│ 6. Overhead/margin          │ ~800,000 cycles (320µs)           │
├─────────────────────────────┼────────────────────────────────────┤
│ TOTAL                       │ 2,500,000 cycles (1ms)            │
└─────────────────────────────┴────────────────────────────────────┘
```

### 7.2 RX Polling Strategy

```rust
const RX_POLL_BUDGET: usize = 16;  // Max packets per iteration

fn poll_rx_phase(device: &mut AsmNetDevice, rx_queue: &mut RxQueue) {
    for _ in 0..RX_POLL_BUDGET {
        let mut buf = [0u8; 1514];
        let len = unsafe { asm_poll_rx(buf.as_mut_ptr(), buf.len()) };
        
        if len == 0 {
            break; // No more packets
        }
        
        rx_queue.push(RxPacket {
            buffer: buf,
            len: len as usize,
        });
    }
}
```

**Key property**: Loop bounded by `RX_POLL_BUDGET`, not by packet availability.

### 7.3 TX Drain Strategy

```rust
const TX_DRAIN_BUDGET: usize = 16;  // Max packets per iteration

fn drain_tx_phase(tx_queue: &mut TxQueue) {
    for _ in 0..TX_DRAIN_BUDGET {
        if let Some(pkt) = tx_queue.pop() {
            let result = unsafe { 
                asm_poll_tx(pkt.buffer.as_ptr(), pkt.len) 
            };
            
            if result != 0 {
                // Queue full, put packet back
                tx_queue.push_front(pkt);
                break;
            }
        } else {
            break; // Queue empty
        }
    }
}
```

### 7.4 Timeout Calculation

All timeouts expressed in TSC cycles, not wall time:

```rust
/// Timeout constants (assuming 2.5GHz TSC)
pub mod timeouts {
    pub const TSC_PER_US: u64 = 2_500;
    pub const TSC_PER_MS: u64 = 2_500_000;
    pub const TSC_PER_SEC: u64 = 2_500_000_000;
    
    pub const DHCP_DISCOVER: u64 = 5 * TSC_PER_SEC;   // 5 seconds
    pub const DHCP_REQUEST: u64 = 3 * TSC_PER_SEC;    // 3 seconds
    pub const TCP_CONNECT: u64 = 30 * TSC_PER_SEC;    // 30 seconds
    pub const TCP_KEEPALIVE: u64 = 60 * TSC_PER_SEC;  // 60 seconds
    pub const DNS_QUERY: u64 = 5 * TSC_PER_SEC;       // 5 seconds
}
```

### 7.5 Timeout Checking Pattern

```rust
/// Check timeout without blocking
fn is_timed_out(start_tsc: u64, timeout_tsc: u64) -> bool {
    let now = unsafe { asm_read_tsc() };
    now.wrapping_sub(start_tsc) > timeout_tsc
}

// Usage in state machine:
match &mut self.state {
    State::Connecting { start_tsc, .. } => {
        if is_timed_out(*start_tsc, timeouts::TCP_CONNECT) {
            self.state = State::Failed(Error::Timeout);
            return StepResult::Done;
        }
        // Check connection status...
    }
    // ...
}
```

### 7.6 TSC Calibration

At boot, calibrate TSC against known reference:

```rust
/// Calibrate TSC using PIT or HPET (before ExitBootServices)
fn calibrate_tsc() -> u64 {
    // Option 1: Use UEFI Stall() as reference
    let start = unsafe { asm_read_tsc() };
    uefi_stall(1_000_000); // 1 second
    let end = unsafe { asm_read_tsc() };
    
    end - start // TSC ticks per second
}

// Store for later use
static mut TSC_FREQ: u64 = 2_500_000_000; // Default 2.5GHz
```

### 7.7 Determinism Guarantees

| Property | Guarantee | Mechanism |
|----------|-----------|-----------|
| Bounded iteration time | < 2ms per loop | Fixed budgets |
| Bounded RX latency | < 1ms from wire to smoltcp | Priority RX phase |
| Bounded TX latency | < 2ms from smoltcp to wire | Immediate submission |
| No unbounded waits | All loops bounded | Budget constants |
| Predictable timing | ±100µs variance | ASM critical paths |

### 7.8 Anti-Patterns to Avoid

```rust
// ❌ WRONG: Unbounded loop
while !condition {
    do_work();
}

// ✅ CORRECT: Bounded check
for _ in 0..MAX_ITERATIONS {
    if condition { break; }
    do_work();
}
if !condition { return Err(Timeout); }

// ❌ WRONG: Busy wait
while time_elapsed < timeout {
    spin_loop();
}

// ✅ CORRECT: Check and return
if time_elapsed > timeout {
    return Err(Timeout);
}
// Continue with non-blocking work

// ❌ WRONG: Blocking inside callback
fn on_tx_submit(&mut self) {
    while !self.tx_complete() { } // BLOCKS
}

// ✅ CORRECT: State machine
fn step(&mut self) -> StepResult {
    match self.state {
        TxPending { .. } => {
            if self.tx_complete() {
                self.state = TxDone;
            }
            StepResult::Pending
        }
        // ...
    }
}
```

---

## Section 8: Protocol State Machines

### 8.1 The State Machine Principle

Every multi-step operation is a **state machine**, not a loop:

```
WRONG: Loop-based thinking
┌─────────────────────────────────────────┐
│ fn do_http_request() {                  │
│     connect();     // blocks            │
│     send();        // blocks            │
│     recv();        // blocks            │
│     return response;                    │
│ }                                       │
└─────────────────────────────────────────┘

CORRECT: State machine thinking
┌─────────────────────────────────────────┐
│ enum HttpState {                        │
│     Resolving,                          │
│     Connecting,                         │
│     SendingHeaders,                     │
│     SendingBody,                        │
│     ReceivingHeaders,                   │
│     ReceivingBody,                      │
│     Done(Response),                     │
│     Failed(Error),                      │
│ }                                       │
│                                         │
│ fn step() -> bool { /* one step */ }    │
└─────────────────────────────────────────┘
```

### 8.2 HTTP Client State Machine

```rust
pub enum HttpState {
    /// Initial state, need to resolve hostname
    Idle,
    
    /// DNS query in flight
    Resolving {
        host: String,
        query_handle: QueryHandle,
        start_tsc: u64,
    },
    
    /// TCP connection in progress
    Connecting {
        ip: Ipv4Addr,
        port: u16,
        socket: SocketHandle,
        start_tsc: u64,
    },
    
    /// Sending HTTP request headers
    SendingHeaders {
        socket: SocketHandle,
        headers: Vec<u8>,
        sent: usize,
        start_tsc: u64,
    },
    
    /// Sending request body (POST/PUT)
    SendingBody {
        socket: SocketHandle,
        body: Vec<u8>,
        sent: usize,
        start_tsc: u64,
    },
    
    /// Receiving response headers
    ReceivingHeaders {
        socket: SocketHandle,
        buffer: Vec<u8>,
        start_tsc: u64,
    },
    
    /// Receiving response body
    ReceivingBody {
        socket: SocketHandle,
        headers: Headers,
        body: Vec<u8>,
        content_length: Option<usize>,
        start_tsc: u64,
    },
    
    /// Request complete
    Done(Response),
    
    /// Request failed
    Failed(HttpError),
}

impl HttpState {
    /// Advance state machine by one step.
    /// Returns true if reached terminal state.
    pub fn step(
        &mut self,
        iface: &mut NetInterface,
        now_tsc: u64,
    ) -> bool {
        match self {
            HttpState::Idle => false,
            
            HttpState::Resolving { host, query_handle, start_tsc } => {
                // Check timeout
                if now_tsc - *start_tsc > timeouts::DNS_QUERY {
                    *self = HttpState::Failed(HttpError::DnsTimeout);
                    return true;
                }
                
                // Poll DNS
                match iface.get_dns_result(*query_handle) {
                    Ok(Some(ip)) => {
                        *self = HttpState::Connecting {
                            ip,
                            port: 80,
                            socket: iface.tcp_socket().unwrap(),
                            start_tsc: now_tsc,
                        };
                    }
                    Ok(None) => {} // Still resolving
                    Err(e) => {
                        *self = HttpState::Failed(HttpError::DnsFailed);
                        return true;
                    }
                }
                false
            }
            
            HttpState::Connecting { ip, port, socket, start_tsc } => {
                // Check timeout
                if now_tsc - *start_tsc > timeouts::TCP_CONNECT {
                    *self = HttpState::Failed(HttpError::ConnectTimeout);
                    return true;
                }
                
                // Check connection state
                if iface.tcp_is_connected(*socket) {
                    // Prepare headers
                    let headers = format!("GET / HTTP/1.1\r\nHost: {}\r\n\r\n", ip);
                    *self = HttpState::SendingHeaders {
                        socket: *socket,
                        headers: headers.into_bytes(),
                        sent: 0,
                        start_tsc: now_tsc,
                    };
                }
                false
            }
            
            // ... other states follow same pattern
            
            HttpState::Done(_) | HttpState::Failed(_) => true,
        }
    }
}
```

### 8.3 TCP Connection State Machine

```rust
pub enum TcpConnState {
    /// Not connected
    Closed,
    
    /// SYN sent, waiting for SYN-ACK
    SynSent {
        socket: SocketHandle,
        start_tsc: u64,
    },
    
    /// Connection established
    Established {
        socket: SocketHandle,
    },
    
    /// FIN sent, waiting for FIN-ACK
    FinWait {
        socket: SocketHandle,
        start_tsc: u64,
    },
    
    /// Error state
    Error(TcpError),
}
```

### 8.4 DHCP State Machine

```rust
pub enum DhcpState {
    /// Not started
    Idle,
    
    /// DHCPDISCOVER sent, waiting for DHCPOFFER
    Discovering {
        start_tsc: u64,
        retries: u8,
    },
    
    /// DHCPREQUEST sent, waiting for DHCPACK
    Requesting {
        offered_ip: Ipv4Addr,
        server_ip: Ipv4Addr,
        start_tsc: u64,
    },
    
    /// Lease obtained
    Bound {
        ip: Ipv4Addr,
        subnet: Ipv4Addr,
        gateway: Option<Ipv4Addr>,
        dns: Option<Ipv4Addr>,
        lease_start_tsc: u64,
        lease_duration_tsc: u64,
    },
    
    /// Lease renewal in progress
    Renewing {
        current_ip: Ipv4Addr,
        start_tsc: u64,
    },
    
    /// Failed to obtain lease
    Failed(DhcpError),
}
```

### 8.5 Composing State Machines

Higher-level operations compose lower-level state machines:

```rust
pub enum IsoDownloadState {
    /// Starting up
    Init,
    
    /// Waiting for network (DHCP)
    WaitingForNetwork {
        dhcp: DhcpState,
    },
    
    /// Resolving mirror hostname
    ResolvingMirror {
        dns: DnsState,
    },
    
    /// Downloading ISO
    Downloading {
        http: HttpState,
        bytes_received: usize,
        total_size: Option<usize>,
    },
    
    /// Verifying checksum
    Verifying {
        hasher: Sha256State,
    },
    
    /// Complete
    Done {
        iso_ptr: *const u8,
        iso_len: usize,
    },
    
    /// Failed
    Failed(DownloadError),
}

impl IsoDownloadState {
    pub fn step(&mut self, iface: &mut NetInterface, now_tsc: u64) -> bool {
        match self {
            IsoDownloadState::WaitingForNetwork { dhcp } => {
                if dhcp.step(iface, now_tsc) {
                    match dhcp {
                        DhcpState::Bound { .. } => {
                            *self = IsoDownloadState::ResolvingMirror {
                                dns: DnsState::new("mirror.example.com"),
                            };
                        }
                        DhcpState::Failed(e) => {
                            *self = IsoDownloadState::Failed(
                                DownloadError::NetworkFailed
                            );
                            return true;
                        }
                        _ => {}
                    }
                }
                false
            }
            // ... other states
            _ => false,
        }
    }
}
```

### 8.6 State Machine Testing

Each state machine can be tested in isolation:

```rust
#[test]
fn test_dhcp_discovery_timeout() {
    let mut state = DhcpState::Discovering {
        start_tsc: 0,
        retries: 0,
    };
    
    // Simulate time passing without DHCP response
    let timeout_tsc = timeouts::DHCP_DISCOVER + 1;
    let done = state.step(&mut mock_iface(), timeout_tsc);
    
    assert!(done);
    assert!(matches!(state, DhcpState::Failed(DhcpError::Timeout)));
}
```

---

## Section 9: Risks, Limitations & Mitigations

### 9.1 Known Risks

| Risk | Severity | Likelihood | Mitigation |
|------|----------|------------|------------|
| TSC frequency varies across CPUs | Medium | High | Calibrate at boot, store in handoff |
| QEMU VirtIO timing differs from real HW | Low | High | Test on both, use conservative timeouts |
| DMA region too small | High | Medium | Calculate needs upfront, reserve 2MB+ |
| smoltcp bugs in edge cases | Medium | Low | Extensive testing, fallback paths |
| ASM bugs hard to debug | High | Medium | Extensive comments, unit tests |
| Real NICs differ from VirtIO | High | Certain | Abstract via trait, test each driver |

### 9.2 Architectural Limitations

| Limitation | Impact | Future Fix |
|------------|--------|------------|
| Single-core only | Can't parallelize RX/TX | Phase 2: multi-core |
| No interrupt support | Higher latency, more CPU | Phase 3: MSI-X support |
| Polling-only model | CPU-intensive | Multi-core offload |
| VirtIO-only initially | Limited to VMs | Add Intel/Realtek drivers |
| No TLS/HTTPS | Insecure downloads | Add TLS state machine |
| Fixed buffer sizes | Memory waste | Dynamic allocation |

### 9.3 Performance Limitations

| Metric | Expected | Limitation Cause |
|--------|----------|------------------|
| Latency | 1-2ms | Polling budget |
| Throughput | ~500 Mbps | Single-core, no batching |
| CPU usage | 50-80% | Continuous polling |
| Memory | ~2MB DMA | Static allocation |

### 9.4 Edge Cases & Failure Modes

**Network Failures**:
- DHCP server unreachable → Timeout, retry, fail gracefully
- DNS failure → Use hardcoded fallbacks
- TCP RST → Close socket, report error
- Packet corruption → Rely on TCP checksums

**Hardware Failures**:
- NIC not responding → Timeout, report error, halt
- DMA failure → Fatal, no recovery
- PCI access failure → Fatal, no recovery

**Resource Exhaustion**:
- RX queue full → Drop oldest packets
- TX queue full → Backpressure to smoltcp
- Memory exhausted → Fatal (pre-allocated)

### 9.5 Security Considerations

| Threat | Mitigation |
|--------|------------|
| Malformed packets | smoltcp validation |
| DMA attacks | Identity mapping, no IOMMU |
| Buffer overflow | Bounds checking in ASM |
| Timing attacks | Deterministic execution |

**Note**: This design does NOT protect against malicious hypervisor or hardware.

---

## Section 10: Future Expansion Paths

### 10.1 Phase 2: Multi-Core Support

```
┌────────────────────────────────────────────────────────────────┐
│                    MULTI-CORE MODEL                            │
├────────────────────────────────────────────────────────────────┤
│                                                                │
│   Core 0 (BSP)              Core 1 (AP)                       │
│   ┌─────────────┐           ┌─────────────┐                   │
│   │ Main Loop   │           │ NIC Driver  │                   │
│   │ - smoltcp   │◄─────────►│ - RX poll   │                   │
│   │ - App state │  IPC      │ - TX submit │                   │
│   │ - Timeouts  │  Ring     │ - Completn  │                   │
│   └─────────────┘           └─────────────┘                   │
│                                                                │
│   Benefits:                                                    │
│   - Core 0 never blocked by NIC                               │
│   - Core 1 can poll continuously                              │
│   - Lower latency, higher throughput                          │
│                                                                │
└────────────────────────────────────────────────────────────────┘
```

### 10.2 Phase 3: Interrupt Support

```rust
// Future: MSI-X interrupt handler
#[naked]
unsafe extern "C" fn nic_interrupt_handler() {
    asm!(
        "push rax",
        "push rcx",
        "push rdx",
        // Acknowledge interrupt
        "call asm_nic_ack_interrupt",
        // Signal main loop (set flag)
        "mov byte ptr [INTERRUPT_PENDING], 1",
        // EOI
        "mov al, 0x20",
        "out 0x20, al",
        "pop rdx",
        "pop rcx",
        "pop rax",
        "iretq",
        options(noreturn)
    );
}
```

### 10.3 Phase 4: Additional NIC Drivers

| Driver | Priority | Complexity | Notes |
|--------|----------|------------|-------|
| Intel e1000 | High | Medium | Common in QEMU |
| Intel e1000e | High | Medium | Modern Intel |
| Intel i219 | Medium | Medium | Recent chipsets |
| Realtek 8169 | High | Low | Consumer boards |
| Realtek 8111 | High | Medium | Modern consumer |
| Broadcom TG3 | Low | High | Servers |

### 10.4 Phase 5: GPU/NPU Offload

```
┌────────────────────────────────────────────────────────────────┐
│                    COMPUTE OFFLOAD MODEL                       │
├────────────────────────────────────────────────────────────────┤
│                                                                │
│   CPU                        GPU/NPU                           │
│   ┌─────────────┐           ┌─────────────┐                   │
│   │ Control     │           │ Data Plane  │                   │
│   │ - TCP state │──────────►│ - Checksum  │                   │
│   │ - Timeouts  │           │ - Crypto    │                   │
│   │ - Errors    │◄──────────│ - Compress  │                   │
│   └─────────────┘           └─────────────┘                   │
│                                                                │
│   Benefits:                                                    │
│   - CPU handles control only                                  │
│   - Data path fully offloaded                                 │
│   - Massive parallelism for crypto/compress                   │
│                                                                │
└────────────────────────────────────────────────────────────────┘
```

---

## Section 11: Implementation Estimate

### 11.1 LOC Breakdown

| Component | Estimated LOC | Language |
|-----------|---------------|----------|
| ASM NIC driver (VirtIO) | 400-500 | x86_64 ASM |
| ASM HAL primitives | 100-150 | x86_64 ASM |
| Device adapter (Rust) | 200-250 | Rust |
| State machines (Rust) | 600-800 | Rust |
| Main loop (Rust) | 150-200 | Rust |
| Boot handoff (Rust) | 100-150 | Rust |
| Tests | 300-400 | Rust |
| **Total** | **~2000** | Mixed |

### 11.2 Implementation Order

1. **Week 1**: ASM primitives (`asm_read_tsc`, `asm_poll_rx`, `asm_poll_tx`)
2. **Week 2**: VirtIO virtqueue setup in ASM
3. **Week 3**: Device adapter, smoltcp integration
4. **Week 4**: State machines (DHCP, TCP)
5. **Week 5**: HTTP state machine, testing
6. **Week 6**: Boot handoff, integration
7. **Week 7-8**: Testing, debugging, hardening

### 11.3 Testing Strategy

| Test Type | Coverage | Tools |
|-----------|----------|-------|
| Unit tests | State machines | `cargo test` |
| Integration | Full stack | QEMU + VirtIO |
| Hardware | Real NICs | Physical machines |
| Stress | Edge cases | Fuzzing, packet loss sim |

---

## Section 12: Conclusion

### 12.1 Summary

This design addresses the fundamental deadlock in MorpheusX's network stack by:

1. **Eliminating blocking patterns** through state machines
2. **Separating ASM and Rust responsibilities** for determinism
3. **Respecting the ExitBootServices boundary** for UEFI compatibility
4. **Using fixed polling budgets** for predictable timing
5. **Designing for future expansion** to multi-core and interrupts

### 12.2 Key Invariants (Enforced)

1. No function loops waiting for external state
2. TX is fire-and-forget, completion collected separately
3. RX is non-blocking poll only
4. All timeouts are TSC-relative observations
5. Main loop is the ONLY entry point for network activity
6. ASM handles all timing-critical NIC operations
7. Rust handles all protocol logic via smoltcp

### 12.3 Success Criteria

- [ ] DHCP completes within 10 seconds
- [ ] TCP connection establishes within 5 seconds
- [ ] HTTP request completes without blocking
- [ ] Main loop iteration < 2ms guaranteed
- [ ] No `tsc_delay_us()` calls remain in codebase
- [ ] All blocking loops replaced with state machines

### 12.4 Next Steps

1. Review this document with stakeholders
2. Prototype ASM VirtIO driver
3. Integrate with existing smoltcp wrapper
4. Test in QEMU
5. Iterate based on findings

---

**Document End**

*Author: MorpheusX Architecture Team*
*Status: Draft for Review*
