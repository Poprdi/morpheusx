//! CPU mode transitions for x86_64
//!
//! Handles transitions between 64-bit long mode (UEFI) and 32-bit protected mode (Linux kernel).

// External assembly function for 64→32 bit transition
extern "C" {
    fn drop_to_protected_mode_asm(entry_point: u32, boot_params: u32) -> !;
}

/// Drop from 64-bit long mode to 32-bit protected mode
///
/// This is implemented in external assembly (trampoline32.asm) to properly
/// handle the mode transition without compiler interference.
///
/// ONLY used for legacy kernels that don't support EFI handover protocol.
/// Modern kernels (2.6.30+) use EFI handover and stay in 64-bit mode.
///
/// When using this path, the bootloader MUST call ExitBootServices first.
///
/// Arguments:
/// - entry_point: 32-bit kernel entry address
/// - boot_params: pointer to Linux boot_params (goes in ESI)
///
/// Does NOT return - jumps to kernel.
#[unsafe(naked)]
pub unsafe extern "C" fn drop_to_protected_mode(entry_point: u32, boot_params: u32) -> ! {
    core::arch::naked_asm!(
        "lea rax, [rip + drop_to_protected_mode_asm]",
        "jmp rax",
        "ud2",
    )
}

/// Check if kernel supports EFI handover protocol
///
/// Modern kernels (2.6.30+) support direct EFI handoff in 64-bit mode.
/// This is preferred when available - no mode switching needed.
///
/// Returns handover_offset if supported, None otherwise.
pub unsafe fn check_efi_handover_support(setup_header: &[u8]) -> Option<u32> {
    // handover_offset is at offset 0x264 in setup header (since kernel 2.6.30)
    if setup_header.len() < 0x268 {
        return None;
    }

    // Read handover_offset (u32 at 0x264)
    let offset = u32::from_le_bytes([
        setup_header[0x264],
        setup_header[0x265],
        setup_header[0x266],
        setup_header[0x267],
    ]);

    // Zero means not supported
    if offset == 0 {
        None
    } else {
        Some(offset)
    }
}
