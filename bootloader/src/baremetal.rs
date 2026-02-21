//! Bare-metal platform entry.
//!
//! enter_baremetal() is THE border. Before: UEFI. After: we own everything.
//!
//! Flow:
//!   allocate stack (UEFI) → EBS → stack switch → hwinit → framebuffer Screen → TUI
//!
//! Data that crosses the border (raw values only, no UEFI types):
//!   - image_handle, system_table (opaque ptrs, for EBS call)
//!   - GOP framebuffer info (address, resolution, format)

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, Ordering};

// ═══════════════════════════════════════════════════════════════════════════
// TYPES THAT CROSS THE BORDER
// ═══════════════════════════════════════════════════════════════════════════

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

// ═══════════════════════════════════════════════════════════════════════════
// POST-EBS STATE
// ═══════════════════════════════════════════════════════════════════════════

static mut FRAMEBUFFER_INFO: FramebufferInfo = FramebufferInfo {
    base: 0, size: 0, width: 0, height: 0, stride: 0, format: 0,
};

static BAREMETAL_MODE: AtomicBool = AtomicBool::new(false);

pub fn is_baremetal() -> bool {
    BAREMETAL_MODE.load(Ordering::Relaxed)
}

pub fn get_framebuffer_info() -> Option<FramebufferInfo> {
    if is_baremetal() { Some(unsafe { FRAMEBUFFER_INFO }) } else { None }
}

// ═══════════════════════════════════════════════════════════════════════════
// THE ENTRY POINT — NEVER RETURNS
// ═══════════════════════════════════════════════════════════════════════════

pub unsafe fn enter_baremetal(config: BaremetalEntryConfig) -> ! {
    use morpheus_hwinit::serial::puts;

    // First serial output — if you see this, our binary loaded and ran.
    puts("[MORPHEUSX] enter_baremetal\n");

    FRAMEBUFFER_INFO = config.framebuffer;

    // ── Allocate stack from UEFI (LoaderData survives EBS) ──────────────
    const STACK_SIZE: usize = 256 * 1024;
    const STACK_PAGES: usize = (STACK_SIZE + 4095) / 4096;

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
        get_memory_map: extern "efiapi" fn(
            *mut usize, *mut u8, *mut usize, *mut usize, *mut u32,
        ) -> usize,
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
        loop { core::hint::spin_loop(); }
    }
    let stack_top = stack_base + STACK_SIZE as u64;
    puts("[EBS-PREP] stack ready, getting memory map\n");

    // ── Memory map + ExitBootServices ───────────────────────────────────
    static mut MMAP_BUF: [u8; 32768] = [0u8; 32768];
    static mut MMAP_SIZE: usize = 0;
    static mut DESC_SIZE: usize = 0;
    static mut DESC_VER: u32 = 0;

    let mut map_size = MMAP_BUF.len();
    let mut map_key: usize = 0;
    let mut desc_size: usize = 0;
    let mut desc_ver: u32 = 0;

    let status = (bs.get_memory_map)(
        &mut map_size, MMAP_BUF.as_mut_ptr(), &mut map_key, &mut desc_size, &mut desc_ver,
    );
    if status != 0 { loop { core::hint::spin_loop(); } }

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
        loop { core::hint::spin_loop(); }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // UEFI IS DEAD. WE OWN THE MACHINE.
    // ═══════════════════════════════════════════════════════════════════════

    BAREMETAL_MODE.store(true, Ordering::SeqCst);

    // Switch allocator to our own heap (UEFI pool is gone)
    crate::uefi_allocator::switch_to_post_ebs();

    // ── Switch stack ────────────────────────────────────────────────────
    core::arch::asm!("mov rsp, {0}", in(reg) stack_top, options(nostack));

    puts("\n");
    puts("[MORPHEUS] machine is ours\n");

    // ── hwinit — GDT, IDT, PIC, heap, TSC, DMA, bus mastering ─────────
    let hwinit_cfg = morpheus_hwinit::SelfContainedConfig {
        memory_map_ptr: MMAP_BUF.as_ptr(),
        memory_map_size: MMAP_SIZE,
        descriptor_size: DESC_SIZE,
        descriptor_version: DESC_VER,
    };

    let _platform = match morpheus_hwinit::platform_init_selfcontained(hwinit_cfg) {
        Ok(p) => {
            puts("[HWINIT] platform ready\n");
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
            loop { core::hint::spin_loop(); }
        }
    };

    // ── Framebuffer → Screen ────────────────────────────────────────────
    let fb_info = FRAMEBUFFER_INFO;
    if fb_info.base == 0 || fb_info.width == 0 {
        puts("[FATAL] no framebuffer\n");
        loop { core::hint::spin_loop(); }
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

    let mut screen = crate::tui::renderer::Screen::from_framebuffer(display_info);
    let mut keyboard = crate::tui::input::Keyboard::new();

    puts("[TUI] screen ready, launching main menu\n");

    // ── Main TUI loop ──────────────────────────────────────────────────
    // MainMenu is pure Screen+Keyboard — zero UEFI dependencies.
    // Sub-menus (downloader, installer, storage) need their own bare-metal
    // I/O drivers before they work. For now: main menu renders and loops.
    use crate::tui::main_menu::{MainMenu, MenuAction};

    let mut menu = MainMenu::new(&screen);

    loop {
        let action = menu.run(&mut screen, &mut keyboard);
        match action {
            MenuAction::ExitToFirmware => {
                // No firmware to exit to. We ARE the firmware now.
                screen.clear();
                screen.put_str_at(
                    screen.center_x(40), screen.center_y(1),
                    "there is no firmware. only morpheus.",
                    crate::tui::renderer::EFI_GREEN,
                    crate::tui::renderer::EFI_BLACK,
                );
                // Re-render menu after a beat
            }
            _ => {
                // Sub-menus need bare-metal I/O — not wired yet.
                screen.clear();
                screen.put_str_at(
                    screen.center_x(30), screen.center_y(1),
                    "not wired yet. patience.",
                    crate::tui::renderer::EFI_YELLOW,
                    crate::tui::renderer::EFI_BLACK,
                );
            }
        }
    }
}
