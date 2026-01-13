//! Boot handoff preparation for bare-metal transition.

use crate::boot::gop::GopFramebufferInfo;
use crate::boot::network_boot::{prepare_handoff_full, BlkProbeResult, NicProbeResult};
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_CYAN, EFI_RED};
use core::ptr;
use morpheus_network::boot::handoff::BootHandoff;

/// UEFI memory allocation types
const EFI_LOADER_DATA: usize = 2;
const EFI_ALLOCATE_ANY_PAGES: usize = 0;

/// Prepare boot handoff structure for bare-metal mode.
///
/// Returns a static reference to the handoff structure.
#[allow(clippy::too_many_arguments)]
pub unsafe fn prepare_boot_handoff(
    bs: &crate::BootServices,
    nic_probe: &NicProbeResult,
    blk_probe: &BlkProbeResult,
    dma_region: u64,
    dma_size: u64,
    tsc_freq: u64,
    stack_top: u64,
    stack_size: u64,
    gop_info: Option<&GopFramebufferInfo>,
    screen: &mut Screen,
    log_y: &mut usize,
) -> Result<&'static BootHandoff, ()> {
    // Allocate handoff in loader data so it survives EBS
    let mut handoff_page: u64 = 0;
    let status = (bs.allocate_pages)(
        EFI_ALLOCATE_ANY_PAGES,
        EFI_LOADER_DATA,
        1, // 4KB is plenty
        &mut handoff_page,
    );

    if status != 0 {
        screen.put_str_at(7, *log_y, "Handoff allocation failed!", EFI_RED, EFI_BLACK);
        return Err(());
    }

    let handoff_ptr = handoff_page as *mut BootHandoff;

    // QEMU default MAC (placeholder - should be read from device)
    let mac = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];

    // Extract framebuffer info (or zeros if no GOP)
    let (fb_base, fb_width, fb_height, fb_stride, fb_format) = match gop_info {
        Some(fb) => (fb.base, fb.width, fb.height, fb.stride, fb.format),
        None => (0, 0, 0, 0, 0),
    };

    let handoff = prepare_handoff_full(
        nic_probe, blk_probe, mac, dma_region, dma_region, // Bus addr = CPU addr (no IOMMU)
        dma_size, tsc_freq, stack_top, stack_size, fb_base, fb_width, fb_height, fb_stride,
        fb_format,
    );

    ptr::write(handoff_ptr, handoff);
    let handoff_ref: &'static BootHandoff = &*handoff_ptr;

    screen.put_str_at(7, *log_y, "Handoff ready", EFI_CYAN, EFI_BLACK);
    *log_y += 1;

    Ok(handoff_ref)
}
