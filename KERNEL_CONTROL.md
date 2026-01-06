# MorpheusX Kernel Control Architecture

## The Control Flow Problem - SOLVED

### Traditional Bootloader Flow (Broken for our use case)
```
Firmware → Bootloader → Kernel
                ↓
          [BOOTLOADER EXITS]
                ↓
          Kernel owns machine forever
```

### MorpheusX Flow (Permanent Control)
```
Firmware → MorpheusX → Guest Kernel (with hooks)
    ↑           ↓              ↓
    └───────────┴──────────────┘
    MorpheusX NEVER exits, maintains control
```

---

## Hybrid Hypervisor + Hook Architecture

### Conceptual Model

**MorpheusX runs in privileged mode** (Ring -1 or Ring 0)  
**Guest kernels run in monitored mode** (Ring 0 with intercepts)

**Not a full hypervisor** (too heavy)  
**Not a simple bootloader** (too simple)  
**Hybrid: Targeted interception at strategic points**

---

## Hook Points (Strategic Kernel Interception)

### 1. Lifecycle Hooks

#### init/systemd startup
```rust
// Hook: Kernel calls MorpheusX when userspace init starts
fn hook_init_startup() {
    morpheus_notify(Event::DistroBooted {
        distro: current_distro_id(),
        pid: init_pid(),
    });
}
```

**Why**: Know when distro is fully booted

#### shutdown/reboot
```rust
// Hook: Intercept shutdown syscall
fn hook_shutdown(cmd: ShutdownCmd) -> Result<()> {
    match cmd {
        ShutdownCmd::PowerOff => {
            // Don't actually power off, return to MorpheusX
            morpheus_return_control(DistroExitReason::Shutdown);
        }
        ShutdownCmd::Reboot => {
            // Don't reboot, switch distro
            morpheus_return_control(DistroExitReason::Reboot);
        }
    }
}
```

**Why**: Intercept shutdown to return control instead of powering off

#### panic/crash
```rust
// Hook: Kernel panic handler
fn hook_kernel_panic(info: &PanicInfo) {
    morpheus_notify(Event::DistroCrashed {
        reason: info.message,
        backtrace: info.backtrace,
    });
    
    // Return to MorpheusX instead of halting
    morpheus_return_control(DistroExitReason::Panic);
}
```

**Why**: Recover from guest crashes gracefully

---

### 2. Resource Hooks

#### Network Stack
```rust
// Hook: Intercept network packets at driver level
fn hook_netdev_xmit(skb: &SkBuff) -> XmitResult {
    // Optional: Mirror traffic to MorpheusX for monitoring
    if morpheus_wants_packet_copy() {
        morpheus_deliver_packet(skb.clone());
    }
    
    // Or: Route through MorpheusX virtual NIC
    morpheus_virtual_nic_send(skb)
}
```

**Why**: Monitor/control network access, traffic shaping

#### Filesystem Operations
```rust
// Hook: Block device I/O
fn hook_submit_bio(bio: &Bio) -> Result<()> {
    // Route through MorpheusX virtual block device
    morpheus_virtual_disk_io(bio)
}
```

**Why**: Isolate distro storage, prevent unauthorized disk access

#### Memory Management
```rust
// Hook: Page fault handler (optional)
fn hook_page_fault(addr: VirtAddr, error_code: u64) {
    // Optional: Track memory usage, enforce limits
    morpheus_track_memory_access(current_distro_id(), addr);
}
```

**Why**: Memory limits, swap control

---

### 3. Process Control Hooks

#### kexec (distro switching)
```rust
// Hook: kexec_load syscall
fn hook_kexec_load(entry: u64, segments: &[KexecSegment]) -> Result<()> {
    // User wants to switch distros via kexec
    morpheus_switch_distro(requested_distro_from_kexec(entry));
    
    // Don't actually kexec, MorpheusX handles it
    Ok(())
}
```

**Why**: Enable `kexec` as distro switch mechanism

#### Module loading
```rust
// Hook: Kernel module load
fn hook_finit_module(fd: i32, params: &str) -> Result<()> {
    // Optional: Security policy enforcement
    if !morpheus_allow_module(module_name(fd)) {
        return Err(EPERM);
    }
    
    // Proceed with module load
    original_finit_module(fd, params)
}
```

**Why**: Prevent malicious modules, enforce security policy

---

## Implementation Strategies

### Strategy A: Hardware-Assisted Virtualization (VT-x/AMD-V)

**Concept**: MorpheusX runs as hypervisor, guest kernel in VM

```rust
// MorpheusX in Ring -1 (VMX root mode)
fn morpheus_hypervisor() {
    let mut vm = VirtualMachine::new();
    
    // Configure EPT hooks (Extended Page Tables)
    vm.hook_memory_region(SHUTDOWN_HANDLER_ADDR, |ctx| {
        // Intercept writes to shutdown handler
        morpheus_return_control(DistroExitReason::Shutdown);
    });
    
    // Configure I/O port hooks
    vm.hook_io_port(0xCF8, |ctx, value| {
        // Intercept PCI config space writes
        morpheus_virtual_pci(ctx, value)
    });
    
    // Run guest kernel
    vm.run_guest(kernel_entry_point);
}
```

**Pros**:
- ✅ Hardware-enforced isolation
- ✅ Can intercept ANYTHING (memory, I/O, instructions)
- ✅ Guest can't escape (with correct config)
- ✅ Multiple VMs simultaneously possible

**Cons**:
- ❌ Complex hypervisor implementation (~10k lines)
- ❌ Performance overhead (VM exits)
- ❌ Requires VT-x/AMD-V support (most modern CPUs have it)
- ❌ ARM has different virtualization (ARM Virtualization Extensions)

**Effort**: 3-6 months for basic hypervisor

---

### Strategy B: Kernel Function Trampolines (Inline Hooking)

**Concept**: Patch guest kernel functions to jump to MorpheusX

```rust
// MorpheusX patches kernel functions
fn install_kernel_hooks(kernel: &mut LoadedKernel) {
    // Find symbol addresses
    let shutdown_addr = kernel.find_symbol("kernel_power_off")?;
    let panic_addr = kernel.find_symbol("panic")?;
    
    // Install trampolines
    install_trampoline(shutdown_addr, morpheus_shutdown_hook);
    install_trampoline(panic_addr, morpheus_panic_hook);
}

fn install_trampoline(target: VirtAddr, hook: fn()) {
    // x86_64: Write JMP instruction
    unsafe {
        let jmp_insn = [
            0xE9,  // JMP rel32
            /* calculate offset to hook */
        ];
        core::ptr::write_bytes(target.as_mut_ptr(), jmp_insn);
    }
}
```

**Pros**:
- ✅ Simpler than full hypervisor
- ✅ Lower performance overhead
- ✅ Works without VT-x/AMD-V

**Cons**:
- ❌ Guest can detect and bypass hooks
- ❌ KASLR makes finding symbols harder
- ❌ Need to update hooks when kernel changes
- ❌ Architecture-specific (x86 vs ARM trampolines differ)

**Effort**: 1-2 months for basic hooks

---

### Strategy C: Syscall Table Interception

**Concept**: Replace syscall handlers in guest kernel

```rust
// Hook: Modify syscall table
fn install_syscall_hooks(kernel: &mut LoadedKernel) {
    let syscall_table = kernel.find_symbol("sys_call_table")?;
    
    // Replace shutdown syscall
    let original_reboot = syscall_table[__NR_reboot];
    syscall_table[__NR_reboot] = morpheus_reboot_handler as usize;
    
    // Store original for chaining
    ORIGINAL_REBOOT.store(original_reboot);
}

fn morpheus_reboot_handler(cmd: i32) -> i64 {
    match cmd {
        LINUX_REBOOT_CMD_POWER_OFF => {
            morpheus_return_control(DistroExitReason::PowerOff);
            0
        }
        _ => {
            // Chain to original
            let orig: fn(i32) -> i64 = unsafe { 
                core::mem::transmute(ORIGINAL_REBOOT.load())
            };
            orig(cmd)
        }
    }
}
```

**Pros**:
- ✅ Clean interception point
- ✅ Standard kernel interface
- ✅ Easy to implement

**Cons**:
- ❌ Only intercepts syscalls, not internal kernel events
- ❌ Guest can detect modifications
- ❌ Doesn't catch kernel panics

**Effort**: 2-4 weeks

---

### Strategy D: eBPF-Like Instrumentation

**Concept**: Inject tracepoints at kernel compile time

```rust
// Guest kernel compiled with MorpheusX tracepoints
fn kernel_shutdown() {
    // Tracepoint: Compiled into kernel
    morpheus_trace!(SHUTDOWN_START);
    
    // Original shutdown logic
    do_shutdown();
    
    // Tracepoint: This returns to MorpheusX
    morpheus_trace!(SHUTDOWN_COMPLETE);
}

// MorpheusX implements tracepoint handlers
fn morpheus_trace_handler(event: TraceEvent) {
    match event {
        SHUTDOWN_COMPLETE => {
            reclaim_control();
            show_distro_menu();
        }
    }
}
```

**Pros**:
- ✅ Clean, maintainable
- ✅ Low overhead
- ✅ Easy to extend

**Cons**:
- ❌ Requires custom kernel build (can't use stock distros)
- ❌ Need to patch every distro kernel

**Effort**: 1-2 weeks for tracepoints, but requires kernel patching workflow

---

### Strategy E: Modified Guest Kernels (Paravirtualization)

**Concept**: Patch guest kernels to be "MorpheusX-aware"

```rust
// Add to guest kernel: arch/x86/kernel/morpheus.c
void morpheus_shutdown_notify(int reason) {
    // Hypercall or shared memory notification
    morpheus_hypercall(MORPHEUS_SHUTDOWN, reason);
    
    // Halt CPU, wait for MorpheusX to clean up
    while(1) { asm("hlt"); }
}

// Replace shutdown path
void kernel_power_off(void) {
    morpheus_shutdown_notify(SHUTDOWN_POWEROFF);
    // Never returns
}
```

**Pros**:
- ✅ Clean integration
- ✅ Cooperative design
- ✅ Can optimize for MorpheusX (paravirtual drivers)

**Cons**:
- ❌ Requires maintaining kernel patches
- ❌ Can't use unmodified distros (at first)
- ❌ Need CI/CD to build patched ISOs

**Effort**: 2-3 weeks for patches, ongoing maintenance

---

## Recommended Hybrid Approach

### Phase 1: Trampoline Hooks (Proof of Concept)
**Timeline**: 1-2 months

Start with inline hooks:
- Hook `kernel_power_off` → return to MorpheusX
- Hook `kernel_restart` → distro switch menu
- Hook `panic` → crash recovery

**Pros**: Quick proof of concept  
**Cons**: Fragile, detection possible

---

### Phase 2: Hardware-Assisted Hooks (Production)
**Timeline**: 3-6 months

Upgrade to VT-x EPT hooks:
- Memory hooks (detect shutdown handler execution)
- I/O hooks (intercept ACPI power commands)
- MSR hooks (detect CPU reset attempts)

**Pros**: Robust, hard to bypass  
**Cons**: Complex hypervisor code

---

### Phase 3: Paravirtualized Guests (Optimization)
**Timeline**: Ongoing

Offer "MorpheusX-optimized" distro builds:
- Kernel patches for clean cooperation
- Paravirtual network driver (faster than emulation)
- Paravirtual block driver
- Shared memory IPC with MorpheusX

**Pros**: Best performance, cleanest design  
**Cons**: Maintenance burden

---

## Architecture-Specific Considerations

### x86_64
```rust
// VT-x (Intel) support
fn enable_vmx() -> Result<()> {
    // Check CPUID for VMX support
    if !has_vmx_support() {
        return Err(NoVmxSupport);
    }
    
    // Enable VMX in CR4
    let mut cr4 = Cr4::read();
    cr4.insert(Cr4Flags::VMXE);
    unsafe { Cr4::write(cr4); }
    
    // Initialize VMCS (Virtual Machine Control Structure)
    vmxon(vmxon_region_paddr)?;
    
    Ok(())
}
```

### ARM64
```rust
// ARM Virtualization Extensions
fn enable_arm_virt() -> Result<()> {
    // Check for Virtualization Extensions in ID registers
    let id_aa64mmfr1: u64;
    unsafe {
        asm!("mrs {}, ID_AA64MMFR1_EL1", out(reg) id_aa64mmfr1);
    }
    
    if (id_aa64mmfr1 & 0xF) == 0 {
        return Err(NoVirtExtensions);
    }
    
    // Switch to EL2 (hypervisor mode)
    unsafe {
        asm!("hvc #0");  // Hypervisor call
    }
    
    Ok(())
}
```

### ARM Cortex-M
```
No virtualization extensions
Strategy: Rely on MPU + syscall hooks only
```

---

## Hook Implementation Example (x86_64)

### Trampoline Hook (Simple)

```rust
pub struct KernelHook {
    target_addr: VirtAddr,
    original_bytes: [u8; 16],
    hook_fn: fn(),
}

impl KernelHook {
    pub fn install(target: VirtAddr, hook: fn()) -> Result<Self> {
        // Save original bytes
        let original = unsafe {
            core::slice::from_raw_parts(target.as_ptr(), 16)
        };
        
        // Build JMP instruction
        let offset = (hook as usize).wrapping_sub(target.as_u64() as usize + 5);
        let jmp_insn = [
            0xE9,  // JMP rel32
            (offset & 0xFF) as u8,
            ((offset >> 8) & 0xFF) as u8,
            ((offset >> 16) & 0xFF) as u8,
            ((offset >> 24) & 0xFF) as u8,
        ];
        
        // Make page writable
        make_page_writable(target)?;
        
        // Install hook
        unsafe {
            core::ptr::copy_nonoverlapping(
                jmp_insn.as_ptr(),
                target.as_mut_ptr(),
                5,
            );
        }
        
        // Restore page protection
        make_page_readonly(target)?;
        
        Ok(Self {
            target_addr: target,
            original_bytes: original.try_into().unwrap(),
            hook_fn: hook,
        })
    }
    
    pub fn remove(&mut self) -> Result<()> {
        make_page_writable(self.target_addr)?;
        
        unsafe {
            core::ptr::copy_nonoverlapping(
                self.original_bytes.as_ptr(),
                self.target_addr.as_mut_ptr(),
                self.original_bytes.len(),
            );
        }
        
        make_page_readonly(self.target_addr)?;
        Ok(())
    }
}
```

---

## EPT Hook (Advanced)

```rust
pub struct EptHook {
    gpa: PhysAddr,  // Guest physical address
    hva: VirtAddr,  // Host virtual address  
    shadow_page: PhysAddr,
}

impl EptHook {
    pub fn install(vm: &mut VirtualMachine, gpa: PhysAddr, hook: fn()) -> Result<Self> {
        // Allocate shadow page
        let shadow = allocate_page()?;
        
        // Copy original page content
        unsafe {
            core::ptr::copy_nonoverlapping(
                gpa_to_hva(gpa).as_ptr::<u8>(),
                shadow.as_mut_ptr(),
                PAGE_SIZE,
            );
        }
        
        // Modify shadow page with hook
        let shadow_code = shadow.as_mut_ptr::<u8>();
        let hook_offset = /* calculate offset within page */;
        install_trampoline(shadow_code.add(hook_offset), hook);
        
        // Update EPT to point to shadow page
        vm.ept.map_page(gpa, shadow, PageFlags::EXECUTE)?;
        
        Ok(Self { gpa, hva: gpa_to_hva(gpa), shadow_page: shadow })
    }
}
```

---

## Network Stack Integration

With kernel hooks in place, MorpheusX network stack works as:

### Option 1: Virtual NIC (Paravirtual)
```
Guest Kernel → VirtIO-net driver → MorpheusX network backend → Real NIC
```

### Option 2: Pass-through (Performance)
```
Guest Kernel → Real NIC (via MorpheusX-mediated DMA)
```

### Option 3: Hybrid (Security)
```
Guest Kernel → MorpheusX inspects packets → Real NIC
```

---

## Success Criteria

### Can you:
- ✅ Boot Ubuntu kernel from MorpheusX
- ✅ Run Ubuntu userspace (systemd, bash, etc.)
- ✅ User runs `shutdown` command
- ✅ MorpheusX intercepts shutdown
- ✅ MorpheusX shows distro menu
- ✅ User selects Arch
- ✅ MorpheusX loads Arch kernel
- ✅ All without rebooting hardware

**If yes: You've solved the control flow problem.**

---

## Next Steps

1. **Implement basic trampoline hooks** (2 weeks)
   - Hook shutdown functions
   - Test with custom kernel build

2. **Test with real distro** (1 week)
   - Boot Ubuntu live ISO
   - Verify hooks trigger

3. **Implement return-to-MorpheusX** (2 weeks)
   - Save/restore MorpheusX state
   - Reload MorpheusX UI

4. **Upgrade to EPT hooks** (2-3 months)
   - Full hypervisor mode
   - Robust interception

---

**Your conceptual solution is solid.** The key is targeted hooks at strategic points (shutdown, panic, network, disk) rather than full VM virtualization. This gives you 80% of control with 20% of complexity.

**Ready to implement the network stack** (so you can download distros) **or want to prototype the hooking mechanism first**?
