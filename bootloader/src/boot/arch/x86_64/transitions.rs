//! CPU mode transitions
//! 
//! x86_64 supports multiple execution modes:
//! - Real mode (16-bit, BIOS startup)
//! - Protected mode (32-bit)
//! - Long mode (64-bit, UEFI startup)
//! 
//! Linux kernel expects 32-bit protected mode entry for traditional boot.
//! We need to transition between modes depending on firmware type.

use core::arch::asm;

/// 32-bit GDT for protected mode handoff to kernel
/// 
/// Layout:
/// - Null descriptor (0x00)
/// - Code segment (0x08) - 32-bit, present, executable
/// - Data segment (0x10) - 32-bit, present, writable
#[repr(C, packed)]
struct Gdt32 {
    null: u64,
    code: u64,
    data: u64,
}

#[repr(C, packed)]
struct GdtDescriptor {
    limit: u16,
    base: u64,
}

/// 32-bit GDT for kernel handoff
static mut KERNEL_GDT_32: Gdt32 = Gdt32 {
    null: 0x0000_0000_0000_0000,
    // Code: base=0, limit=0xFFFFF, 32-bit, present, DPL=0, executable, readable
    code: 0x00cf_9a00_0000_ffff,
    // Data: base=0, limit=0xFFFFF, 32-bit, present, DPL=0, writable
    data: 0x00cf_9200_0000_ffff,
};

/// Setup 32-bit GDT for protected mode
/// 
/// Must be called before transitioning from 64-bit to 32-bit
pub unsafe fn setup_32bit_gdt() -> *const GdtDescriptor {
    let gdt_desc = GdtDescriptor {
        limit: (core::mem::size_of::<Gdt32>() - 1) as u16,
        base: &KERNEL_GDT_32 as *const _ as u64,
    };
    
    // Return pointer to descriptor - caller will lgdt it
    &gdt_desc as *const _
}

/// Drop from 64-bit long mode to 32-bit protected mode
/// 
/// This is the critical transition for UEFI → Linux kernel handoff
/// when kernel doesn't support EFI handover protocol.
/// 
/// Steps:
/// 1. Load 32-bit GDT
/// 2. Disable paging (clear CR0.PG)
/// 3. Disable long mode (clear EFER.LME)
/// 4. Jump to 32-bit code segment
/// 5. Reload segment registers for 32-bit
/// 
/// DANGER: Point of no return. After this, no UEFI services available.
/// ExitBootServices must be called before this.
/// 
/// Arguments:
/// - entry_point: 32-bit kernel entry address
/// - boot_params: pointer to Linux boot_params (goes in ESI)
/// 
/// Does NOT return - jumps to kernel.
#[unsafe(naked)]
pub unsafe extern "C" fn drop_to_protected_mode(entry_point: u32, boot_params: u32) -> ! {
    core::arch::naked_asm!(
        ".att_syntax",
        
        // Save arguments (edi=entry_point, esi=boot_params)
        "mov %edi, %r14d",           // entry_point → R14D
        "mov %esi, %r15d",           // boot_params → R15D
        
        // Disable interrupts (no coming back)
        "cli",
        
        // Disable paging: CR0.PG = 0
        "mov %cr0, %rax",
        "and $0x7fffffff, %eax",     // Clear bit 31 (PG)
        "mov %rax, %cr0",
        
        // Disable long mode: EFER.LME = 0
        "mov $0xc0000080, %ecx",     // EFER MSR
        "rdmsr",
        "and $0xfffffeff, %eax",     // Clear bit 8 (LME)
        "wrmsr",
        
        // Switch to compatibility mode  
        // We're now in 32-bit protected mode
        ".code32",
        ".intel_syntax noprefix",
        
        // Zero out registers (except ESI which holds boot_params)
        "xor eax, eax",
        "xor ebx, ebx",
        "xor ecx, ecx",
        "xor edx, edx",
        "xor edi, edi",
        "xor ebp, ebp",
        
        // Set boot_params pointer in ESI
        "mov esi, r15d",
        
        // Jump to kernel entry point
        "jmp r14d",
        
        ".att_syntax",
        ".code64",
    )
}

/// Alternative: Stay in 64-bit and use EFI handover protocol
/// 
/// Modern kernels (2.6.30+) support direct EFI handoff in 64-bit mode.
/// This is preferred when available - no mode switching needed.
/// 
/// Kernel entry point = startup_64 + handover_offset
/// RSI = boot_params pointer
/// 
/// Much simpler than mode switching, but not universally supported.
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
