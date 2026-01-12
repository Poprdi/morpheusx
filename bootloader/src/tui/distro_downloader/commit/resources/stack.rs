//! Stack allocation for bare-metal mode.

use crate::tui::renderer::{Screen, EFI_BLACK, EFI_CYAN, EFI_RED};

/// UEFI memory allocation types
const EFI_LOADER_DATA: usize = 2;
const EFI_ALLOCATE_ANY_PAGES: usize = 0;

/// Stack size for bare-metal mode
pub const STACK_SIZE: usize = 256 * 1024; // 256KB
const STACK_PAGES: usize = STACK_SIZE / 4096;

/// Allocate stack for bare-metal mode.
///
/// Returns (stack_base, stack_top) where stack_top is the initial SP.
pub unsafe fn allocate_stack(
    bs: &crate::BootServices,
    screen: &mut Screen,
    log_y: &mut usize,
) -> Result<(u64, u64), ()> {
    let mut stack_region: u64 = 0;

    let status = (bs.allocate_pages)(
        EFI_ALLOCATE_ANY_PAGES,
        EFI_LOADER_DATA,
        STACK_PAGES,
        &mut stack_region,
    );

    if status != 0 {
        screen.put_str_at(7, *log_y, "Stack allocation failed!", EFI_RED, EFI_BLACK);
        return Err(());
    }

    let stack_top = stack_region + STACK_SIZE as u64; // Stack grows down

    screen.put_str_at(
        7,
        *log_y,
        &alloc::format!("Stack: {:#x}", stack_region),
        EFI_CYAN,
        EFI_BLACK,
    );
    *log_y += 1;

    Ok((stack_region, stack_top))
}
