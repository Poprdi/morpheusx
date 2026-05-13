//! The border. Before: UEFI. After: we own everything.
//! Stack switch → ExitBootServices → hwinit → framebuffer → TUI.

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, Ordering};
use morpheus_display::console::TextConsole;

use crate::baremetal_ops::network;

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
static mut PRE_EBS_HELIX_BASE: u64 = 0;
static mut PRE_EBS_HELIX_SIZE: usize = 0;
static mut PRE_EBS_HELIX_SECTOR_SIZE: u32 = 512;

const LOADED_IMAGE_PROTOCOL_GUID: [u8; 16] = [
    0xA1, 0x31, 0x1B, 0x5B, 0x62, 0x95, 0xD2, 0x11, 0x8E, 0x3F, 0x00, 0xA0, 0xC9, 0x69,
    0x72, 0x3B,
];
const SIMPLE_FILE_SYSTEM_PROTOCOL_GUID: [u8; 16] = [
    0x22, 0x5B, 0x4E, 0x96, 0x59, 0x64, 0xD2, 0x11, 0x8E, 0x39, 0x00, 0xA0, 0xC9, 0x69,
    0x72, 0x3B,
];
const ACPI_20_TABLE_GUID: [u8; 16] = [
    0x71, 0xE8, 0x68, 0x88, 0xF1, 0xE4, 0xD3, 0x11, 0xBC, 0x22, 0x00, 0x80, 0xC7, 0x3C,
    0x88, 0x81,
];
const ACPI_10_TABLE_GUID: [u8; 16] = [
    0x30, 0x2D, 0x9D, 0xEB, 0x88, 0x2D, 0xD3, 0x11, 0x9A, 0x16, 0x00, 0x90, 0x27, 0x3F,
    0xC1, 0x4D,
];
const PRE_EBS_STAGE_MAX_BYTES: u64 = 512 * 1024 * 1024;
const HELIX_IMG_SECTOR_SIZE: u32 = 512;
const EFI_FILE_MODE_READ: u64 = 0x0000_0000_0000_0001;
const HELIX_IMG_PATH: [u16; 20] = [
    b'\\' as u16,
    b'm' as u16,
    b'o' as u16,
    b'r' as u16,
    b'p' as u16,
    b'h' as u16,
    b'e' as u16,
    b'u' as u16,
    b's' as u16,
    b'\\' as u16,
    b'h' as u16,
    b'e' as u16,
    b'l' as u16,
    b'i' as u16,
    b'x' as u16,
    b'.' as u16,
    b'i' as u16,
    b'm' as u16,
    b'g' as u16,
    0,
];

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

pub unsafe fn take_pre_ebs_helix_image() -> Option<(u64, usize, u32)> {
    if PRE_EBS_HELIX_BASE == 0 || PRE_EBS_HELIX_SIZE == 0 {
        return None;
    }
    let out = (
        PRE_EBS_HELIX_BASE,
        PRE_EBS_HELIX_SIZE,
        PRE_EBS_HELIX_SECTOR_SIZE,
    );
    PRE_EBS_HELIX_BASE = 0;
    PRE_EBS_HELIX_SIZE = 0;
    Some(out)
}

// BSoD CRASH HOOK — called by exception handlers in hwinit

/// Callback invoked by hwinit exception handlers to display the crash screen.
///
/// # Safety
/// Called from exception context — no heap, no locks, framebuffer only.
unsafe fn bsod_crash_hook(info: &morpheus_hwinit::CrashInfo) {
    crate::bsod::show_crash_screen(info);
}

#[inline(always)]
unsafe fn pe_image_size(image_base: u64) -> u64 {
    let pe_off_ptr = (image_base + 0x3C) as *const u32;
    let pe_off = core::ptr::read_unaligned(pe_off_ptr) as u64;
    // SizeOfImage is at offset 56 in the PE32+ optional header.
    // PE signature (4) + COFF header (20) + offset 56 into optional header = +80.
    let size_of_image_ptr = (image_base + pe_off + 4 + 20 + 56) as *const u32;
    core::ptr::read_unaligned(size_of_image_ptr) as u64
}

#[repr(C)]
struct EfiLoadedImage {
    _revision: u32,
    _parent_handle: *mut (),
    _system_table: *mut (),
    device_handle: *mut (),
}

#[repr(C)]
struct EfiSimpleFileSystem {
    _revision: u64,
    open_volume:
        extern "efiapi" fn(*mut EfiSimpleFileSystem, *mut *mut EfiFileProtocol) -> usize,
}

#[repr(C)]
struct EfiConfigurationTable {
    vendor_guid: [u8; 16],
    vendor_table: *const (),
}

#[repr(C)]
struct EfiFileProtocol {
    _revision: u64,
    open: extern "efiapi" fn(
        *mut EfiFileProtocol,
        *mut *mut EfiFileProtocol,
        *const u16,
        u64,
        u64,
    ) -> usize,
    close: extern "efiapi" fn(*mut EfiFileProtocol) -> usize,
    _delete: usize,
    read: extern "efiapi" fn(*mut EfiFileProtocol, *mut usize, *mut u8) -> usize,
    _write: usize,
    get_position: extern "efiapi" fn(*mut EfiFileProtocol, *mut u64) -> usize,
    set_position: extern "efiapi" fn(*mut EfiFileProtocol, u64) -> usize,
}

// THE ENTRY POINT — NEVER RETURNS

pub unsafe fn enter_baremetal(config: BaremetalEntryConfig) -> ! {
    use morpheus_hwinit::serial::{log_error, log_info, log_ok, log_warn, puts};

    // First serial output — if you see this, our binary loaded and ran.
    log_info("BOOT", 901, "enter baremetal");

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
        number_of_table_entries: usize,
        configuration_table: *const EfiConfigurationTable,
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
        _create_event: usize,
        _set_timer: usize,
        _wait_for_event: usize,
        _signal_event: usize,
        _close_event: usize,
        _check_event: usize,
        _install_protocol_interface: usize,
        _reinstall_protocol_interface: usize,
        _uninstall_protocol_interface: usize,
        handle_protocol:
            extern "efiapi" fn(*mut (), *const [u8; 16], *mut *mut ()) -> usize,
        _reserved: usize,
        _register_protocol_notify: usize,
        _locate_handle: usize,
        _locate_device_path: usize,
        _install_configuration_table: usize,
        _load_image: usize,
        _start_image: usize,
        _exit: usize,
        _unload_image: usize,
        exit_boot_services: extern "efiapi" fn(*mut (), usize) -> usize,
    }

    let st = &*(config.system_table as *const MinSystemTable);
    let bs = &*st.boot_services;

    let mut acpi_rsdp_phys = 0u64;
    if !st.configuration_table.is_null() {
        let tables = core::slice::from_raw_parts(st.configuration_table, st.number_of_table_entries);
        for t in tables {
            if t.vendor_guid == ACPI_20_TABLE_GUID {
                acpi_rsdp_phys = t.vendor_table as u64;
                break;
            }
        }
        if acpi_rsdp_phys == 0 {
            for t in tables {
                if t.vendor_guid == ACPI_10_TABLE_GUID {
                    acpi_rsdp_phys = t.vendor_table as u64;
                    break;
                }
            }
        }
    }
    if acpi_rsdp_phys != 0 {
        log_ok("ACPI", 901, "found RSDP via UEFI config table");
    } else {
        log_warn("ACPI", 901, "RSDP not present in UEFI config table");
    }

    // We still need this for hwinit image reservation post-EBS.
    extern "C" {
        static __ImageBase: u8;
    }
    let image_base = &__ImageBase as *const u8 as u64;

    log_info("EBS", 902, "allocating kernel stack");
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
    log_info("EBS", 903, "stack ready; fetching memory map");

    // Pre-EBS media stage: load /morpheus/helix.img from ESP into RAM.
    let mut loaded_image_ptr: *mut () = core::ptr::null_mut();
    let li_status = (bs.handle_protocol)(
        config.image_handle,
        &LOADED_IMAGE_PROTOCOL_GUID,
        &mut loaded_image_ptr,
    );
    if li_status == 0 && !loaded_image_ptr.is_null() {
        let li = &*(loaded_image_ptr as *const EfiLoadedImage);
        let mut sfs_ptr: *mut () = core::ptr::null_mut();
        let sfs_status = (bs.handle_protocol)(
            li.device_handle,
            &SIMPLE_FILE_SYSTEM_PROTOCOL_GUID,
            &mut sfs_ptr,
        );
        if sfs_status == 0 && !sfs_ptr.is_null() {
            let sfs = sfs_ptr as *mut EfiSimpleFileSystem;
            let mut root: *mut EfiFileProtocol = core::ptr::null_mut();
            if ((*sfs).open_volume)(sfs, &mut root) == 0 && !root.is_null() {
                let mut img: *mut EfiFileProtocol = core::ptr::null_mut();
                let open_rc = ((*root).open)(
                    root,
                    &mut img,
                    HELIX_IMG_PATH.as_ptr(),
                    EFI_FILE_MODE_READ,
                    0,
                );
                if open_rc == 0 && !img.is_null() {
                    // Position at EOF to query file size, then rewind.
                    let _ = ((*img).set_position)(img, u64::MAX);
                    let mut file_size = 0u64;
                    let _ = ((*img).get_position)(img, &mut file_size);
                    let _ = ((*img).set_position)(img, 0);

                    if file_size > 0 && file_size <= PRE_EBS_STAGE_MAX_BYTES {
                        let mut alloc_bytes = file_size as usize;
                        let rem = alloc_bytes % HELIX_IMG_SECTOR_SIZE as usize;
                        if rem != 0 {
                            alloc_bytes += (HELIX_IMG_SECTOR_SIZE as usize) - rem;
                        }
                        let mut stage_base: u64 = 0;
                        let stage_pages = alloc_bytes.div_ceil(4096);
                        let alloc_rc = (bs.allocate_pages)(0, 2, stage_pages, &mut stage_base);
                        if alloc_rc == 0 && stage_base != 0 {
                            let mut off = 0usize;
                            let mut ok = true;
                            while off < file_size as usize {
                                let remaining = (file_size as usize) - off;
                                let mut want = core::cmp::min(1024 * 1024, remaining);
                                let rc = ((*img).read)(
                                    img,
                                    &mut want,
                                    (stage_base as *mut u8).add(off),
                                );
                                if rc != 0 {
                                    ok = false;
                                    break;
                                }
                                // want == 0 means EOF — nothing more to read.
                                if want == 0 {
                                    break;
                                }
                                off += want;
                                // don't break on short reads — some UEFI firmware caps per-call
                                // transfer size. just keep looping with what we got.
                            }

                            if ok && off > 0 {
                                let usable = off - (off % HELIX_IMG_SECTOR_SIZE as usize);
                                if usable > 0 {
                                    PRE_EBS_HELIX_BASE = stage_base;
                                    PRE_EBS_HELIX_SIZE = usable;
                                    PRE_EBS_HELIX_SECTOR_SIZE = HELIX_IMG_SECTOR_SIZE;
                                    log_ok("EBS", 901, "pre-EBS loaded /morpheus/helix.img to RAM");
                                } else {
                                    log_warn("EBS", 901, "helix.img size not sector-aligned");
                                }
                            } else {
                                log_warn("EBS", 901, "failed reading /morpheus/helix.img");
                            }
                        } else {
                            log_warn("EBS", 901, "pre-EBS stage alloc failed");
                        }
                    } else {
                        log_warn("EBS", 901, "helix.img missing/empty/too-large");
                    }
                    let _ = ((*img).close)(img);
                } else {
                    log_warn("EBS", 901, "/morpheus/helix.img not found on ESP");
                }
                let _ = ((*root).close)(root);
            }
        }
    }

    log_info("EBS", 903, "fetching memory map");

    // memory map + exitbootservices
    static mut MMAP_BUF: [u8; 65536] = [0u8; 65536];
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

    log_info("EBS", 904, "calling ExitBootServices");
    let status = (bs.exit_boot_services)(config.image_handle, map_key);
    if status != 0 {
        // EBS failed — either stale map key or wrong offset in MinBootServices.
        // This message will appear on serial before we loop.
        let _ = status;
        log_error("EBS", 905, "ExitBootServices failed");
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

    // Switch once, after EBS, onto our own loaderdata stack.
    core::arch::asm!("mov rsp, {0}", in(reg) stack_top, options(nostack));

    log_ok("BOOT", 906, "machine ownership transferred");

    // compute pe image bounds for buddy-allocator reservation
    let image_pages = pe_image_size(image_base).div_ceil(4096);

    // hwinit — gdt, idt, pic, heap, tsc, dma, bus mastering
    let hwinit_cfg = morpheus_hwinit::SelfContainedConfig {
        memory_map_ptr: MMAP_BUF.as_ptr(),
        memory_map_size: MMAP_SIZE,
        descriptor_size: DESC_SIZE,
        descriptor_version: DESC_VER,
        image_base,
        image_pages,
        stack_base,
        stack_pages: STACK_PAGES as u64,
        acpi_rsdp_phys,
    };

    let platform = match morpheus_hwinit::platform_init_selfcontained(hwinit_cfg) {
        Ok(p) => {
            // Register BSoD crash hook now that framebuffer info is available
            unsafe {
                morpheus_hwinit::set_crash_hook(bsod_crash_hook);
            }
            morpheus_hwinit::serial::log_info("BOOT", 210, "crash hook registered");

            p
        }
        Err(e) => {
            let _ = e;
            log_error("BOOT", 907, "hwinit failed");
            loop {
                unsafe {
                    core::arch::asm!("hlt", options(nomem, nostack));
                }
            }
        }
    };

    // userspace network activation hook. we stay offline by default.
    network::init_userspace_network_activation(morpheus_network::dma::DmaRegion::new(
        platform.dma().cpu_base(),
        platform.dma().bus_base(),
        platform.dma().size(),
    ), platform.tsc_freq());

    // persistent storage — try to mount a real block device
    crate::storage::init_persistent_storage(platform.dma(), platform.tsc_freq());

    // initfs — ensure standard directory structure exists
    crate::storage::create_init_directories();

    // framebuffer → screen
    let fb_info = FRAMEBUFFER_INFO;
    if fb_info.base == 0 || fb_info.width == 0 {
        log_error("DISPLAY", 908, "no framebuffer available");
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

    log_ok("DISPLAY", 909, "framebuffer registered");

    // Register framebuffer with the syscall layer so SYS_FB_INFO / SYS_FB_MAP work.
    morpheus_hwinit::register_framebuffer(morpheus_hwinit::FbInfo {
        base: fb_info.base,
        size: fb_info.size as u64,
        width: fb_info.width,
        height: fb_info.height,
        stride: stride_bytes,
        format: fb_info.format,
    });

    log_info("BOOT", 910, "launching desktop");
    crate::tui::desktop::run_desktop(&display_info);
}
