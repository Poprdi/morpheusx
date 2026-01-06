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
    use crate::tui::boot_sequence::BootSequence;
    use crate::tui::widgets::progressbar::ProgressBar;
    
    // Clear screen for installation phase - fresh start
    screen.clear();
    
    // Draw installation header
    screen.put_str_at(start_x, 1, "=== PERSISTENCE INSTALLER ===", EFI_LIGHTGREEN, EFI_BLACK);
    screen.put_str_at(start_x, 3, "--- Writing to ESP ---", EFI_CYAN, EFI_BLACK);
    screen.put_str_at(
        start_x, 
        4, 
        &format!("Target: Disk {} Part {} ({}MB)", esp.disk_index, esp.partition_index, esp.size_mb),
        EFI_DARKGREEN, 
        EFI_BLACK
    );
    
    // Progress bar at fixed position
    let progress_y = 6;
    let logs_y = 9;
    let max_logs = (screen.height() - logs_y - 3).min(15); // Leave room for status message
    
    let mut progress_bar = ProgressBar::new(start_x, progress_y, 60, "Installing:");
    progress_bar.render(screen);
    
    // Log installation start - this will be the first log in our display
    morpheus_core::logger::log("Installation: Starting write to ESP...");
    
    // Track the starting log count so we only show installation logs
    let install_start_log_count = morpheus_core::logger::total_log_count();
    let mut last_percent = 0usize;
    
    let result = {
        let mut progress_callback = |bytes: usize, total: usize, msg: &str| {
            if total > 0 {
                let percent = (bytes * 100) / total;
                if percent != last_percent {
                    progress_bar.set_progress(percent);
                    progress_bar.render(screen);
                    last_percent = percent;
                    
                    // Log at every 1% for smooth updates  === PERSISTENCE INSTALLER ===
                    morpheus_core::logger::log(
                        alloc::format!("Writing: {}% ({} KB / {} KB)", 
                            percent,
                            bytes / 1024, 
                            total / 1024
                        ).leak()
                    );
                    
                    // Render only installation logs (skip boot logs)
                    render_install_logs(screen, start_x, logs_y, max_logs, install_start_log_count);
                }
            }
        };
        
        installer::install_to_esp_with_progress(bs, esp, image_handle, Some(&mut progress_callback))
    };

    // Final render
    progress_bar.set_progress(100);
    progress_bar.render(screen);

    match result {
        Ok(()) => {
            morpheus_core::logger::log("Installation: Complete!");
            render_install_logs(screen, start_x, logs_y, max_logs, install_start_log_count);
            
            let status_y = logs_y + max_logs + 1;
            screen.put_str_at(start_x, status_y, "[OK] MorpheusX written successfully!", EFI_LIGHTGREEN, EFI_BLACK);
            *y = status_y + 2;
        }
        Err(e) => {
            morpheus_core::logger::log(alloc::format!("Installation: FAILED - {:?}", e).leak());
            render_install_logs(screen, start_x, logs_y, max_logs, install_start_log_count);
            
            let status_y = logs_y + max_logs + 1;
            screen.put_str_at(start_x, status_y, &format!("[ERR] Installation failed: {:?}", e), EFI_WHITE, EFI_BLACK);
            *y = status_y + 1;
        }
    }
}

/// Render only logs from installation (skip boot logs)
fn render_install_logs(screen: &mut Screen, x: usize, y: usize, max_lines: usize, start_count: usize) {
    let total_count = morpheus_core::logger::total_log_count();
    let install_log_count = total_count.saturating_sub(start_count);
    
    // Get last N installation logs
    let logs_to_show = install_log_count.min(max_lines);
    let skip_count = install_log_count.saturating_sub(logs_to_show);
    
    // Clear the log area first
    for i in 0..max_lines {
        let line_y = y + i;
        if line_y < screen.height() {
            screen.put_str_at(x, line_y, "                                                                                ", EFI_BLACK, EFI_BLACK);
        }
    }
    
    // Render installation logs only
    let mut line_idx = 0;
    for (i, log) in morpheus_core::logger::get_logs_iter().enumerate() {
        // Skip boot logs (before installation started)
        if i < start_count {
            continue;
        }
        
        // Skip older installation logs if we have more than max_lines
        let install_idx = i - start_count;
        if install_idx < skip_count {
            continue;
        }
        
        if line_idx >= max_lines {
            break;
        }
        
        let line_y = y + line_idx;
        if line_y < screen.height() {
            screen.put_str_at(x, line_y, "[  OK  ] ", EFI_GREEN, EFI_BLACK);
            screen.put_str_at(x + 9, line_y, log, EFI_LIGHTGREEN, EFI_BLACK);
        }
        line_idx += 1;
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
