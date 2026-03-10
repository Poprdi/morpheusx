# MorpheusX SMP Architecture (Source-Verified)

Status: current implementation snapshot
Scope: `hwinit` kernel SMP bring-up, per-core state, interrupt/scheduler behavior, locking, and invariants
Verified against source:
- `hwinit/src/platform.rs`
- `hwinit/src/cpu/{ap_boot.rs,apic.rs,acpi.rs,per_cpu.rs,gdt.rs,idt.rs}`
- `hwinit/asm/cpu/{ap_trampoline.s,context_switch.s,syscall.s}`
- `hwinit/src/process/{mod.rs,context.rs,scheduler.rs}`
- `hwinit/src/sync.rs`

## 1) Executive Summary

MorpheusX boots on the BSP first, initializes core kernel subsystems, then brings up APs via INIT+SIPI using a low-memory trampoline at physical `0x8000`.

Per-core runtime state lives in a GS-based `PerCpu` block. Timer interrupts run on every online core through LAPIC vector `0x20`, entering the same assembly ISR and Rust `scheduler_tick()` path.

Scheduling uses a single global process table plus a global ISR-safe lock, with per-process `running_on` ownership to prevent two cores from running the same process concurrently. PID 0 (kernel process) is BSP-owned; APs run user work or an AP-local ring-0 idle context.

## 2) CPU Topology Discovery

Topology discovery occurs in platform phase 12 (`SMP`).

Primary path:
- Parse ACPI MADT and extract enabled or online-capable Local APIC IDs, excluding BSP LAPIC ID.
- Set CPU count to `madt_ap_count + 1` (BSP).
- Start APs only from the discovered MADT list.

Fallback path:
- Detect CPU count from CPUID topology leaves (`0xB` preferred, leaf `1` fallback).
- Brute-force LAPIC ID scan `0..255` excluding BSP ID, probing each by INIT+SIPI.

Important limits:
- `MAX_CPUS = 16` (`per_cpu.rs`), hard cap for arrays and bring-up loops.

## 3) BSP Bring-up Timeline Relevant to SMP

From `platform_init_selfcontained()`:

1. GDT/TSS init on BSP.
2. IDT init on BSP.
3. SSE enable on BSP.
4. Probe LAPIC base from MSR `IA32_APIC_BASE (0x1B)`.
5. Initialize BSP `PerCpu` (`core_idx=0`, mapped by LAPIC ID).
6. Paging init and BSP LAPIC init.
7. Scheduler init (creates PID 0 as running on BSP).
8. SYSCALL MSR init.
9. Disable legacy PIC; install LAPIC timer ISR at vector `0x20`; enable interrupts.
10. SMP phase:
   - MADT parse or CPUID fallback
   - AP startup pass

So AP startup happens only after scheduler, IDT, syscall path, LAPIC timer, and interrupts are operational on BSP.

## 4) AP Startup Mechanics

## 4.1 Trampoline placement and data contract

AP trampoline is copied to physical `0x8000` (below 1 MiB).

Data area is at trampoline page offset `0xF00`; BSP writes:
- `TD_CR3` at `+0x00`
- `TD_ENTRY64` at `+0x08` (`ap_rust_entry`)
- `TD_STACK` at `+0x10` (AP kernel stack top)
- `TD_CORE_IDX` at `+0x18`
- `TD_LAPIC_ID` at `+0x1C`
- `TD_GDT_PTR` at `+0x20` (10-byte SGDT payload)
- `TD_READY` at `+0x30` (currently written by BSP, not used as readiness source)

Readiness source used by BSP wait loop is `AP_ONLINE_COUNT` increment from AP `per_cpu::init_ap()`.

## 4.2 INIT+SIPI sequence

For each target APIC ID:
- Allocate AP kernel stack (`64 KiB`).
- Fill trampoline data and memory fence.
- Send INIT IPI.
- Delay 10 ms.
- Send SIPI #1 with vector `0x08` (page `0x8000 / 0x1000`).
- Delay 200 us.
- Send SIPI #2.
- Delay 200 us.
- Poll `AP_ONLINE_COUNT` up to ~100 ms.
- On timeout, free allocated AP stack and continue.

## 4.3 Trampoline mode transition

`ap_trampoline.s` flow:
- 16-bit real mode entry at SIPI target.
- Load temporary low-memory GDT (`0x8E00`) to avoid truncated high GDT base in 16-bit `lgdt`.
- Enter 32-bit protected mode.
- Enable `CR4.PAE`.
- Load kernel CR3 from trampoline data.
- Set `IA32_EFER.LME` and `IA32_EFER.NXE`.
- Enable paging (`CR0.PG`) to activate long mode.
- Far jump to 64-bit code selector in temp GDT.
- In long mode, load real BSP GDT pointer from trampoline data (`TD_GDT_PTR`).
- Load AP stack from `TD_STACK`.
- Set args `(core_idx, lapic_id)` in SysV registers `(RDI, RSI)`.
- Jump to `ap_rust_entry`.

NXE is explicitly enabled because kernel page tables use NX bits; without NXE this causes reserved-bit page faults and reset behavior.

## 4.4 AP Rust entry sequence

`ap_rust_entry(core_idx, lapic_id)` does:
1. Per-AP GDT+TSS load (`init_gdt_for_ap`).
2. Shared IDT load (`load_idt_for_ap`).
3. Per-CPU init with probed LAPIC base (`init_ap`).
4. SSE enable.
5. SYSCALL MSR init on that core.
6. AP LAPIC init.
7. LAPIC timer setup (`100 Hz`).
8. `sti`; then AP idle loop (`hlt`).

## 5) Per-CPU Model and ABI

`PerCpu` is `#[repr(C, align(64))]` and GS-addressed. Offsets are ABI with assembly.

Hot fields and offsets:
- `0x00` `self_ptr`
- `0x08` `cpu_id` (LAPIC ID)
- `0x0C` `current_pid`
- `0x10` `next_cr3`
- `0x18` `current_fpu_ptr`
- `0x20` `kernel_syscall_rsp`
- `0x28` `user_rsp_scratch`
- `0x30` `tss_ptr`
- `0x38` `lapic_base`
- `0x40` `tick_count`

Additional core state:
- `online: bool`
- `in_tick: bool`
- `boot_kernel_rsp: u64` (AP original kernel stack top used for AP idle restoration)

Global mapping model:
- Sequential core index (`0 = BSP`) is distinct from LAPIC ID.
- Sparse `LAPIC_TO_IDX[256]` maps APIC IDs to sequential indices.
- `AP_ONLINE_COUNT` counts initialized cores.

GS base setup:
- BSP and APs write `IA32_GS_BASE` to their own `PerCpu` address.
- `IA32_KERNEL_GS_BASE` starts as zero; `swapgs` used on user/kernel transitions.

## 6) Interrupt and Context Switch Path (All Cores)

Timer source:
- LAPIC timer periodic mode at vector `0x20`.
- ISR symbol: `irq_timer_isr` in `context_switch.s`.

ISR high-level flow:
1. If interrupted context is ring 3, do `swapgs` on entry.
2. Allocate `CpuContext` frame on stack, save GPRs.
3. Copy CPU iret frame fields (RIP/CS/RFLAGS/RSP/SS) into `CpuContext`.
4. `fxsave` to outgoing process FPU area (`gs:[0x18]`).
5. ACK LAPIC EOI.
6. Call Rust `scheduler_tick(&current_ctx)`.
7. `fxrstor` from incoming process FPU area (`gs:[0x18]`).
8. Load CR3 from `gs:[0x10]` if nonzero and different.
9. Patch hardware iret frame with incoming context.
10. Restore incoming GPRs.
11. Drop temporary `CpuContext` stack frame.
12. If returning to ring 3, `swapgs` on exit.
13. `iretq`.

`CpuContext` layout is compile-time asserted in Rust (`process/context.rs`) and matches assembly offsets exactly (`0xA0` bytes).

## 7) Scheduling Model on SMP

## 7.1 Global data and lock

- Process storage: static `PROCESS_TABLE: [Option<Process>; 64]`.
- Global guard: `PROCESS_TABLE_LOCK: IsrSafeRawSpinLock`.
- Lock disables interrupts on lock acquisition and restores prior IF on unlock, tracking IF per core.

Current scheduling design is one global run domain (single table, global lock), not per-core runqueues.

## 7.2 Process ownership across cores

Each process has `running_on: u32`:
- `u32::MAX` means not currently executing on any core.
- Otherwise holds sequential core index currently running that process.

Scheduler sets `running_on` when dispatching and clears it when descheduling. `pick_next()` skips candidates with `running_on != u32::MAX`.

## 7.3 PID 0 policy

- PID 0 is kernel process and is BSP-owned.
- AP cores do not execute PID 0 as normal runnable work.
- If AP has no user task to run, scheduler returns AP-local ring-0 idle context (`ap_idle_context`) using AP `boot_kernel_rsp`.

This avoids corrupting BSP kernel context by accidentally saving/restoring PID 0 state from AP timer interrupts.

## 7.4 `scheduler_tick()` on each core

Common behavior:
- Runs on every timer IRQ on every core.
- BSP-only side work: framebuffer present tick and PS/2 mouse poll.
- Holds process table lock through save/select/prepare-return path.

AP idle fast path (`cur_pid==0 && core_idx!=0`):
- Deliver signals + wake sleepers.
- Try select runnable non-PID0 process.
- If none: restore AP kernel stack pointers and return AP idle context.
- If found: mark running, set TSS RSP0 and `kernel_syscall_rsp`, set `next_cr3`, set FPU ptr, return process context.

Normal path:
- Save outgoing context into current process slot.
- Update CPU accounting and kernel idle donation bookkeeping.
- Mark outgoing Running -> Ready; clear `running_on`.
- Deliver signals and wake timed sleepers.
- Pick next with SMP and BSP/AP rules.
- For AP no-work case, switch to AP idle context.
- For selected process: mark Running, set `running_on`, update per-core TSS/syscall stack, set CR3 and FPU pointers, return next context.
- Fallback handling exists if selected PID disappeared between pick and fetch.

## 7.5 Run selection (`pick_next`)

Round-robin over process table with wraparound:
- Skip processes already `running_on` some core.
- APs always skip PID 0.
- BSP may skip PID 0 when kernel recently idled (idle donation), but floor enforcement (`MAX_KERNEL_SKIP=1`) guarantees periodic kernel quanta.
- If no candidate: keep current if still runnable and valid for core role.
- Absolute fallback: return 0 (AP path then converts to AP idle context).

## 8) Signal Delivery SMP Semantics

Signal operations are lock-protected, but delivery and forced termination account for cross-core execution:

- `SIGKILL` and `SIGSTOP`:
  - If target process is running on a core (`running_on != u32::MAX`), request is deferred by setting pending signal.
  - If not running, action is immediate (`terminate_process_inner` for kill, state block for stop).

Rationale in code: avoid racing with a running core that may hold mutable process references during syscall/scheduler handling.

## 9) LAPIC and Timer Details

- Actual LAPIC base is probed from `IA32_APIC_BASE` MSR and cached globally.
- BSP maps LAPIC MMIO UC after paging init (`kmap_mmio`).
- Both BSP and APs enable LAPIC via SVR and set TPR=0.
- Timer calibration uses PIT channel 2 over ~10 ms to derive periodic init count.
- Legacy PIC is masked off once LAPIC path is active.

## 10) SYSCALL and GS Interaction

`syscall.s`:
- `syscall_init` configures `IA32_EFER.SCE`, `STAR`, `LSTAR`, and `FMASK`.
- `syscall_entry` does unconditional `swapgs` (always from ring 3), saves user RSP in `gs:[0x28]`, switches to `gs:[0x20]` kernel stack, dispatches syscall, restores, `swapgs`, `sysret`.

Timer ISR also uses conditional `swapgs` based on interrupted CS privilege level.

So both syscall and interrupt paths rely on valid GS-per-core setup before user mode and periodic preemption are active.

## 11) Synchronization and SMP-Safety Primitives

## 11.1 `IsrSafeRawSpinLock`

Used for high-contention/shared kernel structures touched from both ISR and non-ISR contexts. Behavior:
- Capture current IF.
- Disable interrupts.
- Spin on atomic lock.
- Store previous IF in per-core slot.
- Unlock restores lock and then previous IF.

Core index for IF slot is read from `gs:[0x00]` and masked to 64-entry array.

## 11.2 Other SMP-guarded shared paths

Observed SMP-protected shared structures include:
- Scheduler process table (`PROCESS_TABLE_LOCK`).
- Stdout push path (`PUSH_LOCK`, `stdout.rs`).
- Pipe write serialization comments and lock usage (`pipe.rs`).
- Filesystem operation serialization in syscall common path (`syscall/handler/common.rs` comments and lock).

## 12) Core and Stack Allocation

- AP kernel stack allocation: `64 KiB` per AP during bring-up.
- Process kernel stacks: `128 KiB` per process from memory registry.
- AP per-core GDT/TSS arrays are static (`MAX_CPUS` sized), no heap allocation.

Sequential core index assignment:
- BSP fixed at index `0`.
- APs assigned incrementally as successfully started (`core_idx` increments on successful boot response).
- LAPIC ID to index mapping updated during `per_cpu::init_bsp/init_ap`.

## 13) Hard SMP Invariants (Current Code)

1. PerCpu field order/offsets are ABI; assembly depends on fixed GS offsets.
2. `CpuContext` field offsets/size must match timer ISR assembly exactly.
3. AP startup trampoline data offsets in Rust and assembly must stay identical.
4. AP startup code at `0x8000` must remain below 1 MiB and SIPI-page aligned.
5. AP must not use BSP PID 0 process context as runnable user/kernel context.
6. AP idle fallback must return ring-0 AP idle context, not stale ring-3 context.
7. `running_on` must be set on dispatch and cleared on deschedule for correctness.
8. Scheduler selection must skip processes already `running_on` another core.
9. AP cores must never schedule PID 0 normal work.
10. Core-local `kernel_syscall_rsp` must be updated on context switch for syscall entry.
11. Core-local TSS RSP0 must track incoming process kernel stack top.
12. `next_cr3` and `current_fpu_ptr` must be set before ISR restore path runs.
13. `SIGKILL/SIGSTOP` for currently running targets must be deferred, not immediate in-place mutation.
14. LAPIC EOI must be issued each timer interrupt path.
15. GS base must be initialized per-core before relying on GS hot-path fields.
16. AP online readiness for BSP wait loop is `AP_ONLINE_COUNT` delta.
17. Global process table mutations must hold `PROCESS_TABLE_LOCK`.
18. Interrupt state restoration in ISR-safe locks must be per-core, not global.
19. Legacy PIC must be masked once LAPIC interrupt path is active.
20. NXE must be enabled before using page tables with NX bits during AP long-mode transition.

## 14) Known Design Characteristics and Limitations

- Scheduler is SMP-safe but globally serialized around one process table lock.
- There are no per-core runqueues and no load-balancing heuristics beyond round-robin scan with ownership checks.
- AP bring-up success is inferred by online counter increment, not by explicit trampoline ready flag usage.
- CPU count and AP assignment are capped by `MAX_CPUS`.

## 15) Quick Mental Model

- BSP does full kernel init, enables scheduler and LAPIC timer, then starts APs.
- Every core gets GS->PerCpu, LAPIC timer IRQs, and enters common scheduler tick code.
- BSP can run kernel PID 0 and user processes.
- APs run user processes or AP-local idle loop.
- Global process table lock + `running_on` ownership keep execution single-owner per process across cores.

That is SMP on MorpheusX as implemented in current source.
