# Research Prompt: Polling Loop Audit and Interrupt-Driven Refactoring Assessment

## Objective
Perform a comprehensive audit of all tight polling loops in the MorpheusX exokernel codebase. Identify each polling pattern, assess whether it is necessary, and determine if/how it should be refactored to interrupt-driven behavior to reduce CPU burn and improve kernel responsiveness.

## Context
MorpheusX is a bare-metal x86-64 exokernel written in Rust. It currently relies on periodic polling in several subsystems:
- **xHCI USB controller**: Events polled via event ring
- **Scheduler**: Preemptive 100 Hz tick
- **Network/block drivers**: Polling-based completion detection
- **Device discovery**: PCI/hardware probing

The goal is to transition from polling-based architecture to interrupt-driven design where feasible.

## Search Scope
Examine the entire codebase:
1. `hwinit/src/` - Hardware initialization, scheduler, interrupts
2. `core/src/` - Low-level drivers, filesystem
3. `helix/src/` - Log-structured filesystem, block drivers
4. `network/src/` - Network stack, VirtIO/AHCI
5. `display/src/` - Framebuffer operations
6. `bootloader/src/` - Boot-time device initialization

## What to Find

### 1. Tight Polling Loops
Look for patterns like:
- `loop { ... }` or `while true { ... }` without `yield`/interrupt
- Repeated checking of status registers (device/controller state)
- `spin_until(condition)` or similar timeout loops
- Event ring draining loops that busy-wait
- Scheduler tick implementation (100 Hz polling check)
- Completion queue polling without sleep

**For each loop found:**
- File path and line number
- Loop structure (what condition is checked)
- Loop body (what work is done per iteration)
- Frequency estimate (if measurable)
- CPU impact (always running, triggered, conditional)

### 2. Interrupt Handlers
Identify existing interrupt infrastructure:
- IDT setup (where? how many handlers?)
- Interrupt types handled (timer, device, exceptions)
- Which devices have interrupt support (xHCI, AHCI, VirtIO, PIC)
- Interrupt handler patterns (spinlock usage, stack depth)
- Interrupt context constraints (no sleeping, no allocation)

### 3. Synchronization Primitives
Check how polling loops interact with:
- Spinlocks (may block interrupt-driven transitions)
- Memory barriers (needed for interrupt safety?)
- RCU-style patterns (if any)
- Volatile reads/writes

## Assessment Questions Per Loop

For each polling loop, answer:

1. **What is it waiting for?**
   - Device state change?
   - Event completion?
   - Timeout/deadline?
   - Resource availability?

2. **Is polling necessary?**
   - Can this hardware trigger an interrupt instead?
   - Is the polling a workaround for a quirk? (Note the quirk)
   - Is there latency sensitivity that requires polling?

3. **What interrupt would replace it?**
   - Device interrupt (MSI/MSI-X/line interrupt)?
   - Timer interrupt (APIC timer, HPET)?
   - Software IPI?
   - Event-driven notification?

4. **Refactoring difficulty:**
   - **Low**: Can add interrupt handler immediately
   - **Medium**: Requires synchronization changes
   - **High**: Requires architectural refactoring (e.g., scheduler redesign)
   - **Blocked**: Missing hardware support or requires major protocol change

5. **CPU impact if left as-is:**
   - **High**: Runs every iteration of main loop (e.g., scheduler tick)
   - **Medium**: Runs frequently but not every cycle
   - **Low**: Rare hot paths or bounded loop count

6. **Risk assessment if refactored:**
   - Race conditions introduced?
   - Increased latency vs polling?
   - Interrupt storm potential?
   - Compatibility issues?

## Output Format

Create a single comprehensive report with the following structure:

```
# Polling Loop Audit Report

## Summary
- Total polling loops found: [N]
- High-impact loops: [N]
- Medium-impact loops: [N]
- Low-impact loops: [N]
- Already interrupt-driven: [N]

## Critical Findings
[List any loops that are clear CPU wasters or architectural blockers]

## Detailed Loop Inventory

### Loop 1: [Name/Purpose]
- **Location**: `path/file.rs:line`
- **Current Pattern**: [loop structure code snippet]
- **Frequency**: [estimate]
- **CPU Impact**: [high/medium/low]
- **What it waits for**: [description]
- **Polling necessary?**: [yes/no + reasoning]
- **Refactoring**: [suggested approach]
- **Difficulty**: [low/medium/high/blocked]
- **Risk**: [assessment]

### Loop 2: [repeat for each loop]

## Architectural Blockers
[List any design patterns that prevent interrupt-driven refactoring]

## Recommended Refactoring Priority
1. [Loop most impactful to fix + approach]
2. [Next priority]
3. [Next priority]

## Interrupt Infrastructure Gaps
[Identify missing interrupt handlers or capability gaps]

## Proof-of-Concept Suggestion
[Suggest one quick-win refactoring that could serve as a template]
```

## Key Heuristics

- **Beware of `loop {}` without any break condition** → likely infinite busy-wait
- **TSC-based timeouts** → replace with interrupt-driven timer
- **Event ring draining** → consider event-driven completion handlers
- **Status register polling** → check if hardware can signal via interrupt
- **Scheduler preemption tick** → replace with APIC timer interrupt
- **Device discovery loops** → can be one-time init, not runtime polling

## Special Cases

### xHCI Controller
- Current: Event ring polling in Phase 9
- Status: Memory in USB memory (`usb_memory_next()`)
- Check: Is there an interrupt capability that UEFI disabled?
- Refactoring blocker: UEFI runtime ownership during boot?

### Scheduler
- Current: 100 Hz preemptive tick
- Status: May be implemented as polling check, not timer interrupt
- Check: Is APIC timer configured? Can we use it instead?

### Network Drivers
- Current: Likely polling for packet/DMA completion
- Status: VirtIO/AHCI may support MSI-X
- Refactoring: Requires completion ring + interrupt handler

## Questions to Answer in Report

1. What is the single biggest CPU consumer right now?
2. Which three loops, if refactored, would have highest impact?
3. What interrupt infrastructure is missing or underutilized?
4. Is the 100 Hz scheduler tick polling or timer-based?
5. Do all devices that support interrupts have them enabled?
6. Are there any polling workarounds for hardware quirks?
7. What is the critical path that must remain polling-based?

## Deliverables

1. **Comprehensive inventory** of all polling loops with line-by-line locations
2. **Assessment matrix** (loop name × refactoring difficulty × CPU impact)
3. **Architectural recommendations** for moving to interrupt-driven design
4. **Proof-of-concept roadmap** with quick wins first
5. **Risk analysis** for each proposed refactoring

---

**Note**: This audit should be exhaustive. Scan the codebase systematically (don't rely on familiarity). Look for hidden polling in conditional checks, state machine loops, and device driver initialization. Report every loop, even low-impact ones—the goal is complete visibility.
