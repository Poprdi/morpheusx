//! MorpheusX Master Boot — single authoritative source of the boot chain.
//!
//! This file owns every transition from UEFI entry to the first userspace
//! process. Nothing else in the bootloader orchestrates boot order. Helper
//! modules (storage probing, BSoD rendering, UEFI allocator, PS/2 drivers,
//! network registration) are pure utilities invoked here in a fixed sequence.
//!
//! ## Design contract
//!
//! - One file, one sequence. If a boot step exists, it is invoked from
//!   `run()` below, in the order listed. No hidden boot work happens in
//!   driver or subsystem modules.
//! - No implicit cross-stage state. Every stage receives `&mut BootContext`
//!   and writes its outputs into it. Globals exist only for things that
//!   *must* survive exception context (the framebuffer snapshot read by the
//!   BSoD hook) — that one piece is documented and gated.
//! - No retries, no fallbacks invented at the orchestration layer. A stage
//!   either returns `Ok` and produces its declared output, or aborts via
//!   `boot_panic`. Subsystem-internal fallbacks (e.g. RAM-disk if no
//!   persistent storage) are still allowed inside their helper modules.
//! - Pre/post invariants for every stage are stated as code comments above
//!   the stage function.
//!
//! ## Ownership boundaries
//!
//! ```text
//!   firmware → BootContext (pre-EBS)
//!     → BootContext + memory map (EBS)
//!       → BootContext + platform handle (hwinit done)
//!         → BootContext + rootfs + display + APs (runtime ready)
//!           → userspace (never returns)
//! ```

#![allow(clippy::missing_safety_doc)]

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use morpheus_display::console::TextConsole;
use morpheus_hwinit::serial::{
    clear_live_console_hook, log_error, log_info, log_ok, log_warn, puts,
};

use crate::tui::input::Keyboard;
use crate::tui::mouse::Mouse;
use crate::{baremetal_ops, bsod, storage, tui, uefi_allocator};


/// Raw framebuffer info from GOP.
///
/// `stride` is `pixels_per_scan_line` from GOP — **not** bytes. Conversions
/// to byte-stride happen at the consumer (display crate).
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

/// Pre-EBS staged Helix image: an in-memory copy of `/morpheus/helix.img`
/// that was readable from the EFI System Partition before `ExitBootServices`.
///
/// Storage init consumes this if present, otherwise it falls back to
/// probing block devices on the PCI bus.
#[derive(Clone, Copy)]
pub struct PreEbsHelixImage {
    pub base: u64,
    pub size: usize,
    pub sector_size: u32,
}

/// Everything boot needs to know, accumulated stage by stage.
///
/// This struct replaces the soup of `static mut` globals that used to glue
/// boot stages together. Stages mutate fields explicitly; nothing outside
/// `boot.rs` may construct or own this type.
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

//
// The BSoD/panic screen runs from exception context where touching the
// heap or a SpinLock is forbidden. We publish a single framebuffer
// snapshot here, written exactly once during stage B, and read lock-free
// by the crash hook and the panic handler.
//
// This is the ONLY mutable global boot state. Everything else lives on
// the BootContext on the boot stack.

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

/// Crash-context-safe framebuffer accessor.
///
/// Returns `None` before stage B has run, otherwise the framebuffer info
/// snapshot. No locks, no heap, no alloc — safe from exception context.
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

//
// Owned by the live console stage. The hwinit serial layer holds a raw
// `unsafe fn(u8)` pointer to this function while the live console is
// active. The hook is dropped before userspace owns the framebuffer.

static mut LIVE_CONSOLE: Option<TextConsole> = None;

/// `puts()` mirror hook installed in stage B. Must not touch heap or locks.
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


/// EFI entry point. UEFI invokes this; we never return except via reset.
///
/// All the actual work is in `run()`. This wrapper exists solely to honor
/// the `efiapi` calling convention at the firmware boundary.
#[no_mangle]
pub extern "efiapi" fn efi_main(image_handle: *mut (), system_table: *const ()) -> usize {
    unsafe { run(image_handle, system_table) }
}

/// The canonical boot sequence. Every stage is invoked here in order.
///
/// # Safety
/// Called exactly once by UEFI with valid handle/table pointers. Returns
/// `!` from the perspective of the caller — but Rust types it as `usize`
/// to match the `efiapi` signature.
unsafe fn run(image_handle: *mut (), system_table: *const ()) -> ! {
    let mut ctx = BootContext::new();
    ctx.image_handle = image_handle;
    ctx.system_table = system_table;

    log_info("UEFI", 900, "efi_main entry");

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

    // Phase C — hardware: hwinit takes ownership of the machine.
    let platform = stage_c1_platform_init(&ctx);
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


/// A1. Hand the global allocator the UEFI BootServices pointer so that
/// pre-EBS allocations go through `allocate_pool`.
///
/// Post: hybrid allocator is in pre-EBS mode. Allocations are valid until
/// B3 runs.
unsafe fn stage_a1_arm_uefi_allocator(ctx: &BootContext) {
    let st = &*(ctx.system_table as *const EfiSystemTable);
    // `set_boot_services` accepts the bootloader's own `BootServices` type;
    // both this module and the allocator type-pun the same UEFI layout, so
    // a raw cast is sound here.
    uefi_allocator::set_boot_services(st.boot_services as *const _ as *const crate::BootServices);
}

/// A2. Locate GOP and capture the framebuffer descriptor.
///
/// Post: `ctx.framebuffer` is populated. A zero base is tolerated (the BSoD
/// will not render, but boot continues with serial-only logging).
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
        log_ok("UEFI", 901, "GOP framebuffer captured");
    } else {
        log_warn("UEFI", 902, "GOP not present; boot will run headless");
    }
}

/// A3. Walk the UEFI configuration table looking for an ACPI RSDP.
///
/// Pre: system table valid.
/// Post: `ctx.acpi_rsdp_phys` is the physical address of the RSDP, or 0
/// if neither ACPI 2.0 nor ACPI 1.0 is published.
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
            log_ok("ACPI", 902, "RSDP (ACPI 2.0) found");
            return;
        }
    }
    for t in tables {
        if t.vendor_guid == ACPI_10_TABLE_GUID {
            ctx.acpi_rsdp_phys = t.vendor_table as u64;
            log_ok("ACPI", 903, "RSDP (ACPI 1.0) found");
            return;
        }
    }
    log_warn("ACPI", 904, "no RSDP in UEFI configuration table");
}

/// A4. Capture our own PE image bounds. The buddy allocator excludes this
/// range so it never hands out the kernel's .text/.data/.bss as free RAM.
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

/// A5. Allocate a kernel stack from UEFI LoaderData. Survives EBS.
///
/// Pre: UEFI alive, allocator pre-EBS.
/// Post: `ctx.stack_base/pages/top` populated. Stack memory is in LoaderData
/// so it remains owned across `ExitBootServices`.
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
    log_ok("EBS", 905, "kernel stack allocated");
}

/// A6. Best-effort load of `/morpheus/helix.img` from the ESP into RAM
/// while UEFI's filesystem service is still alive.
///
/// Post: `ctx.pre_ebs_helix` is `Some` if the image was loaded and
/// sector-aligned, otherwise `None`. Storage init in stage D2 will use this
/// in preference to probing block devices.
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
    log_ok("EBS", 914, "pre-EBS staged /morpheus/helix.img into RAM");
}

/// A7. Capture the UEFI memory map immediately before EBS.
///
/// Pre: all earlier UEFI work done (no further allocate_pages may run).
/// Post: `ctx.mmap_*` reference a stable snapshot whose `map_key` is the
/// one we'll pass to `ExitBootServices`.
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

    // Stash the map_key for the EBS call. We use a static so the EBS stage
    // (which intentionally takes only `&ctx`) can read it without making
    // BootContext store firmware-internal handles.
    EBS_MAP_KEY.store(map_key, Ordering::Relaxed);
    log_ok("EBS", 915, "memory map captured");
}


static EBS_MAP_KEY: AtomicUsize = AtomicUsize::new(0);

/// B1. Call `ExitBootServices`. After this point UEFI services are dead.
///
/// Failure here is unrecoverable: the IDT still points into BootServicesCode
/// which is about to be reclaimed, so we can't even safely return.
unsafe fn stage_b1_exit_boot_services(ctx: &BootContext) {
    let st = &*(ctx.system_table as *const EfiSystemTable);
    let bs = &*st.boot_services;
    let map_key = EBS_MAP_KEY.load(Ordering::Relaxed);

    log_info("EBS", 920, "calling ExitBootServices");
    let status = (bs.exit_boot_services)(ctx.image_handle, map_key);
    if status != 0 {
        // OVMF DEBUG scrubs freed BootServicesCode with 0xAF, so a stale
        // map key or a layout-mismatched MinBootServices struct here is a
        // guaranteed crash. We can't even reliably log past this point.
        log_error("EBS", 921, "ExitBootServices failed");
        halt_forever();
    }
    log_ok("EBS", 922, "machine ownership transferred");
}

/// B2. Belt-and-suspenders `cli`. UEFI's PIC/APIC timer may still be armed
/// and its IDT just got freed. A single tick into the 0xAF scrub poisons
/// the buddy allocator's FreeNode pointers and corrupts memory.
unsafe fn stage_b2_disable_interrupts() {
    core::arch::asm!("cli", options(nomem, nostack));
}

/// B3. Switch the global allocator from UEFI pool to our static heap.
unsafe fn stage_b3_switch_allocator() {
    uefi_allocator::switch_to_post_ebs();
}

/// B4. Pivot RSP to the kernel stack we allocated in A5.
///
/// After this, all locals on the old UEFI stack are gone. Do not reference
/// anything that lived on it.
unsafe fn stage_b4_switch_stack(ctx: &BootContext) {
    let stack_top = ctx.stack_top;
    core::arch::asm!("mov rsp, {0}", in(reg) stack_top, options(nostack));
}

/// B5. Start mirroring all subsequent `puts()` to the framebuffer console.
///
/// Pre: framebuffer (if any) is valid; allocator is post-EBS.
/// Post: serial output appears on screen until the hook is cleared in E2.
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
    morpheus_hwinit::serial::set_live_console_hook(live_console_putc);
}

/// B6. Publish framebuffer info for crash-context consumers.
fn stage_b6_publish_framebuffer(ctx: &BootContext) {
    publish_framebuffer(&ctx.framebuffer);
}


/// C1. Hand the memory map + boot stack + image bounds + ACPI RSDP to
/// hwinit. After this returns, GDT/IDT/PIC→APIC/heap/TSC/DMA/PCI/paging/
/// USB-input/scheduler/syscalls/SMP are all live.
unsafe fn stage_c1_platform_init(ctx: &BootContext) -> morpheus_hwinit::PlatformInit {
    let cfg = morpheus_hwinit::SelfContainedConfig {
        memory_map_ptr: ctx.mmap_ptr,
        memory_map_size: ctx.mmap_size,
        descriptor_size: ctx.desc_size,
        descriptor_version: ctx.desc_ver,
        image_base: ctx.image_base,
        image_pages: ctx.image_pages,
        stack_base: ctx.stack_base,
        stack_pages: ctx.stack_pages,
        acpi_rsdp_phys: ctx.acpi_rsdp_phys,
    };
    match morpheus_hwinit::platform_init_selfcontained(cfg) {
        Ok(p) => {
            log_ok("BOOT", 930, "platform init complete");
            p
        }
        Err(_) => boot_panic("BOOT", "platform init failed"),
    }
}

/// C2. Wire the BSoD as the crash hook. Safe to do now because the
/// framebuffer snapshot was published in B6, so the hook can render even
/// from exception context.
unsafe fn stage_c2_register_crash_hook() {
    morpheus_hwinit::set_crash_hook(bsod::show_crash_screen);
    log_info("BOOT", 931, "crash hook registered");
}

/// C3. Record platform outputs (DMA region + TSC freq) into BootContext so
/// later stages don't need to re-borrow the `PlatformInit`.
fn stage_c3_record_platform_outputs(
    ctx: &mut BootContext,
    platform: &morpheus_hwinit::PlatformInit,
) {
    let dma = platform.dma();
    ctx.platform_dma_cpu = dma.cpu_base() as u64;
    ctx.platform_dma_bus = dma.bus_base();
    ctx.platform_dma_size = dma.size();
    ctx.tsc_freq = platform.tsc_freq();
}


/// D1. Register the userspace-triggered network activation callback. The
/// network stack stays offline until userspace explicitly opts in via
/// `SYS_NET_CFG(NET_CFG_ACTIVATE)`.
unsafe fn stage_d1_register_network_activation(
    _ctx: &BootContext,
    platform: &morpheus_hwinit::PlatformInit,
) {
    baremetal_ops::network::init_userspace_network_activation(
        morpheus_network::dma::DmaRegion::new(
            platform.dma().cpu_base(),
            platform.dma().bus_base(),
            platform.dma().size(),
        ),
        platform.tsc_freq(),
    );
}

/// D2. Bring up the root filesystem.
///
/// Order of preference inside storage:
///   1. Pre-EBS staged `/morpheus/helix.img` (if A6 produced one)
///   2. Persistent block device with valid HelixFS superblock
///   3. RAM HelixFS already mounted by hwinit phase 11
unsafe fn stage_d2_storage_init(ctx: &BootContext, platform: &morpheus_hwinit::PlatformInit) {
    storage::init_persistent_storage(platform.dma(), ctx.tsc_freq, ctx.pre_ebs_helix);
}

/// D3. Ensure `/bin /etc /tmp /home /var /dev` exist. Idempotent.
fn stage_d3_initfs_bootstrap() {
    storage::create_init_directories();
}

/// D4. Hand the framebuffer descriptor to the syscall layer so that
/// `SYS_FB_INFO` / `SYS_FB_MAP` work for userspace.
unsafe fn stage_d4_register_framebuffer(ctx: &BootContext) {
    if !ctx.framebuffer.is_valid() {
        log_warn("DISPLAY", 940, "no framebuffer to register");
        return;
    }
    let stride_bytes = ctx.framebuffer.stride * 4;
    morpheus_hwinit::register_framebuffer(morpheus_hwinit::FbInfo {
        base: ctx.framebuffer.base,
        size: ctx.framebuffer.size as u64,
        width: ctx.framebuffer.width,
        height: ctx.framebuffer.height,
        stride: stride_bytes,
        format: ctx.framebuffer.format,
    });
    log_ok("DISPLAY", 941, "framebuffer registered with syscall layer");
}


/// E1. Release the parked APs. From this point on, the system is fully SMP.
///
/// Pre: every preceding stage has completed. Any boot work that is not
/// SMP-safe must already have run.
unsafe fn stage_e1_release_aps() {
    morpheus_hwinit::cpu::ap_boot::release_parked_aps();
    log_ok("BOOT", 950, "APs released into scheduler");
}

/// E2. Final stage. Load `/bin/init`, spawn it, and become the kernel
/// PS/2 input-forwarding loop until reset.
unsafe fn stage_e2_enter_userspace(_ctx: &BootContext) -> ! {
    log_info("BOOT", 960, "preparing to launch /bin/init");

    // Hand the framebuffer over from the live serial mirror to userspace.
    // After this, /bin/init (or its descendants) own the display.
    let mut keyboard = Keyboard::new();
    boot_log_gate(&mut keyboard);
    clear_live_console_hook();

    let mut mouse = Mouse::new();

    let elf_data = match load_init_elf() {
        Some(d) => d,
        None => boot_panic("BOOT", "/bin/init not found"),
    };

    let init_pid = match morpheus_hwinit::process::scheduler::spawn_user_process(
        "init",
        &elf_data,
        &[],
        0,
        false,
    ) {
        Ok(pid) => {
            log_ok("BOOT", 961, "init process spawned");
            pid
        }
        Err(_) => boot_panic("BOOT", "failed to spawn /bin/init"),
    };
    let _ = init_pid;

    // The ELF buffer can drop now; init has its own address space.
    drop(elf_data);

    log_info("BOOT", 962, "entering input forwarding loop");
    input_forwarding_loop(&mut keyboard, &mut mouse);
}


/// Wait for any keypress before tearing down the live console.
///
/// The user sees the full boot log on screen; pressing a key signals
/// "ready to hand off to userspace." Polls BOTH the PS/2 keyboard and the
/// USB HID runtime — on real hardware without a PS/2 controller, USB is
/// the only path through this gate.
fn boot_log_gate(keyboard: &mut Keyboard) {
    puts("\nPress any key to start userspace...");
    loop {
        // PS/2 fast path
        if let Some(_key) = keyboard.read_key() {
            break;
        }

        // USB HID path — non-blocking peek of the event ring; if a report
        // arrived since the last poll, parse it and push the resulting
        // key events into the unified input queue.
        unsafe {
            morpheus_hwinit::usb::runtime::poll_keyboard();
        }

        // Drain unified queue. Any press event (PS/2 didn't put anything
        // here in this codepath, so this is the USB side) dismisses the gate.
        let mut got_press = false;
        while let Some(event) = morpheus_hwinit::poll_keyboard() {
            if let morpheus_hwinit::InputEvent::Key(_scan, pressed) = event {
                if pressed {
                    got_press = true;
                }
            }
        }
        if got_press {
            break;
        }
    }
    puts("\n");
}

/// Load `/bin/init` from the mounted root filesystem.
fn load_init_elf() -> Option<alloc::vec::Vec<u8>> {
    use alloc::string::String;

    let path = String::from("/bin/init");

    let fs = match unsafe { morpheus_helix::vfs::global::fs_global_mut() } {
        Some(f) => f,
        None => {
            log_error("BOOT", 970, "no root filesystem");
            return None;
        }
    };

    let stat = match morpheus_helix::vfs::vfs_stat(&fs.mount_table, &path) {
        Ok(s) => s,
        Err(_) => {
            log_error("BOOT", 971, "stat /bin/init failed");
            return None;
        }
    };
    if stat.size == 0 {
        log_error("BOOT", 972, "/bin/init has zero size");
        return None;
    }

    let mut fd_table = morpheus_helix::vfs::FdTable::new();
    let ts = morpheus_hwinit::cpu::tsc::read_tsc();
    let fd = match morpheus_helix::vfs::vfs_open(
        &mut fs.device,
        &mut fs.mount_table,
        &mut fd_table,
        &path,
        morpheus_helix::types::open_flags::O_READ,
        ts,
    ) {
        Ok(f) => f,
        Err(_) => {
            log_error("BOOT", 973, "open /bin/init failed");
            return None;
        }
    };

    let mut buf = alloc::vec![0u8; stat.size as usize];
    let n = match morpheus_helix::vfs::vfs_read(
        &mut fs.device,
        &fs.mount_table,
        &mut fd_table,
        fd,
        &mut buf,
    ) {
        Ok(bytes) => bytes,
        Err(_) => {
            let _ = morpheus_helix::vfs::vfs_close(&mut fd_table, fd);
            log_error("BOOT", 974, "read /bin/init failed");
            return None;
        }
    };
    buf.truncate(n);
    let _ = morpheus_helix::vfs::vfs_close(&mut fd_table, fd);
    log_ok("BOOT", 975, "/bin/init loaded");
    Some(buf)
}

/// Kernel main loop: poll PS/2 keyboard + mouse, feed kernel stdin and the
/// mouse accumulator. HLTs between batches to keep idle power low.
fn input_forwarding_loop(keyboard: &mut Keyboard, mouse: &mut Mouse) -> ! {
    loop {
        let mut had_work = false;

        // PS/2 polling — unchanged. Drains up to 64 buffered bytes per
        // outer iteration to keep mouse input responsive while the keyboard
        // is also active.
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
                    morpheus_hwinit::mouse::accumulate(pkt.dx, pkt.dy, pkt.buttons);
                }
                continue;
            }
            if device != 0x01 {
                continue;
            }

            if let Some(input) = keyboard.feed_raw(byte) {
                forward_keyboard_byte(input);
            }
        }

        // USB HID polling. `poll_keyboard` is non-blocking: it peeks the
        // event ring and, if the keyboard sent a report since last poll,
        // parses it and pushes per-key `InputEvent`s into the unified input
        // queue via `push_keyboard_event_internal`. Re-arms the interrupt-IN
        // transfer automatically.
        unsafe {
            morpheus_hwinit::usb::runtime::poll_keyboard();
        }

        // Drain any USB-derived key events from the unified input queue and
        // feed them through the same PS/2 scan-code translator the PS/2 path
        // uses — `parse_keyboard_report` translated HID usage codes to Set-1
        // make codes already, so `keyboard.feed_raw` sees the same bytes it
        // would from an actual PS/2 keypress.
        while let Some(event) = morpheus_hwinit::poll_keyboard() {
            if let morpheus_hwinit::InputEvent::Key(scan_code, pressed) = event {
                if pressed && scan_code != 0 {
                    had_work = true;
                    if let Some(input) = keyboard.feed_raw(scan_code) {
                        forward_keyboard_byte(input);
                    }
                }
            }
        }

        if !had_work {
            morpheus_hwinit::process::scheduler::mark_kernel_hlt();
            unsafe {
                core::arch::asm!("sti", "hlt", "cli", options(nostack, nomem));
            }
        }
    }
}

/// Route a decoded keyboard event into the kernel stdin ring buffer, with
/// Ctrl+C routed as `SIGINT` to the foreground process if one is set.
fn forward_keyboard_byte(input: crate::tui::input::InputKey) {
    if input.unicode_char == 0 || input.unicode_char > 0xFF {
        return;
    }
    let ch = input.unicode_char as u8;

    if ch == 0x03 {
        let fg = morpheus_hwinit::stdin::foreground_pid();
        if fg != 0 {
            unsafe {
                let _ = morpheus_hwinit::process::SCHEDULER
                    .send_signal(fg, morpheus_hwinit::process::signals::Signal::SIGINT);
            }
            return;
        }
    }

    morpheus_hwinit::stdin::push(ch);
    unsafe { morpheus_hwinit::process::wake_stdin_waiters() };
}


/// Unrecoverable boot error. Logs and halts the BSP. APs are either still
/// parked (pre-E1) or will quiesce on their next tick; either way the
/// machine is dead from the user's perspective.
fn boot_panic(component: &'static str, msg: &'static str) -> ! {
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

//
// Old code (`bsod.rs`) used `crate::baremetal::get_framebuffer_info()` to
// read the published framebuffer. The new authoritative accessor is
// `boot::published_framebuffer()`. The shim below provides backwards
// compatibility *only* for the panic handler call sites that we don't
// want to touch in this refactor pass; new code must call
// `published_framebuffer()` directly.

extern crate alloc;
