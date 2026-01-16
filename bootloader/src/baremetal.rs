//! Bare-metal platform entry point.
//!
//! This module contains THE entry point to our bare-metal world.
//! Once `enter_baremetal()` is called, we NEVER return to UEFI.
//!
//! # Architecture
//!
//! ```text
//! efi_main (UEFI world)
//!     │
//!     ├── Gather GOP info (framebuffer)
//!     ├── Gather system table pointer
//!     │
//!     └── enter_baremetal() ──────────────────────────────────────────────┐
//!                                                                         │
//!         ┌───────────────────────────────────────────────────────────────┘
//!         │
//!         ▼  BARE-METAL WORLD (never returns)
//!         │
//!         ├── Allocate stack (from UEFI, as LoaderData)
//!         ├── ExitBootServices
//!         ├── Switch to our stack
//!         │
//!         ├── hwinit::platform_init_selfcontained()
//!         │   └── Memory, GDT, IDT, PIC, heap, TSC, DMA, bus mastering
//!         │
//!         ├── Initialize framebuffer display
//!         │
//!         └── Main TUI loop (forever)
//!             │
//!             ├── Render menus
//!             ├── Handle input
//!             ├── Download ISOs (network state machine)
//!             ├── Install distros (storage state machine)
//!             └── etc.
//! ```
//!
//! # The Hard Border
//!
//! This is an INVARIANT: nothing from pre-EBS orchestrates post-EBS.
//! We call EBS ourselves, we own everything after.
//!
//! The only data that crosses the border:
//! - UEFI system table pointer (to call EBS)
//! - GOP framebuffer info (address, resolution, format)
//! - Image handle (for EBS)

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, Ordering};

// ═══════════════════════════════════════════════════════════════════════════
// TYPES THAT CROSS THE BORDER (minimal, raw data only)
// ═══════════════════════════════════════════════════════════════════════════

/// Framebuffer info gathered from GOP before EBS.
/// This is raw data, no UEFI types.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FramebufferInfo {
    /// Physical address of framebuffer
    pub base: u64,
    /// Size in bytes
    pub size: usize,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
    /// Stride in pixels (pixels per scan line)
    pub stride: u32,
    /// Pixel format (0=RGBX, 1=BGRX, 2=BitMask, 3=BltOnly)
    pub format: u32,
}

/// Minimal config passed to bare-metal entry.
/// Raw pointers and values only - no UEFI types leak across.
#[repr(C)]
pub struct BaremetalEntryConfig {
    /// UEFI image handle (opaque, for EBS call)
    pub image_handle: *mut (),
    /// UEFI system table (opaque, for EBS call)
    pub system_table: *const (),
    /// GOP framebuffer info
    pub framebuffer: FramebufferInfo,
}

// ═══════════════════════════════════════════════════════════════════════════
// STATIC STATE (for post-EBS access)
// ═══════════════════════════════════════════════════════════════════════════

/// Framebuffer info, available after entering bare-metal.
static mut FRAMEBUFFER_INFO: FramebufferInfo = FramebufferInfo {
    base: 0,
    size: 0,
    width: 0,
    height: 0,
    stride: 0,
    format: 0,
};

/// Flag indicating we're in bare-metal mode.
static BAREMETAL_MODE: AtomicBool = AtomicBool::new(false);

/// Check if we're in bare-metal mode.
pub fn is_baremetal() -> bool {
    BAREMETAL_MODE.load(Ordering::Relaxed)
}

/// Get framebuffer info (only valid after entering bare-metal).
pub fn get_framebuffer_info() -> Option<FramebufferInfo> {
    if is_baremetal() {
        Some(unsafe { FRAMEBUFFER_INFO })
    } else {
        None
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// THE ENTRY POINT
// ═══════════════════════════════════════════════════════════════════════════

/// Enter bare-metal world. NEVER RETURNS.
///
/// This function:
/// 1. Allocates our own stack (from UEFI pool)
/// 2. Calls ExitBootServices
/// 3. Switches to our stack
/// 4. Runs hwinit
/// 5. Runs our platform forever
///
/// # Safety
/// - Must be called from UEFI context with valid system table
/// - NEVER RETURNS
/// - All UEFI resources become invalid after this
pub unsafe fn enter_baremetal(config: BaremetalEntryConfig) -> ! {
    use morpheus_hwinit::serial::puts;
    
    // Save framebuffer info for post-EBS access
    FRAMEBUFFER_INFO = config.framebuffer;
    
    // ─────────────────────────────────────────────────────────────────────
    // STEP 1: Allocate stack from UEFI (as LoaderData, survives EBS)
    // ─────────────────────────────────────────────────────────────────────
    const STACK_SIZE: usize = 256 * 1024; // 256KB stack
    const STACK_PAGES: usize = (STACK_SIZE + 4095) / 4096;
    
    // Get boot services from system table
    #[repr(C)]
    struct MinimalSystemTable {
        _header: [u8; 24],
        _firmware_vendor: *const u16,
        _firmware_revision: u32,
        _console_in_handle: *const (),
        _con_in: *mut (),
        _console_out_handle: *const (),
        _con_out: *mut (),
        _stderr_handle: *const (),
        _stderr: *const (),
        _runtime_services: *const (),
        boot_services: *const MinimalBootServices,
    }
    
    #[repr(C)]
    struct MinimalBootServices {
        _header: [u8; 24],
        _raise_tpl: usize,
        _restore_tpl: usize,
        allocate_pages: extern "efiapi" fn(
            alloc_type: u32,
            memory_type: u32,
            pages: usize,
            memory: *mut u64,
        ) -> usize,
        _free_pages: usize,
        get_memory_map: extern "efiapi" fn(
            memory_map_size: *mut usize,
            memory_map: *mut u8,
            map_key: *mut usize,
            descriptor_size: *mut usize,
            descriptor_version: *mut u32,
        ) -> usize,
        _allocate_pool: usize,
        _free_pool: usize,
        // ... many more fields before exit_boot_services
        _padding: [usize; 18], // Skip to exit_boot_services
        exit_boot_services: extern "efiapi" fn(
            image_handle: *mut (),
            map_key: usize,
        ) -> usize,
    }
    
    let st = &*(config.system_table as *const MinimalSystemTable);
    let bs = &*st.boot_services;
    
    // Allocate stack pages
    let mut stack_base: u64 = 0;
    let status = (bs.allocate_pages)(
        0, // AllocateAnyPages
        2, // EfiLoaderData (survives EBS)
        STACK_PAGES,
        &mut stack_base,
    );
    
    if status != 0 {
        // Fatal: can't allocate stack
        loop { core::hint::spin_loop(); }
    }
    
    let stack_top = stack_base + STACK_SIZE as u64;
    
    // ─────────────────────────────────────────────────────────────────────
    // STEP 2: Get memory map and call ExitBootServices
    // ─────────────────────────────────────────────────────────────────────
    
    // Static buffer for memory map (must survive stack switch)
    static mut MMAP_BUF: [u8; 32768] = [0u8; 32768];
    static mut MMAP_SIZE: usize = 0;
    static mut DESC_SIZE: usize = 0;
    static mut DESC_VERSION: u32 = 0;
    
    let mut map_size = MMAP_BUF.len();
    let mut map_key: usize = 0;
    let mut desc_size: usize = 0;
    let mut desc_version: u32 = 0;
    
    // Get memory map
    let status = (bs.get_memory_map)(
        &mut map_size,
        MMAP_BUF.as_mut_ptr(),
        &mut map_key,
        &mut desc_size,
        &mut desc_version,
    );
    
    if status != 0 {
        loop { core::hint::spin_loop(); }
    }
    
    // Save for post-EBS
    MMAP_SIZE = map_size;
    DESC_SIZE = desc_size;
    DESC_VERSION = desc_version;
    
    // Exit boot services
    let status = (bs.exit_boot_services)(config.image_handle, map_key);
    if status != 0 {
        // EBS failed - might need to re-get memory map
        // For now, just hang
        loop { core::hint::spin_loop(); }
    }
    
    // ═══════════════════════════════════════════════════════════════════════
    // POINT OF NO RETURN - UEFI IS GONE
    // ═══════════════════════════════════════════════════════════════════════
    
    BAREMETAL_MODE.store(true, Ordering::SeqCst);
    
    // ─────────────────────────────────────────────────────────────────────
    // STEP 3: Switch to our stack
    // ─────────────────────────────────────────────────────────────────────
    core::arch::asm!(
        "mov rsp, {0}",
        in(reg) stack_top,
        options(nostack)
    );
    
    // ─────────────────────────────────────────────────────────────────────
    // STEP 4: Enter platform (never returns)
    // ─────────────────────────────────────────────────────────────────────
    
    // Serial init for debug
    puts("\n");
    puts("╔══════════════════════════════════════════════════════════════╗\n");
    puts("║              MORPHEUS BARE-METAL PLATFORM                    ║\n");
    puts("║              UEFI is gone. We own the machine.               ║\n");
    puts("╚══════════════════════════════════════════════════════════════╝\n");
    puts("\n");
    
    // Run hwinit
    let hwinit_config = morpheus_hwinit::SelfContainedConfig {
        memory_map_ptr: MMAP_BUF.as_ptr(),
        memory_map_size: MMAP_SIZE,
        descriptor_size: DESC_SIZE,
        descriptor_version: DESC_VERSION,
    };
    
    let platform = match morpheus_hwinit::platform_init_selfcontained(hwinit_config) {
        Ok(p) => p,
        Err(e) => {
            puts("[FATAL] Platform init failed\n");
            loop { core::hint::spin_loop(); }
        }
    };
    
    // ─────────────────────────────────────────────────────────────────────
    // STEP 5: Initialize our display
    // ─────────────────────────────────────────────────────────────────────
    // TODO: Initialize framebuffer display driver
    // morpheus_display::init_framebuffer(FRAMEBUFFER_INFO);
    
    puts("[PLATFORM] Display: TODO - framebuffer init\n");
    
    // ─────────────────────────────────────────────────────────────────────
    // STEP 6: Main platform loop (forever)
    // ─────────────────────────────────────────────────────────────────────
    
    // TODO: This will be our TUI main loop running on our framebuffer
    // For now, just a placeholder
    
    puts("\n");
    puts("╔══════════════════════════════════════════════════════════════╗\n");
    puts("║              PLATFORM READY                                  ║\n");
    puts("║                                                              ║\n");
    puts("║  Next steps:                                                 ║\n");
    puts("║  1. Initialize framebuffer display                           ║\n");
    puts("║  2. Port TUI to our display                                  ║\n");
    puts("║  3. Implement input driver (PS/2 keyboard)                   ║\n");
    puts("║  4. Run menu system                                          ║\n");
    puts("╚══════════════════════════════════════════════════════════════╝\n");
    puts("\n");
    
    loop {
        core::hint::spin_loop();
    }
}
