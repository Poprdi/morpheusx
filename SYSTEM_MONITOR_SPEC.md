# MorpheusX System Visualizer — 3D Real-Time OS Inspector

**Vision:** A unified 3D visualization interface that makes the entire running system *visible* in real-time. Watch processes, memory, caching, IPC, exceptions, interrupts, and execution all flow together as a living 3D model of your OS.

This is what no other OS can do: render its own internals as a native 3D application.

---

## Core 3D Visualization Layers

### Layer 1: Process Cloud (The Main View)
```
Every running process is a 3D sphere/cube in virtual space:
- Position: (x, y, z) in 3D grid based on process tree hierarchy
- Size: Volume proportional to memory usage (MB)
- Color: Hue = CPU thread, Saturation = utilization %
- Height (Z): Priority level (high priority floats higher)
- Glow: Intensity = active cycles in current frame
- Lines connecting: IPC messages flowing between processes

Navigation:
- Arrow keys: Rotate view
- Mouse/trackpad: Zoom (scroll), pan
- Enter: Focus on selected process
- 'K': Kill selected process
- 'P': Pin/follow process (camera tracks it)
- Space: Pause/resume visualization
- 'S': Toggle slow-motion (16× slower for detailed inspection)
```

**Design detail:**
- Init process (PID 1) at center origin
- Child processes arranged in orbit around parent
- Dynamic repulsion algorithm: processes slightly push apart (don't overlap)
- Smooth animation: transitions follow easing curves for visceral feel

---

### Layer 2: Real-Time CPU Load Visualization
```
Each process has a "heat aura" around it:
- Animating gradient shell: red (hot) → yellow → green (cool)
- Thickness of gradient = recent CPU usage (10-frame rolling average)
- Sparkles/particles: one per CPU cycle consumed (visual feedback of work)

Graph at bottom-left:
- Timeline of system load: Ready Q depth, Running, Blocked count
- Stacked area chart (area stays same size, color bands show state)
- Color: green=running, yellow=ready, blue=blocked
- Scrolls left as time advances
```

---

### Layer 3: Memory Visualization (Drill-down)
```
Select a process (Enter) → zoom into its memory space:
- Virtual address space shown as vertical 3D column/tower
- Color-coded regions:
  * Blue: Code/text segment
  * Green: Heap (animated blob, grows/shrinks with allocations)
  * Red: Stack (shown as a growing/shrinking bar from top)
  * Yellow: Mapped memory regions
  
Heap fragmentation detail:
- Buddy allocator shown as tree of boxes
- Each size class (8KB, 16KB, 32KB, etc.) as a row
- Green boxes = allocated, gray = free
- Sizes labeled on left, total free space at top
```

---

### Layer 4: IPC Communication Graph
```
Lines between processes represent messages:
- Line color: type of IPC (red=signals, blue=messages, green=shared mem)
- Line thickness: queue depth (thick = backlogged)
- Pulsing animation: message traveling from sender to receiver
- Brightness: frequency (hot pathways glow)

Deadlock detection (automatic):
- If cycle detected (A→B→C→A), those process spheres BLINK RED
- Alert text: "DEADLOCK DETECTED: PID 3→7→5→3"
- Can drill into deadlock to see message queues
```

---

### Layer 5: Exception Trace Overlay
```
When exception occurs (page fault, divide-by-zero, illegal instruction):
- Red spark/explosion at process location
- Line of text appears: "[TIME] PID X: Page Fault at 0x[addr]"
- Exception log scrolls at top-right of screen
- Filter: [All] [Faults] [Traps] [Errors] [Warnings]
- Can replay exception history (timeline slider at bottom)
```

---

### Layer 6: Hardware Interrupt Heatmap
```
IRQ events shown as "lightning strikes" from edges of screen:
- Timer IRQ (frequent): blue bolts from top
- Keyboard IRQ: green bolts from left
- Network IRQ: yellow bolts from bottom
- Disk IRQ: orange bolts from right
- Frequency shown as bolt rate

Histogram at bottom-right:
```
IRQ Frequency (last 10s):
Timer: ████████████ 1000Hz
Kbd:   █ 50Hz
Net:   ░ 0Hz
Disk:  ░░ 5Hz
```
```

---

### Layer 7: Execution Timeline / Flamegraph (Side View)
```
Toggle with 'F' to see timeline view:
- Left panel: Process list (bar for each process)
- Each bar animated: blocks show call stack depth over time
- Color intensity: CPU utilization in that time slice
- Scroll to zoom into time range, click to see details

Hovering shows:
- Process name
- Stack depth at that moment
- Top 3 functions (if we instrument the kernel)
- CPU cycles consumed in that window
```

---

### Layer 8: Cache Heatmap (Thermal View)
```
Toggle with 'H' to see memory thermal visualization:
- Show address space as 2D grid (X=address, Y=time)
- Tile color = access frequency (hot=red, cold=blue)
- Identify working set, hot data structures
- Per-process or system-wide toggle

Shows:
- Spatial locality (consecutive accesses = diagonal lines)
- Temporal locality (repeated accesses = vertical streaks)
- Cache misses as spikes
```

---

### Layer 9: System Syscall Flow
```
When processes make syscalls:
- Draw arrow from process sphere toward center (kernel)
- Arrow color by syscall type (I/O=blue, memory=green, IPC=red)
- Animation speed = syscall duration
- Label shows syscall name: "read" or "write" or "ps"
- Histogram at bottom: "Syscalls this second: [write:50, read:30, ...]"
```

---

### Layer 10: Instruction Heatspot (Advanced)
```
If light cycle sampling available:
- Show hot instructions per process
- ASCII representation of x86-64 instruction bytes
- Highlight most-executed instructions
- Shows instruction cache pressure

Toggle with 'I' for detailed view:
- Disassembly view with frequency annotations
- Identify tight loops, expensive ops
```

---

### Layer 11: Hardware Topology & Physical Layout
```
Toggle with 'W' (hardWare) to enter hardware visualization mode:

Physical layout of system hardware shown in 3D space:
- CPU socket in center (multi-core visualized as nested rings)
- Memory DIMM slots around CPU (color = populated, gray = empty)
- PCI bus lanes extending from CPU (devices on lanes)
- Storage controllers below (SSDs/HDDs as spinning disks)
- Network interfaces floating to the side
- Power delivery/thermal subsystems as atmospheric effects

Colors by health:
- Green: Normal operation
- Yellow: Elevated (warm temps, high utilization)
- Red: Critical (throttling, errors)
- Gray: Disabled/unused

Click on hardware element → drill-in panel shows:
```
═════════════════════════════════════════════════════════
 CPU SOCKET 0 (Details)
───────────────────────────────────────────────────────
 Model: Intel Core i7-9700K
 Cores: 8 @ 3.6GHz (boost to 4.9GHz)
 
 Core status:
  Core 0: 3.8GHz, 72°C, 45% util
  Core 1: 3.6GHz, 68°C, 10% util
  Core 2: 3.9GHz, 75°C, 89% util
  ...
 
 Cache hierarchy:
  L1 Inst: 32KB × 8 (100% hit rate)
  L1 Data: 32KB × 8 (95% hit rate)
  L2:      256KB × 8 (87% hit rate)
  L3:      12MB shared (60% hit rate)
 
 Voltage: 1.05V (stable)
 Power: 65W
 TDP headroom: 29W remaining
 Thermal: Safe (margin: 28°C)
 
 [Press 'I'] Show instruction mix
 [Press 'P'] Show power breakdown
═════════════════════════════════════════════════════════
```

**Hardware elements visualizable:**

1. **CPU Package(s)**
   - Nested rings = cores
   - Color brightness = frequency
   - Rotation speed = activity
   - Label: Model, total cores, max frequency

2. **Memory Subsystem**
   - DIMM slots as vertical bars
   - Color intensity = memory usage on that bank
   - Size label, speed (MHz), current load
   - Bandwidth utilization bar

3. **Caches (3-level hierarchy)**
   - Visualized as concentric circles around CPU core
   - L1 (tiny, innermost), L2 (medium), L3 (large, shared)
   - Color = hit rate (green=hits, red=misses)
   - Miss rate shows as "sparks" flowing from cache to memory

4. **PCI Bus & Devices**
   - Bus lanes as thick lines from CPU
   - Each device as a cube on the lane
   - Color = device type (blue=NIC, orange=storage, green=GPU, etc.)
   - Pulsing = bus traffic

5. **Storage Controllers & Drives**
   - SATA/NVMe controller as main hub
   - SSDs/HDDs as spinning disks below
   - Rotation speed = access frequency
   - LED indicator = activity (green flash on I/O)
   - Latency bar (red if high)

6. **Network Interfaces**
   - NIC as a cube on the side
   - Pulsing lines = packets flowing
   - RX indicator, TX indicator
   - Link speed, packet rate, errors

7. **Power Delivery**
   - Visualized as energy field around CPU
   - Color = voltage stability (green=stable, orange=undershoot, red=unstable)
   - Particle effects = current draw
   - Power rails labeled: 12V, 5V, 3.3V
   - Real-time wattage from power sensors (if available)

8. **Thermal Subsystem**
   - Heat gradient radiating from hot components
   - Red = hot, blue = cool
   - Thermal zones labeled (CPU, VRM, memory, etc.)
   - Fan speeds (if controllable)

9. **Interrupt Controller (APIC/IOAPIC)**
   - Central hub showing IRQ routing
   - Lines to CPU cores (which core handling which IRQ)
   - Pulsing intensity = interrupt rate
   - Number on line = IRQ number

10. **System Clock / Oscillators**
    - Metronome-like oscillator
    - Frequency = system clock, other frequencies offset
    - Connected to CPU, memory controllers, etc.
    - Shows base clock, multiplier, actual achieved frequency
```

---

## Hardware Stats Exposed (Drill-In Detail Views)

**CPU Core Details:**
- Current frequency (actual vs nominal)
- Temperature (per-core if available from CPUID thermal leaf)
- Utilization %
- Context switch count
- Cache miss rates (L1, L2, L3)
- Power consumption (estimated from frequency/voltage)
- Thermal throttling status

**Memory Details:**
- Capacity (per DIMM or bank)
- Speed (MHz)
- Latency (row, column, refresh)
- Current bandwidth utilization
- Error correction status (if ECC)
- Temperature (DIMM thermal sensor if available)
- Refresh cycles/second

**Storage Details:**
- Capacity
- Used space
- Access time (recent samples)
- I/O throughput (IOPS, MB/s)
- Error rate
- S.M.A.R.T. status (health indicators)
- Power state (active, idle, sleep)

**Network Details:**
- Link speed
- MAC address
- RX packets/bytes, TX packets/bytes
- Error rate, dropped packets
- Current throughput
- Link state (up/down)

**PCI Devices Details:**
- Device ID / vendor
- Bus address (B:D:F)
- Current power state
- Interrupts routed to core
- BAR mappings (memory regions)

**Power Delivery Details:**
- Voltage on each rail (12V, 5V, 3.3V)
- Voltage ripple (stability)
- Current draw per rail
- Total system power
- Power efficiency (PUE-like metric if applicable)

**Thermal Details:**
- Die temperature (CPU)
- Junction temperature (if available)
- Thermal margin to throttle point
- Fan speed / PWM controls
- Thermal throttling status
- Thermal time constant (how fast it heats/cools)

---

## Syscall Audit & Implementation Strategy

### What We Already Have (No New Syscalls Needed)

| Syscall | ID | Exposed Data |
|---------|-----|-----------|
| `SYS_PS` | 65 | PID, PPID, state, priority, cpu_ticks, pages_alloc, name |
| `SYS_SYSINFO` | 23 | total/free mem, proc count, uptime, TSC freq, heap stats |
| `SYS_CPUID` | 69 | CPU model, core count, cache sizes, features, thermal leaf |
| `SYS_RDTSC` | 70 | Cycle counter + calibrated frequency |
| `SYS_MEMMAP` | 72 | Physical memory map (phys_start, num_pages, mem_type) |
| `SYS_BOOT_LOG` | 71 | Kernel boot log — contains device enum, IRQ setup, driver init |
| `SYS_NIC_INFO` | 32 | MAC address, link up/down, NIC presence |
| `SYS_NIC_LINK` | 35 | Link state |
| `SYS_PCI_CFG_READ` | 54 | Read any PCI config register (vendor, device, class, BARs) |
| `SYS_PORT_IN` | 52 | Read I/O ports — can probe CMOS RTC, PIT, etc. |
| `SYS_IRQ_ATTACH` | 60 | Attach to IRQ line |
| `SYS_SHM_GRANT` | 73 | Shared memory between processes |

### What Can Be Built Without New Syscalls

**Hardware Topology (Phase 4 — Layer 11):**
- **CPU:** CPUID leaf 0x80000002-4 (model string), leaf 0x1/0xB (core count), leaf 0x4/0x8 (cache), leaf 0x16 (freq), leaf 0x6 (thermal). ✅ No new syscall.
- **Memory:** SYS_MEMMAP gives physical map, SYS_SYSINFO gives used/free. ✅ No new syscall.
- **PCI Devices:** SYS_PCI_CFG_READ — enumerate bus 0-255, dev 0-31, func 0-7, read vendor/device/class. ✅ No new syscall.
- **Network:** SYS_NIC_INFO gives MAC + link. ✅ No new syscall.
- **Storage:** Parse boot log for disk detection, or PCI enumerate for AHCI/NVMe class codes. ✅ No new syscall.

**Memory Visualization (Phase 1 — Layer 3):**
- Physical map via SYS_MEMMAP. Process-level pages_alloc from SYS_PS. Kernel heap from SYS_SYSINFO. ✅ No new syscall.

**Boot Log Analysis (for exception/IRQ history):**
- SYS_BOOT_LOG returns full kernel log text — parse for exception traces, IRQ info, driver events. ✅ No new syscall.

### What Genuinely Needs New Syscalls

| Feature | Why Existing Can't Do It | Required Syscall |
|---------|-------------------------|------------------|
| **Live IPC graph** (Layer 4) | SYS_PS lacks blocked_on_pid, message queues | Extend PsEntry OR add SYS_IPC_STATE |
| **Live exception events** (Layer 5) | Boot log is write-once history, not real-time | SYS_EXCEPTION_LOG (ring buffer) |
| **Live IRQ counters** (Layer 6) | Can attach via SYS_IRQ_ATTACH but can't read counters | SYS_IRQ_STATS |
| **Per-process heap map** (Layer 3 detail) | SYS_SYSINFO only gives kernel heap | SYS_HEAP_MAP |
| **Instruction profiling** (Layer 10) | No PMC access exposed | SYS_PERF_SAMPLE |

### Recommended Approach

**Implement now (highest ROI):**
1. **Hardware topology visualization:** Use CPUID + SYS_PCI_CFG_READ + SYS_NIC_INFO to render Layer 11 (CPU cores, memory, storage, NIC, thermal). **Zero kernel changes.**
2. **Boot log parser:** Extract exception/IRQ history from SYS_BOOT_LOG for Layer 5/6 background. **Zero kernel changes.**
3. **Extend PsEntry** (low-effort kernel change): Add `blocked_on_pid: u32`, `context_switches: u32`. Enables Layer 4 (IPC graph) + improved Layer 1 visualization.

**Defer (Phase 3+):**
- Exception ring buffer, IRQ counters, heap map, perf sampling — kernel-deep changes requiring new syscalls.

---

## New Syscalls Needed for Hardware Info (Future)

If we decide to fully instrument hardware monitoring:

1. `SYS_HW_INFO` (future)
   - Query hardware topology: CPU cores, memory banks, PCI devices, storage
   - Returns struct with hardware inventory (can be derived from existing syscalls)

2. `SYS_CPU_FREQ` (future)
   - Current frequency per core
   - Base / boost frequencies
   - Thermal state (can read from CPUID + CMOS)

3. `SYS_MEMORY_STAT` (future)
   - Per-bank statistics: capacity, populated, temperature
   - Total memory pressure (can derive from SYS_MEMMAP + SYS_SYSINFO)

4. `SYS_EXCEPTION_LOG` (future - recommended priority)
   - Returns recent exceptions: (timestamp, type, pid, address)
   - Types: PageFault, DivByZero, IllegalInstr, GPFault, etc.

5. `SYS_IRQ_STATS` (future - recommended priority)
   - Returns per-IRQ statistics: (irq_num, count, total_cycles)
   - Allows visualizing interrupt frequency

6. `SYS_PERF_SAMPLE` (future)
   - Returns instruction frequency data (which x86 opcodes executed most)

---

## Hardware Visualization UI Controls

```
HARDWARE VIEW ENTRY:
  'W'              Toggle hardware mode (enter/exit)
  
IN HARDWARE MODE:
  Arrow Keys       Rotate 3D hardware layout
  Scroll/Pinch     Zoom in/out on specific component
  
  Click hardware   Drill-in to get detailed stats
  Esc              Back to previous view / exit hardware mode
  'C'              Show cache hierarchy details
  'P'              Show power breakdown (all rails)
  'T'              Show temperature map (thermal gradient)
  'F'              Show frequency scaling (all cores)
  'M'              Show memory bank stats
  'S'              Show storage device stats
  'N'              Show network interface stats
  'I'              Show interrupt routing (APIC)
  
FILTERING:
  'E'              Show only components with errors/warnings
  'H'              Highlight hottest components
  'P'              Highlight highest power draw
```

---

## Hardware Visualization Integration with Process View

**Bidirectional linking:**
```
Process → Hardware:
- Click on process sphere
- Shows which CPU core it's running on (highlight that core)
- Shows memory banks it's using (highlight banks)
- Shows which storage/network I/O it's doing

Hardware → Process:
- Click a CPU core
- Shows which processes are running/waiting on that core
- Shows scheduling history (which process ran when)

Example interactions:
- Process A is laggy
- Click on it → see it's running on slow-boosting core 2
- Click core 2 → see it's thermally throttled (high temp)
- Click thermal zone → see it's getting heat from nearby VRM
- Click VRM → see it's in power-delivery limiter mode
- Result: "Power delivery limited, thermal throttled core → app lagging"
```

---

## Visualization Aesthetics

**Color scheme:**
- CPU cores: Blue (base frequency) → Purple → Red (max frequency)
- Memory: Green (available) → Yellow (used) → Red (full)
- Storage: Gray (idle) → Yellow (active) → Red (hot)
- Thermal: Blue (cool) → Green → Yellow → Red (hot)
- Power: Green (normal) → Orange (elevated) → Red (critical)

**Animation:**
- CPU cores rotate/pulse with activity
- Memory banks "fill up" like water tanks as load increases
- Storage drives spin faster with I/O load
- Power delivery visualized as flowing energy (particle effects)
- Thermal radiates as glowing heat field
- Clock oscillators blink at their respective frequencies

**Overlays:**
- Real-time values float above/near each component (e.g., "3.8 GHz", "72°C")
- Unit labels: GHz, MHz, MB/s, IOPS, °C, V, W
- Arrows showing data flow: CPU ↔ Memory, CPU ↔ Storage, etc.

---

## Why Hardware Visualization Matters

1. **Unique.** No mainstream OS visualizes its own hardware like this
2. **Educational.** Users *see* how the system works (CPU-memory bandwidth, thermal limits)
3. **Diagnostic.** "Why is my app slow?" Visual inspection finds: thermal throttle, power limit, cache misses
4. **Beautiful demo.** Glowing 3D hardware with real-time stats is mesmerizing
5. **Foundational.** Later can add device drivers' own visualization (GPU workload, NIC packet flow, etc.)

---

## Integration with Existing System Visualizer

**Main view modes:**

Mode 1: **Process Cloud** (default, 'W' to switch out)
- Process spheres + IPC lines + exceptions + syscalls
- Shows OS software behavior

Mode 2: **Hardware Topology** (toggle with 'W')
- CPU/Memory/Storage/Thermal physical layout
- Shows hardware state + constraints

**Data bridge:**
- Clicking process drills into which hardware it uses
- Clicking hardware shows which processes exploit/stress it
- Both views scroll horizontally showing timeline of activity
- Can pause/replay to see what was happening when lag occurred

---

## Hardware-Specific Optimizations Visible

Once you can see hardware in real-time, you can debug:
- **Unbalanced thread affinity** — see threads bouncing between cores
- **False sharing** — cache line bouncing between cores (miss spikes)
- **Thermal throttling** — frequency drops when hot
- **Power delivery limits** — current spikes cut off (brown-out)
- **Memory bandwidth exhaustion** — app stalls waiting for DRAM
- **Storage latency** — I/O completions delayed (bad seek patterns)
- **Interrupt storms** — IRQ lines lighting up constantly
- **Clock jitter** — oscillators drifting (power supply issue)

This visualization is **debugging+performance analysis baked into the OS itself.**


```
MAIN VIEW (3D Process Cloud):
  Arrow Keys       Rotate view
  Scroll/Pinch     Zoom in/out
  Mouse drag       Pan camera
  
  1-9              Select process (by PID mod 9)
  Enter            Focus on selected process (zoom in)
  Esc              Return to full system view
  'K'              Kill selected process
  'P'              Pin camera to selected process (follow)
  Space            Pause/resume animation
  'S'              Slow-motion mode (16× slower for inspection)
  'R'              Reset view (home position)
  
SUB-VIEWS (Toggles):
  'M'              Memory map (drill into heap/stack)
  'F'              Flamegraph timeline view
  'H'              Cache heatmap (thermal visualization)
  'I'              Instruction hotspot profiler
  'E'              Exception log (detailed)
  'X'              Syscall flow graph
  'C'              IPC communication matrix (table view)
  
FILTERING & SEARCH:
  '/'              Search process by name
  Ctrl+F           Filter view (show only matching processes)
  Ctrl+Z           Reset filters
  
UTILITIES:
  'T'              Toggle legend (control help)
  'Q'              Quit
  Ctrl+C           Force exit
```

---

## Syscalls Required

**Already available:**
- `SYS_PS` — process list
- `SYS_SYSINFO` — system state
- `SYS_MEMMAP` — virtual address mapping
- `SYS_BOOT_LOG` — kernel logs
- `SYS_RDTSC` — cycle counter
- `SYS_CLOCK` — wall time
- `SYS_KILL` — terminate process

**New syscalls to add (implement progressively):**
1. `SYS_PROC_STAT_EX` (new)
   - Returns extended process info: blocked_on_pid, signal_count, exception_count
   - Flags showing if process is faulting, signaled, etc.

2. `SYS_IPC_STATE` (new)
   - Returns IPC graph: list of (sender_pid, receiver_pid, queue_depth)
   - Allows visualizing communication topology

3. `SYS_EXCEPTION_LOG` (new)
   - Returns recent exceptions: (timestamp, type, pid, address)
   - Types: PageFault, DivByZero, IllegalInstr, GPFault, etc.

4. `SYS_IRQ_STATS` (new)
   - Returns per-IRQ statistics: (irq_num, count, total_cycles)
   - Allows visualizing interrupt frequency

5. `SYS_HEAP_MAP` (new)
   - Returns buddy allocator state: (order, allocated, free, fragmentation%)
   - Enables heap visualization

6. `SYS_PERF_SAMPLE` (new) — optional, for instruction profiling
   - Returns instruction frequency data (which x86 opcodes executed most)

---

## Architecture

```
system-visualizer/  ← NEW CRATE
├── Cargo.toml
└── src/
    ├── main.rs              // Entry, app loop
    ├── renderer.rs          // 3D scene rendering (uses gfx3d pipeline)
    ├── state.rs             // System state model + polling logic
    ├── camera.rs            // 3D camera control + navigation
    ├── geometry.rs          // Shape generation (spheres for processes, etc.)
    ├── syscalls.rs          // Wrappers around SYS_PS, SYS_SYSINFO, new syscalls
    ├── visualization/
    │   ├── mod.rs
    │   ├── processes.rs     // Process cloud rendering
    │   ├── memory.rs        // Heap/stack/memory region rendering
    │   ├── ipc.rs           // IPC graph lines
    │   ├── exceptions.rs    // Exception overlay
    │   ├── interrupts.rs    // IRQ lightning bolts
    │   ├── timeline.rs      // Flamegraph rendering
    │   └── heatmap.rs       // Thermal/cache visualization
    ├── input.rs             // Keyboard + mouse input handling
    ├── ui.rs                // 2D overlays (stats, labels, legend)
    └── animation.rs         // Easing, smooth transitions, particle effects
```

**Dependencies:**
- `libmorpheus` (syscalls, framebuffer)
- `gfx3d` (3D rendering pipeline)
- No external crates

**Build target:** `x86_64-morpheus.json` (same as spinning-cube)

---

## UI Controls & Interactions

```
MAIN VIEW (3D Process Cloud):
  Arrow Keys       Rotate view
  Scroll/Pinch     Zoom in/out
  Mouse drag       Pan camera
  
  1-9              Select process (by PID mod 9)
  Enter            Focus on selected process (zoom in)
  Esc              Return to full system view
  'K'              Kill selected process
  'P'              Pin camera to selected process (follow)
  Space            Pause/resume animation
  'S'              Slow-motion mode (16× slower for inspection)
  'R'              Reset view (home position)
  
SUB-VIEWS (Toggles):
  'M'              Memory map (drill into heap/stack)
  'F'              Flamegraph timeline view
  'H'              Cache heatmap (thermal visualization)
  'I'              Instruction hotspot profiler
  'E'              Exception log (detailed)
  'X'              Syscall flow graph
  'C'              IPC communication matrix (table view)
  'W'              Hardware topology view (toggle mode)
  
HARDWARE MODE (when 'W' active):
  Arrow Keys       Rotate hardware view
  Scroll/Pinch     Zoom in/out on specific component
  
  Click hardware   Drill-in to get detailed stats
  Esc              Back to process view
  'C'              Show cache hierarchy details
  'P'              Show power breakdown (all rails)
  'T'              Show temperature map (thermal gradient)
  'F'              Show frequency scaling (all cores)
  'M'              Show memory bank stats
  'S'              Show storage device stats
  'N'              Show network interface stats
  'I'              Show interrupt routing (APIC)
  'E'              Highlight errors/warnings only
  
FILTERING & SEARCH:
  '/'              Search process by name (in process mode)
  Ctrl+F           Filter view (show only matching processes)
  Ctrl+Z           Reset filters
  
UTILITIES:
  'T'              Toggle legend (control help)
  'Q'              Quit
  Ctrl+C           Force exit
```

---

## Architecture for Hardware Visualization

```
system-visualizer/  ← Updated
├── Cargo.toml
└── src/
    ├── main.rs
    ├── renderer.rs
    ├── state.rs
    ├── camera.rs
    ├── geometry.rs
    ├── syscalls.rs
    ├── visualization/
    │   ├── mod.rs
    │   ├── processes.rs
    │   ├── memory.rs
    │   ├── ipc.rs
    │   ├── exceptions.rs
    │   ├── interrupts.rs
    │   ├── timeline.rs
    │   ├── heatmap.rs
    │   └── hardware.rs       ← NEW: CPU, memory, storage, thermal, power
    ├── hardware/
    │   ├── mod.rs            ← NEW: Hardware data model
    │   ├── cpu.rs            ← NEW: CPU topology from CPUID
    │   ├── memory.rs         ← NEW: Memory subsystem
    │   ├── storage.rs        ← NEW: Storage devices
    │   ├── network.rs        ← NEW: NIC status
    │   ├── thermal.rs        ← NEW: Temperature zones
    │   └── power.rs          ← NEW: Voltage rails, power draw
    ├── input.rs
    ├── ui.rs
    └── animation.rs
```

---

## Phased Implementation (Updated Timeline)

### Phase 1 (Week 1-2): MVP — Process Cloud + Basic Stats
- Process spheres rendered in 3D
- CPU load aura (glowing shell)
- Real-time system load graph (Ready/Running/Blocked)
- Process selection + kill
- Camera controls (rotate, zoom)
- Ability to drill into one process (view memory map)

### Phase 2 (Week 3): IPC + Exceptions
- IPC lines between processes (animated messages)
- Deadlock detection (red blinking)
- Exception log overlay with real-time events
- Exception filter + replay

### Phase 3 (Week 4): Advanced Layers
- Interrupt heatmap (lightning bolts from edges)
- Flamegraph timeline view ('F' toggle)
- Cache thermal heatmap ('H' toggle)

### Phase 4 (Week 5): Hardware Visualization
- Hardware topology mode ('W' toggle)
- CPU package, memory DIMMs, storage drives, NIC, thermal zones
- Real-time hardware stats drill-in
- Power delivery visualization
- Process ↔ Hardware linking (click process to see which hardware)

### Phase 5 (Week 6): Polish & Advanced Hardware Features
- Temperature map ('T' in hardware mode)
- Frequency scaling visualization ('F' in hardware mode)
- Power breakdown by rail ('P' in hardware mode)
- Error detection & highlighting ('E' in hardware mode)
- Bidirectional navigation (process ↔ hardware ↔ process)
- Animation polish, performance tuning, legend, help

---

## Total Implementation Effort

- **MVP (Phase 1-2):** ~3 weeks — Shipping process visualization
- **Advanced (Phase 3-4):** ~2 weeks — Hardware visualization
- **Polish (Phase 5):** ~1 week — Refinement, optimization
- **Total:** ~6 weeks for full-featured 3D OS inspector

---

## Why the Hardware Visualizer Makes This Epic

**The killer combination:**
1. **Process cloud** shows OS soft behavior (what runs, IPC, exceptions)
2. **Hardware topology** shows physical constraints (CPU/memory/thermal/power)
3. **Bidirectional linking** lets you debug: "Why is app slow?" → see it's on thermally-throttled core
4. **Real-time 3D** makes bottlenecks *visible* instead of theoretical
5. **Baremetal uniqueness** — no Linux/Windows can do this natively

This is a **legitimate debugging tool** masquerading as a beautiful visualization. It turns MorpheusX into a system you can understand at a glance.

---




