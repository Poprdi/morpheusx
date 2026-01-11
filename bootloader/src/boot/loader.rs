// Boot orchestrator - high-level API for booting a kernel

use core::ptr;

use super::memory::{
    allocate_boot_params, allocate_cmdline, allocate_kernel_memory, allocate_low_buffer,
    exit_boot_services, load_kernel_image, MemoryError, MemoryMap, INITRD_MIN_ADDR,
};
use super::{boot_kernel, efi_stub, kernel_loader::KernelError, KernelImage, LinuxBootParams};
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_LIGHTGREEN};

const LOADER_TYPE_UEFI: u8 = 0x30;
const EFI_LOADER_SIGNATURE_EL64: u32 = 0x34364C45; // "EL64"

const EFI_ACPI_TABLE_GUID: [u8; 16] = [
    0x30, 0x2d, 0x9d, 0xeb, 0x88, 0x2d, 0xd3, 0x11, 0x9a, 0x16, 0x00, 0x90, 0x27, 0x3f, 0xc1, 0x4d,
];
const EFI_ACPI_20_TABLE_GUID: [u8; 16] = [
    0x71, 0xe8, 0x68, 0x88, 0xf1, 0x04, 0xd3, 0x11, 0xbc, 0x22, 0x00, 0x80, 0xc7, 0x3c, 0x88, 0x81,
];

#[derive(Debug)]
pub enum BootError {
    KernelParse(KernelError),
    KernelAllocation(MemoryError),
    KernelLoad(MemoryError),
    BootParamsAllocation(MemoryError),
    CmdlineAllocation(MemoryError),
    InitrdAllocation(MemoryError),
    MemorySnapshot(MemoryError),
    ExitBootServices(MemoryError),
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
    initrd_data: Option<&[u8]>,
    cmdline: &str,
    screen: &mut Screen,
) -> Result<core::convert::Infallible, BootError> {
    let mut log_y = 18;

    screen.put_str_at(5, log_y, "Parsing kernel...", EFI_LIGHTGREEN, EFI_BLACK);
    log_y += 1;
    morpheus_core::logger::log("Parsing kernel...");

    let kernel = KernelImage::parse(kernel_data).map_err(BootError::KernelParse)?;

    match unsafe {
        efi_stub::boot_via_efi_stub(
            boot_services,
            image_handle,
            kernel_data,
            initrd_data,
            cmdline,
        )
    } {
        Ok(never) => return Ok(never),
        Err(error) => {
            screen.put_str_at(
                5,
                log_y,
                "EFI stub path unavailable, falling back...",
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );
            log_y += 1;
            let err_line = alloc::format!("EFI stub path unavailable, falling back ({:?})", error);
            morpheus_core::logger::log(err_line.leak());
        }
    }

    screen.put_str_at(
        5,
        log_y,
        "Allocating kernel memory...",
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );
    log_y += 1;
    morpheus_core::logger::log("Allocating kernel memory...");

    let kernel_dest =
        allocate_kernel_memory(boot_services, &kernel).map_err(BootError::KernelAllocation)?;

    screen.put_str_at(
        5,
        log_y,
        "Loading kernel to memory...",
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );
    log_y += 1;
    morpheus_core::logger::log("Loading kernel to memory...");

    load_kernel_image(&kernel, kernel_dest).map_err(BootError::KernelLoad)?;

    screen.put_str_at(
        5,
        log_y,
        "Setting up boot params...",
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );
    log_y += 1;
    morpheus_core::logger::log("Setting up boot params...");

    let boot_params_ptr =
        allocate_boot_params(boot_services).map_err(BootError::BootParamsAllocation)?;
    let boot_params = &mut *boot_params_ptr;
    *boot_params = LinuxBootParams::new();
    boot_params.copy_setup_header(kernel.setup_header_ptr());
    boot_params.set_loader_type(LOADER_TYPE_UEFI);
    boot_params.set_video_mode();

    if !cmdline.is_empty() {
        let limit = kernel.cmdline_limit().saturating_sub(1).max(1);
        let slice = truncate_cmdline(cmdline, limit as usize);
        let cmdline_ptr =
            allocate_cmdline(boot_services, slice).map_err(BootError::CmdlineAllocation)?;
        boot_params.set_cmdline(cmdline_ptr as u64, (slice.len() + 1) as u32);
    }

    if let Some(initrd) = initrd_data {
        if !initrd.is_empty() {
            screen.put_str_at(
                5,
                log_y,
                "Allocating initrd memory...",
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );
            log_y += 1;
            morpheus_core::logger::log("initrd alloc start");

            let limit = kernel.initrd_addr_max() as u64;
            let max_addr = if limit == 0 { 0xFFFF_FFFF } else { limit };

            let log_msg = alloc::format!(
                "Initrd allocation: limit={:#x}, max_addr={:#x}, size={:#x}",
                limit,
                max_addr,
                initrd.len()
            );
            morpheus_core::logger::log(log_msg.leak());

            let initrd_ptr = allocate_low_buffer(boot_services, max_addr, initrd.len())
                .map_err(BootError::InitrdAllocation)?;

            morpheus_core::logger::log("initrd alloc ok");
            screen.put_str_at(
                5,
                log_y,
                "Copying initrd to memory...",
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );
            log_y += 1;
            morpheus_core::logger::log("initrd copy start");

            ptr::copy_nonoverlapping(initrd.as_ptr(), initrd_ptr, initrd.len());

            morpheus_core::logger::log("initrd copy ok");
            screen.put_str_at(
                5,
                log_y,
                &alloc::format!(
                    "Initrd @ {:#x} ({} bytes)",
                    initrd_ptr as usize,
                    initrd.len()
                ),
                EFI_LIGHTGREEN,
                EFI_BLACK,
            );
            log_y += 1;

            // Re-enable ramdisk fields
            boot_params.set_ramdisk(initrd_ptr as u64, initrd.len() as u64);

            let ramdisk_start = initrd_ptr as u64;
            let ramdisk_end = ramdisk_start + initrd.len() as u64;

            // Validate initrd address - these are critical errors, not warnings
            if ramdisk_start < INITRD_MIN_ADDR {
                morpheus_core::logger::log(
                    "ERROR: initrd allocated below 1MiB - kernel will fault!",
                );
                return Err(BootError::InitrdAllocation(MemoryError::AllocationFailed));
            }
            if ramdisk_end > max_addr + 1 {
                morpheus_core::logger::log("ERROR: initrd extends past max allowed address!");
                return Err(BootError::InitrdAllocation(MemoryError::AllocationFailed));
            }

            let (ramdisk_image_field, ramdisk_size_field) = boot_params.ramdisk_info();
            let ramdisk_field_line = alloc::format!(
                "Boot params ramdisk image={:#x}, size={:#x}",
                ramdisk_image_field,
                ramdisk_size_field
            );
            screen.put_str_at(5, log_y, &ramdisk_field_line, EFI_LIGHTGREEN, EFI_BLACK);
            log_y += 1;
        }
    }

    screen.put_str_at(
        5,
        log_y,
        "Building E820 memory map...",
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );
    log_y += 1;
    morpheus_core::logger::log("Building E820 memory map...");

    let mut memory_map = MemoryMap::new();
    memory_map
        .ensure_snapshot(boot_services)
        .map_err(BootError::MemorySnapshot)?;
    let highest_ram_end = build_e820(boot_params, &memory_map);
    boot_params.set_alt_mem_k((highest_ram_end / 1024) as u32);

    let systab_ptr = system_table as u64;
    boot_params.set_efi_info(
        EFI_LOADER_SIGNATURE_EL64,
        systab_ptr,
        memory_map.buffer_ptr() as u64,
        memory_map.size as u32,
        memory_map.descriptor_size as u32,
        memory_map.descriptor_version,
    );

    if let Some(rsdp) = find_rsdp(system_table as *const RawSystemTable) {
        boot_params.set_acpi_rsdp(rsdp);
    }

    screen.put_str_at(5, log_y, "Built E820 memory map", EFI_LIGHTGREEN, EFI_BLACK);
    log_y += 1;

    let boot_params_phys = boot_params_ptr as usize;
    screen.put_str_at(
        5,
        log_y,
        &alloc::format!("Boot params @ {:#x}", boot_params_phys),
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );
    log_y += 1;

    screen.put_str_at(
        5,
        log_y,
        &alloc::format!("Kernel loaded at: {:#x}", kernel_dest as usize),
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );
    log_y += 1;

    // CRITICAL: With EFI handover protocol, the KERNEL calls ExitBootServices, not us!
    // Only exit boot services for legacy 32-bit protected mode boot.
    let handover_offset = kernel.handover_offset();
    let uses_efi_handover = handover_offset != 0;

    if !uses_efi_handover {
        // Legacy 32-bit protected mode: we must exit boot services
        screen.put_str_at(
            5,
            log_y,
            "Exiting boot services...",
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );
        log_y += 1;

        exit_boot_services(boot_services, image_handle, &mut memory_map)
            .map_err(BootError::ExitBootServices)?;

        // DO NOT use screen after exiting boot services - UEFI GOP is no longer available
    } else {
        // EFI Handover Protocol: kernel will call ExitBootServices itself
        screen.put_str_at(
            5,
            log_y,
            "Jumping to kernel (handover protocol)...",
            EFI_LIGHTGREEN,
            EFI_BLACK,
        );
        log_y += 1;
        morpheus_core::logger::log("About to call handover");

        // Log handover parameters for debugging
        let entry_addr = kernel_dest as u64 + handover_offset as u64 + 512;
        let log_line = alloc::format!(
            "Handover: entry={:#x} img_handle={:p} sys_table={:p} boot_params={:p}",
            entry_addr,
            image_handle,
            system_table,
            boot_params_ptr
        );
        morpheus_core::logger::log(log_line.leak());
    }

    boot_kernel(
        &kernel,
        boot_params_ptr,
        system_table,
        image_handle,
        kernel_dest,
    )
}

fn truncate_cmdline<'a>(cmdline: &'a str, max_bytes: usize) -> &'a str {
    if cmdline.len() <= max_bytes {
        return cmdline;
    }

    // Log warning about truncation - this could break boot!
    morpheus_core::logger::log("WARNING: Command line too long, truncating!");
    let truncated_msg = alloc::format!(
        "cmdline {} bytes > {} max, critical params may be lost!",
        cmdline.len(),
        max_bytes
    );
    morpheus_core::logger::log(truncated_msg.leak());

    let mut end = max_bytes;
    while end > 0 && !cmdline.is_char_boundary(end) {
        end -= 1;
    }
    &cmdline[..end]
}

fn build_e820(boot_params: &mut LinuxBootParams, memory_map: &MemoryMap) -> u64 {
    let mut highest_ram_end = 0u64;
    for descriptor in memory_map.descriptors() {
        let size = descriptor.number_of_pages * 4096;
        if size == 0 {
            continue;
        }
        let entry_type = map_uefi_type(descriptor.typ);
        boot_params.add_e820_entry(descriptor.physical_start, size, entry_type);
        if entry_type == 1 {
            highest_ram_end = highest_ram_end.max(descriptor.physical_start + size);
        }
    }
    highest_ram_end
}

fn map_uefi_type(typ: u32) -> u32 {
    match typ {
        1 | 2 | 3 | 4 | 7 => 1, // RAM
        9 => 3,                 // ACPI reclaim
        10 => 4,                // ACPI NVS
        8 => 5,                 // Unusable
        _ => 2,                 // Reserved
    }
}

fn find_rsdp(system_table: *const RawSystemTable) -> Option<u64> {
    if system_table.is_null() {
        return None;
    }
    unsafe {
        let table = &*system_table;
        let mut entry = table.configuration_table;
        for _ in 0..table.number_of_table_entries {
            if entry.is_null() {
                break;
            }
            let config = &*entry;
            if guid_equals(&config.vendor_guid, &EFI_ACPI_20_TABLE_GUID)
                || guid_equals(&config.vendor_guid, &EFI_ACPI_TABLE_GUID)
            {
                return Some(config.vendor_table as u64);
            }
            entry = entry.add(1);
        }
    }
    None
}

fn guid_equals(lhs: &[u8; 16], rhs: &[u8; 16]) -> bool {
    lhs.iter().zip(rhs.iter()).all(|(a, b)| a == b)
}

#[repr(C)]
struct RawSystemTable {
    _header: [u8; 24],
    _firmware_vendor: *const u16,
    _firmware_revision: u32,
    _console_in_handle: *const (),
    _con_in: *const (),
    _console_out_handle: *const (),
    _con_out: *const (),
    _stderr_handle: *const (),
    _stderr: *const (),
    _runtime_services: *const (),
    _boot_services: *const (),
    number_of_table_entries: usize,
    configuration_table: *const RawConfigurationTable,
}

#[repr(C)]
struct RawConfigurationTable {
    vendor_guid: [u8; 16],
    vendor_table: *const (),
}
