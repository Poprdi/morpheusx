//! Single authoritative boot sequence: UEFI entry → first userspace process.
//! Every stage is invoked from `run()` in order; no hidden boot work in helper modules.

#![allow(clippy::missing_safety_doc)]

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use morpheus_display::console::TextConsole;
use morpheus_hal_x86_64::platform::PlatformInit;
use morpheus_hal_x86_64::serial::{
    boot_banner, boot_step_fail, boot_step_ok, boot_step_warn, clear_live_console_hook, log_error,
    log_warn, puts, set_log_style,
};

use crate::tui::input::Keyboard;
use crate::tui::mouse::Mouse;
use crate::{baremetal_ops, bsod, storage, tui, uefi_allocator};

/// Raw framebuffer info from GOP. `stride` is `pixels_per_scan_line` (not bytes).
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FramebufferInfo {
    pub base: u64,
    pub size: usize,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    /// 0 = RGBX, 1 = BGRX, 2 = BitMask, 3 = BltOnly
    pub format: u32,
}

impl FramebufferInfo {
    const fn zeroed() -> Self {
        Self {
            base: 0,
            size: 0,
            width: 0,
            height: 0,
            stride: 0,
            format: 0,
        }
    }

    fn is_valid(&self) -> bool {
        self.base != 0 && self.width > 0 && self.height > 0
    }
}

/// In-memory copy of `/morpheus/helix.img` staged before `ExitBootServices`.
/// Storage init (D2) uses this in preference to block-device probing.
#[derive(Clone, Copy)]
pub struct PreEbsHelixImage {
    pub base: u64,
    pub size: usize,
    pub sector_size: u32,
}

/// Per-stage boot outputs accumulated across the UEFI→runtime transition.
pub struct BootContext {
    // ── Phase A: pre-EBS (UEFI alive) ────────────────────────────────
    image_handle: *mut (),
    system_table: *const (),
    framebuffer: FramebufferInfo,

    // ── Phase B: pre-EBS prep ────────────────────────────────────────
    /// Physical base of the post-EBS kernel stack we asked UEFI for.
    stack_base: u64,
    /// Stack size in 4 KiB pages.
    stack_pages: u64,
    /// Top of stack (base + size).
    stack_top: u64,
    /// Optional pre-EBS staged HelixFS image (consumed by storage init).
    pre_ebs_helix: Option<PreEbsHelixImage>,
    /// PE image base (from `__ImageBase` linker symbol).
    image_base: u64,
    /// PE image size in pages.
    image_pages: u64,
    /// ACPI RSDP physical address (0 = none found).
    acpi_rsdp_phys: u64,

    // ── Phase C: memory-map snapshot for EBS ─────────────────────────
    mmap_ptr: *const u8,
    mmap_size: usize,
    desc_size: usize,
    desc_ver: u32,

    // ── Phase D: post-platform-init outputs ──────────────────────────
    platform_dma_cpu: u64,
    platform_dma_bus: u64,
    platform_dma_size: usize,
    tsc_freq: u64,
}

impl BootContext {
    const fn new() -> Self {
        Self {
            image_handle: core::ptr::null_mut(),
            system_table: core::ptr::null(),
            framebuffer: FramebufferInfo::zeroed(),
            stack_base: 0,
            stack_pages: 0,
            stack_top: 0,
            pre_ebs_helix: None,
            image_base: 0,
            image_pages: 0,
            acpi_rsdp_phys: 0,
            mmap_ptr: core::ptr::null(),
            mmap_size: 0,
            desc_size: 0,
            desc_ver: 0,
            platform_dma_cpu: 0,
            platform_dma_bus: 0,
            platform_dma_size: 0,
            tsc_freq: 0,
        }
    }
}

// Framebuffer snapshot for crash context (no heap, no locks). Written once in
// stage B; read lock-free by BSoD hook and panic handler. Only mutable globals.

static FB_PUBLISHED: AtomicBool = AtomicBool::new(false);
static FB_BASE: AtomicU64 = AtomicU64::new(0);
static FB_SIZE: AtomicUsize = AtomicUsize::new(0);
static FB_WIDTH: AtomicU32 = AtomicU32::new(0);
static FB_HEIGHT: AtomicU32 = AtomicU32::new(0);
static FB_STRIDE: AtomicU32 = AtomicU32::new(0);
static FB_FORMAT: AtomicU32 = AtomicU32::new(0);

fn publish_framebuffer(fb: &FramebufferInfo) {
    FB_BASE.store(fb.base, Ordering::Relaxed);
    FB_SIZE.store(fb.size, Ordering::Relaxed);
    FB_WIDTH.store(fb.width, Ordering::Relaxed);
    FB_HEIGHT.store(fb.height, Ordering::Relaxed);
    FB_STRIDE.store(fb.stride, Ordering::Relaxed);
    FB_FORMAT.store(fb.format, Ordering::Relaxed);
    FB_PUBLISHED.store(true, Ordering::Release);
}

/// Lock-free framebuffer accessor; returns `None` before stage B.
pub fn published_framebuffer() -> Option<FramebufferInfo> {
    if !FB_PUBLISHED.load(Ordering::Acquire) {
        return None;
    }
    Some(FramebufferInfo {
        base: FB_BASE.load(Ordering::Relaxed),
        size: FB_SIZE.load(Ordering::Relaxed),
        width: FB_WIDTH.load(Ordering::Relaxed),
        height: FB_HEIGHT.load(Ordering::Relaxed),
        stride: FB_STRIDE.load(Ordering::Relaxed),
        format: FB_FORMAT.load(Ordering::Relaxed),
    })
}

// Serial puts() mirror hook; held as raw fn ptr by the serial layer. Dropped before userspace.
static mut LIVE_CONSOLE: Option<TextConsole> = None;

/// Stage B puts() hook. Must not touch heap or locks.
unsafe fn live_console_putc(b: u8) {
    if let Some(ref mut con) = LIVE_CONSOLE {
        con.write_char(b as char);
    }
}

const LOADED_IMAGE_PROTOCOL_GUID: [u8; 16] = [
    0xA1, 0x31, 0x1B, 0x5B, 0x62, 0x95, 0xD2, 0x11, 0x8E, 0x3F, 0x00, 0xA0, 0xC9, 0x69, 0x72, 0x3B,
];
const SIMPLE_FILE_SYSTEM_PROTOCOL_GUID: [u8; 16] = [
    0x22, 0x5B, 0x4E, 0x96, 0x59, 0x64, 0xD2, 0x11, 0x8E, 0x39, 0x00, 0xA0, 0xC9, 0x69, 0x72, 0x3B,
];
const ACPI_20_TABLE_GUID: [u8; 16] = [
    0x71, 0xE8, 0x68, 0x88, 0xF1, 0xE4, 0xD3, 0x11, 0xBC, 0x22, 0x00, 0x80, 0xC7, 0x3C, 0x88, 0x81,
];
const ACPI_10_TABLE_GUID: [u8; 16] = [
    0x30, 0x2D, 0x9D, 0xEB, 0x88, 0x2D, 0xD3, 0x11, 0x9A, 0x16, 0x00, 0x90, 0x27, 0x3F, 0xC1, 0x4D,
];
const GOP_GUID: [u8; 16] = [
    0xDE, 0xA9, 0x42, 0x90, 0xDC, 0x23, 0x38, 0x4A, 0x96, 0xFB, 0x7A, 0xDE, 0xD0, 0x80, 0x51, 0x6A,
];

const EFI_FILE_MODE_READ: u64 = 0x0000_0000_0000_0001;
const PRE_EBS_STAGE_MAX_BYTES: u64 = 512 * 1024 * 1024;
const HELIX_IMG_SECTOR_SIZE: u32 = 512;
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

const KERNEL_STACK_SIZE: usize = 256 * 1024;
const KERNEL_STACK_PAGES: usize = KERNEL_STACK_SIZE.div_ceil(4096);

#[repr(C)]
struct EfiSystemTable {
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
    boot_services: *const EfiBootServices,
    number_of_table_entries: usize,
    configuration_table: *const EfiConfigurationTable,
}

#[repr(C)]
struct EfiBootServices {
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
    handle_protocol: extern "efiapi" fn(*mut (), *const [u8; 16], *mut *mut ()) -> usize,
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
    _get_next_monotonic_count: usize,
    _stall: usize,
    _set_watchdog_timer: usize,
    _connect_controller: usize,
    _disconnect_controller: usize,
    _open_protocol: usize,
    _close_protocol: usize,
    _open_protocol_information: usize,
    _protocols_per_handle: usize,
    _locate_handle_buffer: usize,
    locate_protocol: extern "efiapi" fn(*const [u8; 16], *const (), *mut *mut ()) -> usize,
}

#[repr(C)]
struct EfiConfigurationTable {
    vendor_guid: [u8; 16],
    vendor_table: *const (),
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
    open_volume: extern "efiapi" fn(*mut EfiSimpleFileSystem, *mut *mut EfiFileProtocol) -> usize,
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

#[repr(C)]
struct GopModeInfo {
    _version: u32,
    horizontal_resolution: u32,
    vertical_resolution: u32,
    pixel_format: u32,
    _pixel_bitmask: [u32; 4],
    pixels_per_scan_line: u32,
}

#[repr(C)]
struct GopMode {
    _max_mode: u32,
    _mode: u32,
    info: *const GopModeInfo,
    _size_of_info: usize,
    frame_buffer_base: u64,
    frame_buffer_size: usize,
}

#[repr(C)]
struct Gop {
    _query_mode: usize,
    _set_mode: usize,
    _blt: usize,
    mode: *mut GopMode,
}

/// EFI entry point; wrapper for `efiapi` calling convention.
#[no_mangle]
pub extern "efiapi" fn efi_main(image_handle: *mut (), system_table: *const ()) -> usize {
    unsafe { run(image_handle, system_table) }
}

/// Canonical boot sequence — every stage in order.
///
/// # Safety
/// Called once by UEFI with valid handle/table pointers.
unsafe fn run(image_handle: *mut (), system_table: *const ()) -> ! {
    let mut ctx = BootContext::new();
    ctx.image_handle = image_handle;
    ctx.system_table = system_table;

    // Lock COM1 to 115200 8N1 before any serial output — firmware leaves the
    // UART baud/line-control undefined, which breaks a real RS-232 link.
    morpheus_hal_x86_64::serial::serial_init();

    // Color on, numeric event codes off — the framebuffer console parses ANSI
    // SGR, so warn/error lines stand out and the happy-path checklist stays clean.
    set_log_style(true, false);

    // Phase A — pre-EBS: UEFI still alive, allocator points at UEFI pool.
    stage_a1_arm_uefi_allocator(&ctx);
    stage_a2_query_gop(&mut ctx);
    stage_a3_collect_acpi_rsdp(&mut ctx);
    stage_a4_record_pe_image_bounds(&mut ctx);
    stage_a5_allocate_kernel_stack(&mut ctx);
    stage_a6_stage_helix_image(&mut ctx);
    stage_a7_fetch_memory_map(&mut ctx);

    // Phase B — the border: ExitBootServices and machine takeover.
    stage_b1_exit_boot_services(&ctx);
    stage_b2_disable_interrupts();
    stage_b3_switch_allocator();
    stage_b4_switch_stack(&ctx);
    stage_b5_start_live_console(&ctx);
    stage_b6_publish_framebuffer(&ctx);

    // Console (if any) is live and cleared — paint the banner and the first
    // checklist row summarizing all the pre-EBS firmware work.
    boot_banner("MorpheusX", concat!("v", env!("CARGO_PKG_VERSION")));
    boot_step_ok("firmware handoff (UEFI/GOP)");

    // Phase C — hardware: hwinit takes ownership of the machine.
    let platform = stage_c1_platform_init(&ctx);
    stage_c1b_kernel_late_init(&platform);
    stage_c2_register_crash_hook();
    stage_c3_record_platform_outputs(&mut ctx, &platform);

    // Phase D — runtime services on top of hwinit.
    stage_d1_register_network_activation(&ctx, &platform);
    stage_d2_storage_init(&ctx, &platform);
    stage_d3_initfs_bootstrap();
    stage_d4_register_framebuffer(&ctx);

    // Phase E — release concurrency and enter userspace.
    stage_e1_release_aps();
    stage_e2_enter_userspace(&ctx);
}

/// A1. Arm UEFI allocator; pre-EBS allocs route through `allocate_pool`.
unsafe fn stage_a1_arm_uefi_allocator(ctx: &BootContext) {
    let st = &*(ctx.system_table as *const EfiSystemTable);
    // `set_boot_services` accepts the bootloader's own `BootServices` type;
    // both this module and the allocator type-pun the same UEFI layout, so
    // a raw cast is sound here.
    uefi_allocator::set_boot_services(st.boot_services as *const _ as *const crate::BootServices);
}

/// A2. Locate GOP and capture the framebuffer descriptor. Zero base → headless.
unsafe fn stage_a2_query_gop(ctx: &mut BootContext) {
    let st = &*(ctx.system_table as *const EfiSystemTable);
    let bs = &*st.boot_services;

    let mut gop_ptr: *mut Gop = core::ptr::null_mut();
    let status = (bs.locate_protocol)(
        &GOP_GUID,
        core::ptr::null(),
        &mut gop_ptr as *mut _ as *mut *mut (),
    );

    if status == 0 && !gop_ptr.is_null() {
        let mode = &*(*gop_ptr).mode;
        let info = &*mode.info;
        ctx.framebuffer = FramebufferInfo {
            base: mode.frame_buffer_base,
            size: mode.frame_buffer_size,
            width: info.horizontal_resolution,
            height: info.vertical_resolution,
            stride: info.pixels_per_scan_line,
            format: info.pixel_format,
        };
    } else {
        log_warn("UEFI", 902, "GOP not present; boot will run headless");
    }
}

/// A3. Walk UEFI config table for ACPI RSDP (2.0 first); sets `ctx.acpi_rsdp_phys`.
unsafe fn stage_a3_collect_acpi_rsdp(ctx: &mut BootContext) {
    let st = &*(ctx.system_table as *const EfiSystemTable);
    if st.configuration_table.is_null() || st.number_of_table_entries == 0 {
        log_warn("ACPI", 901, "no UEFI configuration table");
        return;
    }

    let tables = core::slice::from_raw_parts(st.configuration_table, st.number_of_table_entries);

    // ACPI 2.0 first (XSDT); fall back to ACPI 1.0 (RSDT).
    for t in tables {
        if t.vendor_guid == ACPI_20_TABLE_GUID {
            ctx.acpi_rsdp_phys = t.vendor_table as u64;
            return;
        }
    }
    for t in tables {
        if t.vendor_guid == ACPI_10_TABLE_GUID {
            ctx.acpi_rsdp_phys = t.vendor_table as u64;
            return;
        }
    }
    log_warn("ACPI", 904, "no RSDP in UEFI configuration table");
}

/// A4. Capture PE image bounds so the buddy never reclaims kernel .text/.data/.bss.
unsafe fn stage_a4_record_pe_image_bounds(ctx: &mut BootContext) {
    extern "C" {
        static __ImageBase: u8;
    }
    let image_base = &__ImageBase as *const u8 as u64;
    ctx.image_base = image_base;

    // PE32+: SizeOfImage at offset 56 of the optional header.
    let pe_off = core::ptr::read_unaligned((image_base + 0x3C) as *const u32) as u64;
    let size_of_image =
        core::ptr::read_unaligned((image_base + pe_off + 4 + 20 + 56) as *const u32) as u64;
    ctx.image_pages = size_of_image.div_ceil(4096);
}

/// A5. Allocate kernel stack (EfiLoaderData — survives EBS).
unsafe fn stage_a5_allocate_kernel_stack(ctx: &mut BootContext) {
    let st = &*(ctx.system_table as *const EfiSystemTable);
    let bs = &*st.boot_services;

    let mut base: u64 = 0;
    // AllocateType 0 = AllocateAnyPages, MemoryType 2 = EfiLoaderData.
    let status = (bs.allocate_pages)(0, 2, KERNEL_STACK_PAGES, &mut base);
    if status != 0 || base == 0 {
        boot_panic("EBS", "kernel stack allocation failed");
    }

    ctx.stack_base = base;
    ctx.stack_pages = KERNEL_STACK_PAGES as u64;
    ctx.stack_top = base + KERNEL_STACK_SIZE as u64;
}

/// A6. Best-effort load of `/morpheus/helix.img` from the ESP into EfiLoaderData RAM.
unsafe fn stage_a6_stage_helix_image(ctx: &mut BootContext) {
    let st = &*(ctx.system_table as *const EfiSystemTable);
    let bs = &*st.boot_services;

    // Resolve the device the bootloader was loaded from.
    let mut loaded_image_ptr: *mut () = core::ptr::null_mut();
    let li_status = (bs.handle_protocol)(
        ctx.image_handle,
        &LOADED_IMAGE_PROTOCOL_GUID,
        &mut loaded_image_ptr,
    );
    if li_status != 0 || loaded_image_ptr.is_null() {
        log_warn(
            "EBS",
            906,
            "loaded-image protocol unavailable; skipping pre-EBS stage",
        );
        return;
    }
    let li = &*(loaded_image_ptr as *const EfiLoadedImage);

    // Open the simple filesystem on that device.
    let mut sfs_ptr: *mut () = core::ptr::null_mut();
    let sfs_status = (bs.handle_protocol)(
        li.device_handle,
        &SIMPLE_FILE_SYSTEM_PROTOCOL_GUID,
        &mut sfs_ptr,
    );
    if sfs_status != 0 || sfs_ptr.is_null() {
        log_warn(
            "EBS",
            907,
            "no simple-fs on boot device; skipping pre-EBS stage",
        );
        return;
    }
    let sfs = sfs_ptr as *mut EfiSimpleFileSystem;

    let mut root: *mut EfiFileProtocol = core::ptr::null_mut();
    if ((*sfs).open_volume)(sfs, &mut root) != 0 || root.is_null() {
        log_warn("EBS", 908, "open_volume failed");
        return;
    }

    let mut img: *mut EfiFileProtocol = core::ptr::null_mut();
    let open_rc = ((*root).open)(
        root,
        &mut img,
        HELIX_IMG_PATH.as_ptr(),
        EFI_FILE_MODE_READ,
        0,
    );
    if open_rc != 0 || img.is_null() {
        log_warn("EBS", 909, "/morpheus/helix.img absent");
        let _ = ((*root).close)(root);
        return;
    }

    // Query file size by seeking to EOF.
    let _ = ((*img).set_position)(img, u64::MAX);
    let mut file_size: u64 = 0;
    let _ = ((*img).get_position)(img, &mut file_size);
    let _ = ((*img).set_position)(img, 0);

    if file_size == 0 || file_size > PRE_EBS_STAGE_MAX_BYTES {
        log_warn("EBS", 910, "helix.img missing/empty/too-large");
        let _ = ((*img).close)(img);
        let _ = ((*root).close)(root);
        return;
    }

    // Round up to whole sectors for downstream block-device semantics.
    let mut alloc_bytes = file_size as usize;
    let rem = alloc_bytes % HELIX_IMG_SECTOR_SIZE as usize;
    if rem != 0 {
        alloc_bytes += (HELIX_IMG_SECTOR_SIZE as usize) - rem;
    }

    let mut stage_base: u64 = 0;
    let stage_pages = alloc_bytes.div_ceil(4096);
    if (bs.allocate_pages)(0, 2, stage_pages, &mut stage_base) != 0 || stage_base == 0 {
        log_warn("EBS", 911, "pre-EBS stage allocation failed");
        let _ = ((*img).close)(img);
        let _ = ((*root).close)(root);
        return;
    }

    // Stream-read in 1 MiB chunks; some firmware caps single-call sizes.
    let mut off = 0usize;
    let mut ok = true;
    while off < file_size as usize {
        let remaining = (file_size as usize) - off;
        let mut want = core::cmp::min(1024 * 1024, remaining);
        let rc = ((*img).read)(img, &mut want, (stage_base as *mut u8).add(off));
        if rc != 0 {
            ok = false;
            break;
        }
        if want == 0 {
            // EOF earlier than reported size.
            break;
        }
        off += want;
    }
    let _ = ((*img).close)(img);
    let _ = ((*root).close)(root);

    if !ok || off == 0 {
        log_warn("EBS", 912, "failed reading /morpheus/helix.img");
        return;
    }

    let usable = off - (off % HELIX_IMG_SECTOR_SIZE as usize);
    if usable == 0 {
        log_warn("EBS", 913, "helix.img not sector-aligned");
        return;
    }

    ctx.pre_ebs_helix = Some(PreEbsHelixImage {
        base: stage_base,
        size: usable,
        sector_size: HELIX_IMG_SECTOR_SIZE,
    });
}

/// A7. Snapshot UEFI memory map. No further allocate_pages after this.
unsafe fn stage_a7_fetch_memory_map(ctx: &mut BootContext) {
    // Buffer lives in .bss (static), valid across EBS since it's our memory.
    static mut MMAP_BUF: [u8; 65536] = [0u8; 65536];

    let st = &*(ctx.system_table as *const EfiSystemTable);
    let bs = &*st.boot_services;

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
        boot_panic("EBS", "get_memory_map failed");
    }

    ctx.mmap_ptr = MMAP_BUF.as_ptr();
    ctx.mmap_size = map_size;
    ctx.desc_size = desc_size;
    ctx.desc_ver = desc_ver;

    // Static so stage_b1 (takes only &ctx) can read the key without it living in BootContext.
    EBS_MAP_KEY.store(map_key, Ordering::Relaxed);
}

static EBS_MAP_KEY: AtomicUsize = AtomicUsize::new(0);

/// B1. ExitBootServices. Failure unrecoverable — IDT still in BootServicesCode.
unsafe fn stage_b1_exit_boot_services(ctx: &BootContext) {
    let st = &*(ctx.system_table as *const EfiSystemTable);
    let bs = &*st.boot_services;
    let map_key = EBS_MAP_KEY.load(Ordering::Relaxed);

    let status = (bs.exit_boot_services)(ctx.image_handle, map_key);
    if status != 0 {
        // OVMF DEBUG scrubs freed BootServicesCode with 0xAF, so a stale
        // map key or a layout-mismatched MinBootServices struct here is a
        // guaranteed crash. We can't even reliably log past this point.
        log_error("EBS", 921, "ExitBootServices failed");
        halt_forever();
    }
}

/// B2. `cli` — UEFI's timer may still be armed; one tick into the OVMF 0xAF
/// scrub poisons buddy FreeNode pointers.
unsafe fn stage_b2_disable_interrupts() {
    core::arch::asm!("cli", options(nomem, nostack));
}

/// B3. Switch allocator from UEFI pool to static heap.
unsafe fn stage_b3_switch_allocator() {
    uefi_allocator::switch_to_post_ebs();
}

/// B4. Pivot RSP to the kernel stack (A5). Old UEFI stack is gone after this.
unsafe fn stage_b4_switch_stack(ctx: &BootContext) {
    let stack_top = ctx.stack_top;
    core::arch::asm!("mov rsp, {0}", in(reg) stack_top, options(nostack));
}

/// B5. Mirror serial puts() to the framebuffer console until E2 clears the hook.
unsafe fn stage_b5_start_live_console(ctx: &BootContext) {
    if !ctx.framebuffer.is_valid() {
        return;
    }
    let stride_bytes = ctx.framebuffer.stride * 4;
    let display_fb = morpheus_display::types::FramebufferInfo {
        base: ctx.framebuffer.base,
        size: ctx.framebuffer.size,
        width: ctx.framebuffer.width,
        height: ctx.framebuffer.height,
        stride: stride_bytes,
        format: match ctx.framebuffer.format {
            0 => morpheus_display::types::PixelFormat::Rgbx,
            _ => morpheus_display::types::PixelFormat::Bgrx,
        },
    };
    let raw_fb = morpheus_display::framebuffer::Framebuffer::new(display_fb);
    let mut con = TextConsole::new(raw_fb);
    con.clear();
    LIVE_CONSOLE = Some(con);
    morpheus_hal_x86_64::serial::set_live_console_hook(live_console_putc);
}

/// B6. Publish framebuffer for crash-context consumers.
fn stage_b6_publish_framebuffer(ctx: &BootContext) {
    publish_framebuffer(&ctx.framebuffer);
}

/// C1. Hand memory map/stack/image/RSDP to hal-x86_64 (phases 1–9).
/// Late-init (scheduler/syscalls/FS) and SMP in C1b.
unsafe fn stage_c1_platform_init(ctx: &BootContext) -> PlatformInit {
    // Wire the 4 fn-pointer hooks hal-x86_64 phase 9 needs (kernel-side TSC
    // publish + HID ringbuf init + xHCI MSI-X wiring + xHCI runtime install).
    // Wave C1c moved the USB symbols to morpheus-xhci; we install them here
    // BEFORE invoking platform init so phase 9's xHCI block can see them.
    morpheus_hal_x86_64::platform::set_tsc_freq_publish_hook(
        morpheus_kernel::schedular::set_tsc_frequency,
    );
    morpheus_hal_x86_64::platform::set_input_init_hook(kernel_input_init_shim);
    morpheus_hal_x86_64::platform::set_xhci_msix_hook(xhci_msix_shim);
    morpheus_hal_x86_64::platform::set_xhci_runtime_hook(
        morpheus_xhci::usb::runtime::install_runtime,
    );

    // Port-IO seam: SYS_PORT_IN/OUT call these x86 primitives (kept out of the
    // HAL trait per LD25 so non-x86 stays ENOSYS).
    morpheus_kernel::syscall::handler::hw::set_port_io_hooks(
        morpheus_hal_x86_64::io::port_in,
        morpheus_hal_x86_64::io::port_out,
    );

    // HID event sinks: the xHCI HID drivers deliver events here so the kernel
    // input layer receives them. Must be installed before HID init runs.
    //
    // SAFETY: pre-scheduler, single-threaded; no HID parsing in flight.
    unsafe {
        morpheus_xhci::usb::hid::sink::set_keyboard_sink(morpheus_kernel::input::hid_keyboard_sink);
        morpheus_xhci::usb::hid::sink::set_mouse_sink(morpheus_kernel::input::hid_mouse_sink);
    }

    // Install the HAL on the kernel BEFORE platform_init_selfcontained runs.
    // Phase 9 inside platform init invokes the input_init_hook, which calls
    // morpheus_kernel::input::init(); that touches kernel SpinLocks, which
    // route IF management through hal() — so hal() must already be live.
    //
    // SAFETY: BSP, single-threaded; `HalImpl::new()` has no hardware
    // preconditions (it's a zero-sized struct), so it's safe pre-phase-1.
    morpheus_kernel::install_hal(unsafe { morpheus_hal_x86_64::platform::init() });

    // Keep the pre-EBS HelixFS image out of the buddy: it's UEFI LoaderData
    // (which import would otherwise reclaim and scribble FreeNodes into) but the
    // RAM block device reads/writes it for the system's lifetime.
    let (helix_base, helix_pages) = match ctx.pre_ebs_helix.as_ref() {
        Some(img) => (img.base, (img.size as u64).div_ceil(4096)),
        None => (0, 0),
    };

    let cfg = morpheus_hal_x86_64::platform::SelfContainedConfig {
        memory_map_ptr: ctx.mmap_ptr,
        memory_map_size: ctx.mmap_size,
        descriptor_size: ctx.desc_size,
        descriptor_version: ctx.desc_ver,
        image_base: ctx.image_base,
        image_pages: ctx.image_pages,
        stack_base: ctx.stack_base,
        stack_pages: ctx.stack_pages,
        helix_base,
        helix_pages,
        acpi_rsdp_phys: ctx.acpi_rsdp_phys,
    };
    match morpheus_hal_x86_64::platform::platform_init_selfcontained(cfg) {
        Ok(p) => p,
        Err(_) => boot_panic("BOOT", "platform init failed"),
    }
}

/// Convert `PciAddr` → `BusAddr` at the hal/xhci boundary before wiring MSI-X.
unsafe fn xhci_msix_shim(pci_addr: morpheus_hal_x86_64::pci::PciAddr, rt_base: u64) {
    let bus_addr = morpheus_hal_api::BusAddr::new(pci_addr.bus, pci_addr.device, pci_addr.function);
    // SAFETY: IDT + LAPIC + BAR all live by hal-x86_64 phase 9 entry.
    unsafe { morpheus_xhci::usb::msi::wire_msix(bus_addr, rt_base) };
}

/// Coerce safe `fn input::init()` to `unsafe fn()` — Rust won't auto-coerce through a generic register call.
unsafe fn kernel_input_init_shim() {
    morpheus_kernel::input::init();
}

/// C1b. Disable legacy 8259, late-init (scheduler/syscalls/FS), reclaim BootServices RAM, SMP bring-up.
/// Split from C1: scheduler must precede APs (LD16); reclaim must be post-LAPIC-takeover.
unsafe fn stage_c1b_kernel_late_init(platform: &PlatformInit) {
    let hal = morpheus_kernel::hal();

    // SAFETY: BSP, interrupts already off (B2 disabled them; late_init below
    // re-enables once the LAPIC has taken over). PIC is quiesced — no UEFI
    // driver thread is alive past ExitBootServices.
    unsafe { hal.intr().disable_legacy_pic() };

    const ROOT_FS_SIZE: usize = 16 * 1024 * 1024;

    morpheus_kernel::late_init(
        hal,
        morpheus_kernel::InitParams {
            timer_isr: hal.smp().timer_isr(),
            root_fs_size: ROOT_FS_SIZE,
            kernel_stack_top: platform.kernel_stack_top,
        },
    );

    // Wire KernelCr3Guard's kernel-CR3 lookup. The kernel can't call the arch
    // HAL directly (portability gate), so the bootloader installs the hook now
    // that init_scheduler has set the kernel CR3. Without this the guard is a
    // permanent no-op and phys walks under a user CR3 rely solely on the cloned
    // kernel half.
    //
    // SAFETY: BSP, single-threaded, post-late_init (kernel CR3 is set).
    unsafe {
        morpheus_hal_x86_64::memory::set_kernel_cr3_hook(morpheus_kernel::init::kernel_cr3_hook);
    }

    // Phase 10.5: reclaim BootServices{Code,Data}. Must run AFTER late-init
    // (timer IRQ live, no UEFI-stage reference outstanding) AND BEFORE the
    // helixfs 16 MiB alloc — pre-refactor order. Reclaim's
    // `validate_free_lists` walk has been observed to PF on real hardware
    // when the buddy was already hammered by a prior large alloc.
    //
    // SAFETY: BSP, post-late_init per the trait contract. Byte count is
    // discarded; the impl logs the figure internally.
    let _ = unsafe { hal.phys().reclaim_boot_services() };

    // Phase 11b: HelixFS root mount. Runs AFTER reclaim so the buddy is in
    // a known-clean post-reclaim state when the 16 MiB alloc carves it.
    morpheus_kernel::init::mount_root_fs(hal, ROOT_FS_SIZE);

    // Phase 12: SMP bring-up. Per LD16 this happens AFTER the kernel
    // scheduler is initialized. Discovery returns an empty slice when MADT
    // is absent / invalid; in that case fall back to CPUID brute-force
    // enumeration (`start_aps`) rather than forcing single-core — real
    // hardware with a quirky/missing MADT booted SMP pre-refactor.
    //
    // SAFETY: BSP, scheduler live, ACPI tables still identity-mapped per
    // the HAL trait contract.
    let lapic_ids = match unsafe { hal.smp().discover_ap_lapic_ids(platform.acpi_rsdp_phys) } {
        Ok(ids) => ids,
        Err(_) => {
            log_warn("SMP", 213, "MADT discovery failed; trying CPUID scan");
            &[]
        },
    };
    let mut smp_buf = [0u8; 32];
    if lapic_ids.is_empty() {
        // No MADT APs: CPUID brute-force fallback (detects topology internally).
        let cores = hal.smp().start_aps();
        boot_step_ok(fmt_smp_label(&mut smp_buf, cores.max(1)));
    } else {
        // SAFETY: BSP, post-discover, IF managed by the impl.
        match unsafe { hal.smp().start_aps_from_list(lapic_ids) } {
            // ap_count excludes the BSP.
            Ok(ap_count) => boot_step_ok(fmt_smp_label(&mut smp_buf, ap_count + 1)),
            Err(_) => {
                boot_step_warn("smp");
                log_warn("SMP", 214, "AP startup failed; continuing single-core");
            },
        }
    }
}

/// Format `smp (N cpu online)` into `buf`. No alloc.
fn fmt_smp_label(buf: &mut [u8], cpus: u32) -> &str {
    const PREFIX: &[u8] = b"smp (";
    const SUFFIX: &[u8] = b" cpu online)";
    let mut i = 0;
    for &b in PREFIX {
        buf[i] = b;
        i += 1;
    }
    if cpus == 0 {
        buf[i] = b'0';
        i += 1;
    } else {
        let mut digits = [0u8; 10];
        let mut n = cpus;
        let mut d = 0;
        while n > 0 {
            digits[d] = b'0' + (n % 10) as u8;
            n /= 10;
            d += 1;
        }
        while d > 0 {
            d -= 1;
            buf[i] = digits[d];
            i += 1;
        }
    }
    for &b in SUFFIX {
        buf[i] = b;
        i += 1;
    }
    core::str::from_utf8(&buf[..i]).unwrap_or("smp")
}

/// C2. Wire BSoD crash hook (framebuffer was published in B6; safe from exception context).
unsafe fn stage_c2_register_crash_hook() {
    use morpheus_hal_x86_64::cpu::idt;
    idt::set_crash_hook(bsod::show_crash_screen);
    // Without these, every USER fault fell through to the BSoD + SYSTEM HALTED
    // (the exit hook was never installed) and crash dumps misattributed to PID 0
    // / [kernel]. Wire them so a faulting user thread is terminated and the box
    // keeps scheduling, with the real pid/name in the dump.
    idt::set_process_exit_hook(morpheus_kernel::schedular::exit_process);
    idt::set_current_pid_hook(morpheus_kernel::schedular::state::idt_current_pid);
    idt::set_process_lookup_hook(morpheus_kernel::schedular::state::idt_lookup_name);
}

/// C3. Copy DMA region + TSC freq out of `PlatformInit` into `BootContext`.
fn stage_c3_record_platform_outputs(ctx: &mut BootContext, platform: &PlatformInit) {
    let dma = platform.dma();
    ctx.platform_dma_cpu = dma.cpu_base() as u64;
    ctx.platform_dma_bus = dma.bus_base();
    ctx.platform_dma_size = dma.size();
    ctx.tsc_freq = platform.tsc_freq();
}

/// D1. Register network activation callback (userspace opt-in via SYS_NET_CFG).
unsafe fn stage_d1_register_network_activation(_ctx: &BootContext, platform: &PlatformInit) {
    baremetal_ops::network::init_userspace_network_activation(
        morpheus_virtio::dma::DmaRegion::new(
            platform.dma().cpu_base(),
            platform.dma().bus_base(),
            platform.dma().size(),
        ),
        platform.tsc_freq(),
    );
}

/// D2. Root FS bring-up: pre-EBS image → persistent block device → RAM HelixFS.
unsafe fn stage_d2_storage_init(ctx: &BootContext, platform: &PlatformInit) {
    storage::init_persistent_storage(platform.dma(), ctx.tsc_freq, ctx.pre_ebs_helix);
}

/// D3. Create standard root directories. Idempotent.
fn stage_d3_initfs_bootstrap() {
    storage::create_init_directories();
}

/// D4. Register framebuffer with the syscall layer (SYS_FB_INFO / SYS_FB_MAP).
unsafe fn stage_d4_register_framebuffer(ctx: &BootContext) {
    if !ctx.framebuffer.is_valid() {
        log_warn("DISPLAY", 940, "no framebuffer to register");
        return;
    }
    let stride_bytes = ctx.framebuffer.stride * 4;
    morpheus_kernel::syscall::handler::register_framebuffer(
        morpheus_kernel::syscall::handler::FbInfo {
            base: ctx.framebuffer.base,
            size: ctx.framebuffer.size as u64,
            width: ctx.framebuffer.width,
            height: ctx.framebuffer.height,
            stride: stride_bytes,
            format: ctx.framebuffer.format,
        },
    );
}

/// E1. Release parked APs. All preceding stages must be SMP-safe before this.
unsafe fn stage_e1_release_aps() {
    morpheus_hal_x86_64::cpu::ap_boot::release_parked_aps();
}

/// E2. Load and spawn `/bin/init`; become the kernel input-forwarding loop.
unsafe fn stage_e2_enter_userspace(_ctx: &BootContext) -> ! {
    // Pick input init based on what Phase 9 actually detected. If a USB
    // keyboard was enumerated we don't need (and don't want) to probe PS/2 —
    // on boards without a PS/2 controller the full reset path floods the log
    // with warnings and accomplishes nothing.
    let usb_kbd_present = morpheus_xhci::usb::runtime::keyboard_present();
    let mut keyboard = if usb_kbd_present {
        Keyboard::new_decoder_only()
    } else {
        Keyboard::new()
    };
    boot_log_gate(&mut keyboard);
    clear_live_console_hook();

    // Mouse currently has no USB driver; if USB keyboard is present we still
    // skip PS/2 mouse init to keep the log clean. Mouse becomes a no-op
    // until a USB mouse driver is wired up.
    let mut mouse = if usb_kbd_present {
        Mouse::new_decoder_only()
    } else {
        Mouse::new()
    };

    let elf_data = match load_init_elf() {
        Some(d) => d,
        None => boot_panic("BOOT", "/bin/init not found"),
    };

    let init_pid =
        match morpheus_kernel::schedular::spawn_user_process("init", &elf_data, &[], 0, false) {
            Ok(pid) => pid,
            Err(_) => boot_panic("BOOT", "failed to spawn /bin/init"),
        };
    let _ = init_pid;

    // The ELF buffer can drop now; init has its own address space.
    drop(elf_data);

    input_forwarding_loop(&mut keyboard, &mut mouse);
}

/// Spin until keypress. USB HID primary; PS/2 fallback when no USB keyboard detected.
fn boot_log_gate(keyboard: &mut Keyboard) {
    puts("\nPress any key to start userspace...");
    let usb_active = morpheus_xhci::usb::runtime::keyboard_present()
        || morpheus_xhci::usb::runtime::mouse_present();
    loop {
        if usb_active {
            // USB-primary. Non-blocking peek + drain the unified queue.
            unsafe {
                morpheus_xhci::usb::runtime::poll_input();
            }
            let mut got_press = false;
            while let Some(event) = morpheus_kernel::input::poll_keyboard() {
                if let morpheus_kernel::input::InputEvent::Key(_scan, pressed) = event {
                    if pressed {
                        got_press = true;
                    }
                }
            }
            if got_press {
                break;
            }
        } else {
            // PS/2 fallback
            if let Some(_key) = keyboard.read_key() {
                break;
            }
        }
        core::hint::spin_loop();
    }
    puts("\n");
}

/// Read `/bin/init` bytes from the root filesystem.
fn load_init_elf() -> Option<alloc::vec::Vec<u8>> {
    use alloc::string::String;

    let path = String::from("/bin/init");
    let ts = morpheus_hal_x86_64::cpu::tsc::read_tsc();

    // Size the load buffer (stat under the storage lock).
    let size = {
        let guard = unsafe { morpheus_kernel::storage::lock() };
        let g = &mut *guard.g;
        let (_, m, dev, rel) = match g.resolve_mut(&path) {
            Some(t) => t,
            None => {
                log_error("BOOT", 970, "no root filesystem");
                return None;
            },
        };
        match m.fs.stat(dev, rel) {
            Ok(s) => s.size,
            Err(_) => {
                log_error("BOOT", 971, "stat /bin/init failed");
                return None;
            },
        }
    };
    if size == 0 {
        log_error("BOOT", 972, "/bin/init has zero size");
        return None;
    }

    // Read via the backend with a transient FdState (no process fd slot).
    let mut buf = alloc::vec![0u8; size as usize];
    let n = {
        let guard = unsafe { morpheus_kernel::storage::lock() };
        let g = &mut *guard.g;
        let (mount_id, m, dev, rel) = match g.resolve_mut(&path) {
            Some(t) => t,
            None => {
                log_error("BOOT", 970, "no root filesystem");
                return None;
            },
        };
        let opened = match m
            .fs
            .open(dev, rel, morpheus_foundation::flags::open_flags::O_READ, ts)
        {
            Ok(o) => o,
            Err(_) => {
                log_error("BOOT", 973, "open /bin/init failed");
                return None;
            },
        };
        let mut fdstate = morpheus_kernel::storage::fs_api::FdState::empty();
        fdstate.mount_id = mount_id;
        fdstate.cookie = opened.cookie;
        // Backend reads key off the (mount-relative) path, not just the cookie.
        let pb = rel.as_bytes();
        let pn = pb.len().min(fdstate.path.len());
        fdstate.path[..pn].copy_from_slice(&pb[..pn]);
        fdstate.path_len = pn as u16;
        let n = match m.fs.read(dev, &fdstate, &mut buf) {
            Ok(bytes) => bytes,
            Err(_) => {
                let _ = m.fs.close(dev, &fdstate);
                log_error("BOOT", 974, "read /bin/init failed");
                return None;
            },
        };
        let _ = m.fs.close(dev, &fdstate);
        n
    };
    buf.truncate(n);
    Some(buf)
}

/// Kernel main loop: pump USB HID or PS/2 into the kernel event ring; HLT between batches.
///
/// Keyboard policy: raw PS/2 Set 1 bytes (including 0xE0 prefix and 0x80 break bit)
/// pushed directly into the kernel ring — userland owns scancode→character decoding
/// so layout is runtime-configurable without reboot.
fn input_forwarding_loop(_keyboard: &mut Keyboard, mouse: &mut Mouse) -> ! {
    // Pin the primary-source decision once at entry. USB present in Phase 9
    // means USB is authoritative for the rest of uptime; PS/2 only polls if
    // USB enumeration found nothing.
    let usb_active = morpheus_xhci::usb::runtime::keyboard_present()
        || morpheus_xhci::usb::runtime::mouse_present();

    loop {
        let mut had_work = false;

        if usb_active {
            // Pump the USB HID controller; parsed key events land in the kernel
            // keyboard event ring via the HID sink. The compositor drains that
            // ring through SYS_KEYBOARD_READ — we no longer bridge scancodes
            // into stdin here. We idle-HLT below; the 100 Hz timer wakes us to
            // pump again (HID poll latency ≈ one tick) and schedules compd to
            // drain on the same ticks.
            unsafe {
                morpheus_xhci::usb::runtime::poll_input();
            }
        } else {
            // PS/2 fallback. Drains up to 64 buffered bytes per outer
            // iteration to keep mouse input responsive.
            for _ in 0..64 {
                let raw = unsafe { tui::input::asm_ps2_poll_any() };
                if raw == 0 {
                    break;
                }
                had_work = true;

                let device = (raw >> 8) & 0xFF;
                let byte = (raw & 0xFF) as u8;

                if device == 0x03 {
                    if let Some(pkt) = mouse.feed(byte) {
                        morpheus_kernel::mouse::accumulate(pkt.dx, pkt.dy, pkt.buttons);
                    }
                    continue;
                }
                if device != 0x01 {
                    continue;
                }

                // PS/2 keyboard feeds the same kernel event ring as USB HID,
                // so the compositor drains both through SYS_KEYBOARD_READ.
                // `pressed=true` is the ring's "process this byte" flag; make/
                // break is encoded in the byte itself (|0x80).
                morpheus_kernel::input::push_keyboard_event_internal(
                    morpheus_kernel::input::InputEvent::Key(byte, true),
                );
            }
        }

        if !had_work {
            morpheus_kernel::schedular::mark_kernel_hlt();
            unsafe {
                core::arch::asm!("sti", "hlt", "cli", options(nostack, nomem));
            }
        }
    }
}

/// Log and halt. APs either still parked or quiesce on next tick.
fn boot_panic(component: &'static str, msg: &'static str) -> ! {
    boot_step_fail(msg);
    log_error(component, 999, msg);
    halt_forever();
}

fn halt_forever() -> ! {
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack));
        }
    }
}

extern crate alloc;
