// Boot orchestrator - high-level API for booting a kernel

use super::{KernelImage, LinuxBootParams, boot_kernel};
use super::memory::{allocate_kernel_memory, allocate_boot_params, allocate_cmdline, load_kernel_image};
use crate::tui::renderer::{Screen, EFI_LIGHTGREEN, EFI_BLACK};

pub enum BootError {
    ParseFailed,
    MemoryAllocationFailed,
    LoadFailed,
}

// Boot a Linux kernel from a bzImage in memory
// kernel_data: Raw bzImage file contents
// cmdline: Kernel command line (e.g., "root=/dev/sda1 ro quiet")
// screen: For displaying progress
// This function never returns - it jumps to kernel
pub unsafe fn boot_linux_kernel(
    boot_services: &crate::BootServices,
    system_table: *mut (),
    image_handle: *mut (),
    kernel_data: &[u8],
    cmdline: &str,
    screen: &mut Screen,
) -> ! {
    let mut log_y = 18;
    
    screen.put_str_at(5, log_y, "Parsing kernel...", EFI_LIGHTGREEN, EFI_BLACK);
    log_y += 1;
    morpheus_core::logger::log("Parsing kernel...");
    
    // Parse kernel image
    let kernel = match KernelImage::parse(kernel_data) {
        Ok(k) => k,
        Err(_) => panic!("Failed to parse kernel"),
    };

    screen.put_str_at(5, log_y, "Allocating kernel memory...", EFI_LIGHTGREEN, EFI_BLACK);
    log_y += 1;
    morpheus_core::logger::log("Allocating kernel memory...");
    
    // Allocate memory for kernel
    let kernel_dest = match allocate_kernel_memory(boot_services, &kernel) {
        Ok(d) => d,
        Err(_) => panic!("Failed to allocate kernel memory"),
    };

    screen.put_str_at(5, log_y, "Loading kernel to memory...", EFI_LIGHTGREEN, EFI_BLACK);
    log_y += 1;
    morpheus_core::logger::log("Loading kernel to memory...");
    
    // Load kernel into memory
    let _ = load_kernel_image(&kernel, kernel_dest);

    screen.put_str_at(5, log_y, "Setting up boot params...", EFI_LIGHTGREEN, EFI_BLACK);
    log_y += 1;
    morpheus_core::logger::log("Setting up boot params...");
    
    // Allocate boot parameters
    let boot_params = match allocate_boot_params(boot_services) {
        Ok(b) => b,
        Err(_) => panic!("Failed to allocate boot params"),
    };

    // CRITICAL: Copy the setup header from kernel to boot params
    // The kernel expects to see its own setup header in boot_params
    (*boot_params).copy_setup_header(kernel.setup_header_ptr());
    
    // Setup boot params
    (*boot_params).set_loader_type(0xFF); // 0xFF = undefined loader
    (*boot_params).set_video_mode(); // Basic text mode
    
    // Allocate and set command line
    if !cmdline.is_empty() {
        if let Ok(cmdline_ptr) = allocate_cmdline(boot_services, cmdline) {
            (*boot_params).set_cmdline(cmdline_ptr as u32);
        }
    }

    screen.put_str_at(5, log_y, "Building E820 memory map...", EFI_LIGHTGREEN, EFI_BLACK);
    log_y += 1;
    morpheus_core::logger::log("Exiting boot services...");
    
    // Get memory map before exiting boot services
    let mut map_size: usize = 8192; // Start with reasonable buffer
    let mut map_key: usize = 0;
    let mut descriptor_size: usize = 0;
    let mut descriptor_version: u32 = 0;
    
    // Allocate buffer for memory map
    let mut map_buffer: *mut u8 = core::ptr::null_mut();
    let alloc_status = (boot_services.allocate_pool)(
        1, // EfiLoaderData
        map_size,
        &mut map_buffer,
    );
    
    // Get memory map
    let map_status = (boot_services.get_memory_map)(
        &mut map_size,
        map_buffer,
        &mut map_key,
        &mut descriptor_size,
        &mut descriptor_version,
    );
    
    // Build E820 memory map from UEFI memory map
    if alloc_status == 0 && map_status == 0 && !map_buffer.is_null() && descriptor_size > 0 {
        let num_descriptors = map_size / descriptor_size;
        let mut current = map_buffer as *const u8;
        
        // Safety: limit to reasonable number of entries
        let count = if num_descriptors > 128 { 128 } else { num_descriptors };
        
        for _ in 0..count {
            // UEFI memory descriptor: type(u32), pad(u32), phys_start(u64), virt_start(u64), num_pages(u64), attribute(u64)
            let mem_type = unsafe { *(current as *const u32) };
            let phys_start = unsafe { *(current.add(8) as *const u64) };
            let num_pages = unsafe { *(current.add(24) as *const u64) };
            let size = num_pages * 4096;
            
            // Convert UEFI type to E820 type
            // EfiConventionalMemory(7) → E820_RAM(1)
            // EfiACPIReclaimMemory(9) → E820_ACPI(3)
            // EfiACPIMemoryNVS(10) → E820_NVS(4)
            // Everything else → E820_RESERVED(2)
            let e820_type = match mem_type {
                7 => 1,  // RAM
                9 => 3,  // ACPI reclaimable
                10 => 4, // ACPI NVS
                _ => 2,  // Reserved
            };
            
            (*boot_params).add_e820_entry(phys_start, size, e820_type);
            
            current = unsafe { current.add(descriptor_size) };
        }
    }
    
    screen.put_str_at(5, log_y, "Built E820 memory map", EFI_LIGHTGREEN, EFI_BLACK);
    log_y += 1;
    morpheus_core::logger::log("Built E820 memory map");
    
    // DEBUG: Check what boot path we're taking
    use crate::boot::arch::x86_64::transitions::check_efi_handover_support;
    let setup_header = kernel.setup_header_bytes();
    let handover_offset = check_efi_handover_support(setup_header);
    
    if let Some(offset) = handover_offset {
        screen.put_str_at(5, log_y, &alloc::format!("EFI handover: offset={}", offset), EFI_LIGHTGREEN, EFI_BLACK);
    } else {
        screen.put_str_at(5, log_y, &alloc::format!("32-bit mode: code32_start={:#x}", kernel.code32_start()), EFI_LIGHTGREEN, EFI_BLACK);
    }
    log_y += 1;
    
    // Show where we loaded the kernel
    screen.put_str_at(5, log_y, &alloc::format!("Kernel loaded at: {:#x}", kernel_dest as usize), EFI_LIGHTGREEN, EFI_BLACK);
    log_y += 1;
    
    screen.put_str_at(5, log_y, "Exiting boot services...", EFI_LIGHTGREEN, EFI_BLACK);
    log_y += 1;
    
    // Get fresh memory map right before ExitBootServices
    // The previous map_key is now stale
    map_size = 8192;
    let _ = (boot_services.get_memory_map)(
        &mut map_size,
        map_buffer,
        &mut map_key,
        &mut descriptor_size,
        &mut descriptor_version,
    );
    
    // Exit boot services - kernel now owns hardware
    // This terminates UEFI runtime and gives full control to kernel
    let exit_status = (boot_services.exit_boot_services)(
        image_handle,
        map_key,
    );
    
    // If ExitBootServices fails, retry with fresh map key
    if exit_status != 0 {
        let _ = (boot_services.get_memory_map)(
            &mut map_size,
            map_buffer,
            &mut map_key,
            &mut descriptor_size,
            &mut descriptor_version,
        );
        let _ = (boot_services.exit_boot_services)(
            image_handle,
            map_key,
        );
    }

    screen.put_str_at(5, log_y, "Jumping to kernel...", EFI_LIGHTGREEN, EFI_BLACK);
    
    // CRITICAL: After ExitBootServices, we can't use UEFI services anymore
    // No more logging, no more panics - we're on our own
    
    // Jump to kernel (never returns)
    // kernel still has the setup header from original bzImage
    // kernel_dest is where we actually loaded the kernel code
    boot_kernel(&kernel, boot_params, system_table, kernel_dest)
}
