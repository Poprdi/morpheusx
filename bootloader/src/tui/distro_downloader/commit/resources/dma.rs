//! DMA region allocation for bare-metal networking.

use crate::tui::renderer::{Screen, EFI_BLACK, EFI_CYAN, EFI_RED};
use core::ptr;

/// UEFI memory allocation types
const EFI_LOADER_DATA: usize = 2;
const EFI_ALLOCATE_MAX_ADDRESS: usize = 1;

/// DMA region size (8MB)
pub const DMA_SIZE: usize = 8 * 1024 * 1024;
const DMA_PAGES: usize = DMA_SIZE / 4096;

/// Allocate DMA region (must be <4GB for VirtIO).
///
/// Returns physical address of DMA region.
pub unsafe fn allocate_dma_region(
    bs: &crate::BootServices,
    screen: &mut Screen,
    log_y: &mut usize,
) -> Result<u64, ()> {
    let mut dma_region: u64 = 0xFFFF_FFFF; // Max address hint (<4GB)

    let status = (bs.allocate_pages)(
        EFI_ALLOCATE_MAX_ADDRESS,
        EFI_LOADER_DATA,
        DMA_PAGES,
        &mut dma_region,
    );

    if status != 0 {
        screen.put_str_at(7, *log_y, "DMA allocation failed!", EFI_RED, EFI_BLACK);
        *log_y += 1;
        screen.put_str_at(
            7,
            *log_y,
            "Cannot proceed with download",
            EFI_RED,
            EFI_BLACK,
        );
        return Err(());
    }

    // Zero the DMA region
    ptr::write_bytes(dma_region as *mut u8, 0, DMA_SIZE);

    screen.put_str_at(
        7,
        *log_y,
        &alloc::format!("DMA base: {:#x}", dma_region),
        EFI_CYAN,
        EFI_BLACK,
    );
    *log_y += 1;

    Ok(dma_region)
}
