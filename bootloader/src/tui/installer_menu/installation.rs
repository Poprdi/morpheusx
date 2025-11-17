// Installation operations and feedback rendering

use crate::installer::{self, EspInfo};
use crate::tui::input::Keyboard;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_CYAN, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN, EFI_WHITE};
use crate::BootServices;
use alloc::format;
use morpheus_persistent::feedback::{FeedbackCategory, FeedbackCollector, FeedbackLevel};
use morpheus_persistent::pe::header::PeHeaders;

pub fn install_to_selected(
    esp: &EspInfo,
    screen: &mut Screen,
    keyboard: &mut Keyboard,
    bs: &BootServices,
    image_handle: *mut (),
) {
    screen.clear();
    let start_x = 2;
    let mut y = 1;

    screen.put_str_at(
        start_x,
        y,
        "=== PERSISTENCE INSTALLER ===",
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );
    y += 1;

    // Create feedback collector
    let mut feedback = FeedbackCollector::new(50);

    // Phase 1: Parse PE headers from running bootloader
    y += 1;
    screen.put_str_at(
        start_x,
        y,
        "--- PE Header Analysis ---",
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );
    y += 1;

    feedback.info(
        FeedbackCategory::PeHeader,
        "Analyzing running bootloader image...",
    );
    render_feedback(screen, &feedback, start_x, &mut y);

    let loaded_image = unsafe { crate::uefi::file_system::get_loaded_image(bs, image_handle) };

    if loaded_image.is_err() {
        feedback.error(
            FeedbackCategory::General,
            "Failed to get LoadedImageProtocol",
        );
        render_feedback(screen, &feedback, start_x, &mut y);
        screen.put_str_at(
            start_x,
            y + 2,
            "Press any key to return...",
            EFI_DARKGREEN,
            EFI_BLACK,
        );
        keyboard.wait_for_key();
        return;
    }

    let loaded_image = unsafe { &*loaded_image.unwrap() };
    let image_base = loaded_image.image_base as *const u8;
    let image_size = loaded_image.image_size as usize;

    feedback.success(
        FeedbackCategory::Memory,
        format!("Image loaded at: 0x{:016X}", image_base as u64),
    );
    feedback.info(
        FeedbackCategory::Memory,
        format!("Image size: {} bytes", image_size),
    );
    render_feedback(screen, &feedback, start_x, &mut y);

    // Parse PE headers
    feedback.info(FeedbackCategory::PeHeader, "Parsing DOS/PE/COFF headers...");
    render_feedback(screen, &feedback, start_x, &mut y);

    let pe_headers = unsafe { PeHeaders::parse(image_base, image_size) };

    match pe_headers {
        Ok(headers) => {
            analyze_pe_headers(&headers, &mut feedback, screen, start_x, &mut y);
            reconstruct_image_base(&headers, image_base, image_size, &mut feedback, screen, start_x, &mut y);
            perform_installation(esp, bs, image_handle, &mut feedback, screen, start_x, &mut y);
        }
        Err(e) => {
            feedback.error(
                FeedbackCategory::PeHeader,
                format!("PE parsing failed: {}", e),
            );
            render_feedback(screen, &feedback, start_x, &mut y);
        }
    }

    screen.put_str_at(
        start_x,
        y + 2,
        "Press any key to return...",
        EFI_DARKGREEN,
        EFI_BLACK,
    );
    keyboard.wait_for_key();
}

fn analyze_pe_headers(
    headers: &PeHeaders,
    feedback: &mut FeedbackCollector,
    screen: &mut Screen,
    start_x: usize,
    y: &mut usize,
) {
    feedback.success(FeedbackCategory::PeHeader, "PE headers parsed successfully");
    feedback.info(
        FeedbackCategory::PeHeader,
        format!("Architecture: {}", headers.coff.machine_name()),
    );
    feedback.info(
        FeedbackCategory::PeHeader,
        format!(
            "ImageBase (in memory - PATCHED): 0x{:016X}",
            headers.optional.image_base
        ),
    );
    feedback.info(
        FeedbackCategory::PeHeader,
        format!("Sections: {}", headers.coff.number_of_sections),
    );
    render_feedback(screen, feedback, start_x, y);
}

fn reconstruct_image_base(
    headers: &PeHeaders,
    image_base: *const u8,
    image_size: usize,
    feedback: &mut FeedbackCollector,
    screen: &mut Screen,
    start_x: usize,
    y: &mut usize,
) {
    feedback.info(
        FeedbackCategory::Relocation,
        "Reversing original ImageBase...",
    );
    render_feedback(screen, feedback, start_x, y);

    let actual_load = image_base as u64;
    let reconstruction = unsafe {
        headers.reconstruct_original_image_base(image_base, image_size, actual_load)
    };

    match reconstruction {
        Ok((orig_base, valid_count, total_count)) => {
            feedback.info(
                FeedbackCategory::Relocation,
                format!("Tested {} relocation entries", total_count),
            );

            if valid_count == total_count {
                feedback.success(
                    FeedbackCategory::Relocation,
                    format!("Original ImageBase: 0x{:016X}", orig_base),
                );
                feedback.success(
                    FeedbackCategory::Relocation,
                    format!(
                        "Validated: {}/{} relocations (100%)",
                        valid_count, total_count
                    ),
                );
            } else {
                feedback.warning(
                    FeedbackCategory::Relocation,
                    format!("Best guess ImageBase: 0x{:016X}", orig_base),
                );
                feedback.warning(
                    FeedbackCategory::Relocation,
                    format!("Validated: {}/{} relocations", valid_count, total_count),
                );
            }

            render_feedback(screen, feedback, start_x, y);

            let actual_delta = image_base as u64 - orig_base;
            if actual_delta == 0 {
                feedback.success(
                    FeedbackCategory::Relocation,
                    "Loaded at preferred address!",
                );
            } else {
                feedback.warning(
                    FeedbackCategory::Relocation,
                    format!("Relocation delta: +0x{:016X}", actual_delta),
                );
                feedback.info(
                    FeedbackCategory::Relocation,
                    "Will reverse relocations for bootable image",
                );
            }

            render_feedback(screen, feedback, start_x, y);
        }
        Err(e) => {
            feedback.error(
                FeedbackCategory::Relocation,
                format!("Reconstruction failed: {}", e),
            );
            render_feedback(screen, feedback, start_x, y);
        }
    }
}

fn perform_installation(
    esp: &EspInfo,
    bs: &BootServices,
    image_handle: *mut (),
    feedback: &mut FeedbackCollector,
    screen: &mut Screen,
    start_x: usize,
    y: &mut usize,
) {
    *y += 1;
    screen.put_str_at(start_x, *y, "--- Installation ---", EFI_CYAN, EFI_BLACK);
    *y += 1;

    feedback.info(FeedbackCategory::Storage, "Writing to ESP partition...");
    feedback.debug(
        FeedbackCategory::Storage,
        format!(
            "Target: Disk {} Part {} ({}MB)",
            esp.disk_index, esp.partition_index, esp.size_mb
        ),
    );
    render_feedback(screen, feedback, start_x, y);

    match installer::install_to_esp(bs, esp, image_handle) {
        Ok(()) => {
            feedback.success(
                FeedbackCategory::Storage,
                "Bootloader written successfully!",
            );
            feedback.success(FeedbackCategory::General, "Morpheus is now persistent");
            render_feedback(screen, feedback, start_x, y);
        }
        Err(e) => {
            feedback.error(
                FeedbackCategory::Storage,
                format!("Installation failed: {:?}", e),
            );
            render_feedback(screen, feedback, start_x, y);
        }
    }
}

pub fn render_feedback(
    screen: &mut Screen,
    feedback: &FeedbackCollector,
    start_x: usize,
    y: &mut usize,
) {
    // Only show last few messages to avoid overflow
    let messages = feedback.messages();
    let start_idx = if messages.len() > 3 {
        messages.len() - 3
    } else {
        0
    };

    for msg in &messages[start_idx..] {
        let (color, prefix) = match msg.level {
            FeedbackLevel::Info => (EFI_GREEN, "[INFO]"),
            FeedbackLevel::Success => (EFI_LIGHTGREEN, "[OK]"),
            FeedbackLevel::Warning => (EFI_WHITE, "[WARN]"),
            FeedbackLevel::Error => (EFI_WHITE, "[ERR]"),
            FeedbackLevel::Debug => (EFI_DARKGREEN, "[DBG]"),
        };

        let line = format!("{} {}", prefix, msg.message);
        screen.put_str_at(start_x, *y, &line, color, EFI_BLACK);
        *y += 1;

        // Prevent overflow
        if *y >= screen.height() - 3 {
            break;
        }
    }
}
