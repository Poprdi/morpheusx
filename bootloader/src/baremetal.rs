//! The border. Before: UEFI. After: we own everything.
//! Stack switch → ExitBootServices → hwinit → framebuffer → TUI.

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, Ordering};
use morpheus_display::console::TextConsole;

// TYPES THAT CROSS THE BORDER

/// Raw framebuffer info from GOP. stride is pixels_per_scan_line from GOP.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FramebufferInfo {
    pub base: u64,
    pub size: usize,
    pub width: u32,
    pub height: u32,
    /// pixels_per_scan_line from GOP — NOT bytes. display crate wants bytes.
    pub stride: u32,
    /// 0=RGBX, 1=BGRX, 2=BitMask, 3=BltOnly
    pub format: u32,
}

/// Minimal config for the border crossing.
#[repr(C)]
pub struct BaremetalEntryConfig {
    pub image_handle: *mut (),
    pub system_table: *const (),
    pub framebuffer: FramebufferInfo,
}

// POST-EBS STATE

static mut FRAMEBUFFER_INFO: FramebufferInfo = FramebufferInfo {
    base: 0,
    size: 0,
    width: 0,
    height: 0,
    stride: 0,
    format: 0,
};

static BAREMETAL_MODE: AtomicBool = AtomicBool::new(false);

// live framebuffer console
// Initialized as soon as we have a valid framebuffer (before EBS).
// After that every puts() in hwinit mirrors to the screen in real-time.

static mut LIVE_CONSOLE: Option<TextConsole> = None;

/// Called by the hwinit serial hook for every byte emitted via `puts()`.
/// Writes directly to the TextConsole backed by the GOP framebuffer.
pub unsafe fn live_console_putc(b: u8) {
    if let Some(ref mut con) = LIVE_CONSOLE {
        con.write_char(b as char);
    }
}

/// Initialize the live framebuffer console and register it as the serial hook.
/// Call this once after FRAMEBUFFER_INFO is set and before any hwinit phases.
unsafe fn start_live_console(fb: &FramebufferInfo) {
    if fb.base == 0 || fb.width == 0 {
        return;
    }
    // GOP stride is pixels_per_scan_line; display crate wants stride in bytes.
    let stride_bytes = fb.stride * 4;
    let display_fb = morpheus_display::types::FramebufferInfo {
        base: fb.base,
        size: fb.size,
        width: fb.width,
        height: fb.height,
        stride: stride_bytes,
        format: match fb.format {
            0 => morpheus_display::types::PixelFormat::Rgbx,
            _ => morpheus_display::types::PixelFormat::Bgrx,
        },
    };
    let raw_fb = morpheus_display::framebuffer::Framebuffer::new(display_fb);
    let mut con = TextConsole::new(raw_fb);
    con.clear();
    LIVE_CONSOLE = Some(con);
    morpheus_hwinit::serial::set_live_console_hook(live_console_putc);
}

pub fn is_baremetal() -> bool {
    BAREMETAL_MODE.load(Ordering::Relaxed)
}

pub fn get_framebuffer_info() -> Option<FramebufferInfo> {
    if is_baremetal() {
        Some(unsafe { FRAMEBUFFER_INFO })
    } else {
        None
    }
}

// BSoD CRASH HOOK — called by exception handlers in hwinit

/// Callback invoked by hwinit exception handlers to display the crash screen.
///
/// # Safety
/// Called from exception context — no heap, no locks, framebuffer only.
unsafe fn bsod_crash_hook(info: &morpheus_hwinit::CrashInfo) {
    crate::bsod::show_crash_screen(info);
}

// THE ENTRY POINT — NEVER RETURNS

pub unsafe fn enter_baremetal(config: BaremetalEntryConfig) -> ! {
    use morpheus_hwinit::serial::puts;

    // First serial output — if you see this, our binary loaded and ran.
    puts("[MORPHEUSX] enter_baremetal\n");

    FRAMEBUFFER_INFO = config.framebuffer;

    // Start mirroring serial output to the framebuffer immediately so all
    // boot messages appear on screen in real-time, not just on COM1.
    start_live_console(&FRAMEBUFFER_INFO);

    // allocate stack from uefi (loaderdata survives ebs)
    const STACK_SIZE: usize = 256 * 1024;
    const STACK_PAGES: usize = STACK_SIZE.div_ceil(4096);

    #[repr(C)]
    struct MinSystemTable {
        _header: [u8; 24],
        _fw_vendor: *const u16,
        _fw_rev: u32,
        _cin_handle: *const (),
        _con_in: *mut (),
        _cout_handle: *const (),
        _con_out: *mut (),
        _stderr_handle: *const (),
        _stderr: *const (),
        _runtime: *const (),
        boot_services: *const MinBootServices,
    }

    #[repr(C)]
    struct MinBootServices {
        _header: [u8; 24],
        _raise_tpl: usize,
        _restore_tpl: usize,
        allocate_pages: extern "efiapi" fn(u32, u32, usize, *mut u64) -> usize,
        _free_pages: usize,
        get_memory_map:
            extern "efiapi" fn(*mut usize, *mut u8, *mut usize, *mut usize, *mut u32) -> usize,
        _alloc_pool: usize,
        _free_pool: usize,
        // slots 8..26 (create_event → unload_image) = 19 × 8 = 152 bytes
        // puts exit_boot_services at offset 232, matching UEFI spec exactly.
        _padding: [usize; 19],
        exit_boot_services: extern "efiapi" fn(*mut (), usize) -> usize,
    }

    let st = &*(config.system_table as *const MinSystemTable);
    let bs = &*st.boot_services;

    puts("[EBS-PREP] allocating stack\n");
    let mut stack_base: u64 = 0;
    let status = (bs.allocate_pages)(0, 2, STACK_PAGES, &mut stack_base);
    if status != 0 {
        puts("[FATAL] allocate_pages failed\n");
        loop {
            unsafe {
                core::arch::asm!("hlt", options(nomem, nostack));
            }
        }
    }
    let stack_top = stack_base + STACK_SIZE as u64;
    puts("[EBS-PREP] stack ready, getting memory map\n");

    // memory map + exitbootservices
    static mut MMAP_BUF: [u8; 32768] = [0u8; 32768];
    static mut MMAP_SIZE: usize = 0;
    static mut DESC_SIZE: usize = 0;
    static mut DESC_VER: u32 = 0;

    let mut map_size = MMAP_BUF.len();
    let mut map_key: usize = 0;
    let mut desc_size: usize = 0;
    let mut desc_ver: u32 = 0;

    let status = (bs.get_memory_map)(
        &mut map_size,
        MMAP_BUF.as_mut_ptr(),
        &mut map_key,
        &mut desc_size,
        &mut desc_ver,
    );
    if status != 0 {
        loop {
            unsafe {
                core::arch::asm!("hlt", options(nomem, nostack));
            }
        }
    }

    MMAP_SIZE = map_size;
    DESC_SIZE = desc_size;
    DESC_VER = desc_ver;

    puts("[EBS-PREP] mmap done, calling ExitBootServices\n");
    let status = (bs.exit_boot_services)(config.image_handle, map_key);
    if status != 0 {
        // EBS failed — either stale map key or wrong offset in MinBootServices.
        // This message will appear on serial before we loop.
        puts("[FATAL] ExitBootServices failed — status=");
        morpheus_hwinit::serial::put_hex64(status as u64);
        puts("\n");
        loop {
            unsafe {
                core::arch::asm!("hlt", options(nomem, nostack));
            }
        }
    }

    // UEFI IS DEAD. WE OWN THE MACHINE.
    //
    // Immediately disable interrupts. UEFI's PIC/APIC timer is still armed
    // and IF may be set. The UEFI IDT points into BootServicesCode which
    // ExitBootServices just freed (and OVMF DEBUG scrubs with 0xAF).
    // A single timer tick here vectors into 0xAFAFAF garbage, corrupts
    // whatever RDI points at, and poisons buddy FreeNode chains → #GP.
    // We'll re-enable interrupts in Phase 10 after our own GDT/IDT/PIC.
    core::arch::asm!("cli", options(nomem, nostack));

    BAREMETAL_MODE.store(true, Ordering::SeqCst);

    // Switch allocator to our own heap (UEFI pool is gone)
    crate::uefi_allocator::switch_to_post_ebs();

    // switch stack
    core::arch::asm!("mov rsp, {0}", in(reg) stack_top, options(nostack));

    puts("\n");
    puts("[MORPHEUS] machine is ours\n");

    // compute pe image bounds for buddy-allocator reservation
    // __ImageBase is defined by LLD for every PE/COFF binary.  From it we
    // read the PE optional header's SizeOfImage to determine the full
    // virtual extent (including BSS) that must be kept out of the free pool.
    extern "C" {
        static __ImageBase: u8;
    }
    let image_base = &__ImageBase as *const u8 as u64;
    let image_pages = {
        let pe_off_ptr = (image_base + 0x3C) as *const u32;
        let pe_off = core::ptr::read_unaligned(pe_off_ptr) as u64;
        // SizeOfImage is at offset 56 in the PE32+ optional header.
        // PE signature (4) + COFF header (20) + offset 56 into optional header = +80.
        let size_of_image_ptr = (image_base + pe_off + 4 + 20 + 56) as *const u32;
        let size_of_image = core::ptr::read_unaligned(size_of_image_ptr) as u64;
        size_of_image.div_ceil(4096)
    };

    // hwinit — gdt, idt, pic, heap, tsc, dma, bus mastering
    let hwinit_cfg = morpheus_hwinit::SelfContainedConfig {
        memory_map_ptr: MMAP_BUF.as_ptr(),
        memory_map_size: MMAP_SIZE,
        descriptor_size: DESC_SIZE,
        descriptor_version: DESC_VER,
        image_base,
        image_pages,
    };

    let platform = match morpheus_hwinit::platform_init_selfcontained(hwinit_cfg) {
        Ok(p) => {
            puts("[HWINIT] platform ready\n");

            // Register BSoD crash hook now that framebuffer info is available
            unsafe {
                morpheus_hwinit::set_crash_hook(bsod_crash_hook);
            }
            puts("[HWINIT] crash hook registered\n");

            p
        }
        Err(e) => {
            puts("[FATAL] hwinit: ");
            match e {
                morpheus_hwinit::InitError::InvalidDmaRegion => puts("bad DMA"),
                morpheus_hwinit::InitError::TscCalibrationFailed => puts("TSC dead"),
                morpheus_hwinit::InitError::NoFreeMemory => puts("no RAM"),
                morpheus_hwinit::InitError::MemoryRegistryFailed => puts("mmap broke"),
            }
            puts("\n");
            loop {
                unsafe {
                    core::arch::asm!("hlt", options(nomem, nostack));
                }
            }
        }
    };

    // persistent storage — try to mount a real block device
    crate::storage::init_persistent_storage(platform.dma(), platform.tsc_freq());

    // initfs — ensure standard directory structure exists
    crate::storage::create_init_directories();

    // framebuffer → screen
    let fb_info = FRAMEBUFFER_INFO;
    if fb_info.base == 0 || fb_info.width == 0 {
        puts("[FATAL] no framebuffer\n");
        loop {
            unsafe {
                core::arch::asm!("hlt", options(nomem, nostack));
            }
        }
    }

    // GOP gives pixels_per_scan_line, display crate wants bytes.
    // 4 bytes per pixel (32-bit RGBX/BGRX).
    let stride_bytes = fb_info.stride * 4;

    let display_info = morpheus_display::types::FramebufferInfo {
        base: fb_info.base,
        size: fb_info.size,
        width: fb_info.width,
        height: fb_info.height,
        stride: stride_bytes,
        format: match fb_info.format {
            0 => morpheus_display::types::PixelFormat::Rgbx,
            1 => morpheus_display::types::PixelFormat::Bgrx,
            _ => morpheus_display::types::PixelFormat::Bgrx, // sane default
        },
    };

    puts("[DISPLAY] framebuffer: ");
    morpheus_hwinit::serial::put_hex64(fb_info.width as u64);
    puts("x");
    morpheus_hwinit::serial::put_hex64(fb_info.height as u64);
    puts(" @ ");
    morpheus_hwinit::serial::put_hex64(fb_info.base);
    puts("\n");

    // Register framebuffer with the syscall layer so SYS_FB_INFO / SYS_FB_MAP work.
    morpheus_hwinit::register_framebuffer(morpheus_hwinit::FbInfo {
        base: fb_info.base,
        size: fb_info.size as u64,
        width: fb_info.width,
        height: fb_info.height,
        stride: stride_bytes,
        format: fb_info.format,
    });

    puts("[BAREMETAL] launching desktop\n");
    crate::tui::desktop::run_desktop(&display_info);
}
