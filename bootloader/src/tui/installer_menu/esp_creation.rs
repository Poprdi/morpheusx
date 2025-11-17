// ESP creation operations and help screens

use crate::installer::{self, EspInfo, InstallError};
use crate::tui::input::Keyboard;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN};
use crate::BootServices;
use alloc::string::ToString;

pub fn create_new_esp(
    screen: &mut Screen,
    keyboard: &mut Keyboard,
    bs: &BootServices,
) -> Option<EspInfo> {
    screen.clear();
    let start_x = screen.center_x(80);

    render_creation_prompt(screen, start_x);

    let key = keyboard.wait_for_key();
    if key.unicode_char != b'y' as u16 && key.unicode_char != b'Y' as u16 {
        return None;
    }

    screen.clear();
    screen.put_str_at(
        start_x,
        3,
        "=== CREATING ESP ===",
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );
    screen.put_str_at(start_x, 5, "Scanning disk...", EFI_GREEN, EFI_BLACK);

    let result = installer::create_esp_and_install(bs, 0);
    render_creation_result(screen, keyboard, start_x, result)
}

fn render_creation_prompt(screen: &mut Screen, start_x: usize) {
    screen.put_str_at(
        start_x,
        3,
        "=== CREATE NEW ESP ===",
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );
    screen.put_str_at(start_x, 5, "This will:", EFI_GREEN, EFI_BLACK);
    screen.put_str_at(
        start_x,
        6,
        "  - Find free space on Disk 0",
        EFI_DARKGREEN,
        EFI_BLACK,
    );
    screen.put_str_at(
        start_x,
        7,
        "  - Create 512MB ESP partition",
        EFI_DARKGREEN,
        EFI_BLACK,
    );
    screen.put_str_at(start_x, 8, "  - Format as FAT32", EFI_DARKGREEN, EFI_BLACK);
    screen.put_str_at(
        start_x,
        9,
        "  - Verify filesystem integrity",
        EFI_DARKGREEN,
        EFI_BLACK,
    );
    screen.put_str_at(
        start_x,
        11,
        "[Y] Continue    [N] Cancel",
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );
}

fn render_creation_result(
    screen: &mut Screen,
    keyboard: &mut Keyboard,
    start_x: usize,
    result: Result<EspInfo, InstallError>,
) -> Option<EspInfo> {
    match result {
        Ok(esp_info) => {
            render_success(screen, start_x, &esp_info);
            screen.put_str_at(
                start_x,
                17,
                "Press any key to continue...",
                EFI_DARKGREEN,
                EFI_BLACK,
            );
            keyboard.wait_for_key();
            Some(esp_info)
        }
        Err(err) => {
            render_error(screen, start_x, err);
            screen.put_str_at(
                start_x,
                17,
                "Press any key to continue...",
                EFI_DARKGREEN,
                EFI_BLACK,
            );
            keyboard.wait_for_key();
            None
        }
    }
}

fn render_success(screen: &mut Screen, start_x: usize, esp_info: &EspInfo) {
    screen.put_str_at(
        start_x,
        7,
        "SUCCESS: ESP created and formatted",
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );

    let disk_str = esp_info.disk_index.to_string();
    screen.put_str_at(start_x, 9, "  Disk:", EFI_GREEN, EFI_BLACK);
    screen.put_str_at(start_x + 15, 9, &disk_str, EFI_LIGHTGREEN, EFI_BLACK);

    let part_str = esp_info.partition_index.to_string();
    screen.put_str_at(start_x, 10, "  Partition:", EFI_GREEN, EFI_BLACK);
    screen.put_str_at(start_x + 15, 10, &part_str, EFI_LIGHTGREEN, EFI_BLACK);

    let size_str = esp_info.size_mb.to_string();
    screen.put_str_at(start_x, 11, "  Size:", EFI_GREEN, EFI_BLACK);
    screen.put_str_at(start_x + 15, 11, &size_str, EFI_LIGHTGREEN, EFI_BLACK);
    screen.put_str_at(
        start_x + 15 + size_str.len(),
        11,
        " MB",
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );

    screen.put_str_at(
        start_x,
        13,
        "ESP ready for installation",
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );
}

fn render_error(screen: &mut Screen, start_x: usize, err: InstallError) {
    let (msg1, msg2) = match err {
        InstallError::NoFreeSpc => (
            "ERROR: No free space (need 512MB)",
            "Free up space using Storage Manager",
        ),
        InstallError::FormatFailed => (
            "ERROR: Partition created but format failed",
            "Try formatting manually in Storage Manager",
        ),
        InstallError::IoError => (
            "ERROR: Failed to create partition",
            "Disk may be full or GPT corrupted",
        ),
        InstallError::ProtocolError => ("ERROR: Failed to access disk", ""),
        _ => ("ERROR: Unknown error occurred", ""),
    };

    screen.put_str_at(start_x, 7, msg1, EFI_LIGHTGREEN, EFI_BLACK);
    if !msg2.is_empty() {
        screen.put_str_at(start_x, 9, msg2, EFI_GREEN, EFI_BLACK);
    }
}

pub fn show_create_esp_help(screen: &mut Screen, keyboard: &mut Keyboard) {
    screen.clear();
    let start_x = screen.center_x(80);

    screen.put_str_at(
        start_x,
        3,
        "=== CREATE ESP PARTITION ===",
        EFI_LIGHTGREEN,
        EFI_BLACK,
    );
    screen.put_str_at(
        start_x,
        5,
        "To create an EFI System Partition:",
        EFI_GREEN,
        EFI_BLACK,
    );
    screen.put_str_at(
        start_x,
        7,
        "1. Go to Storage Manager (from main menu)",
        EFI_DARKGREEN,
        EFI_BLACK,
    );
    screen.put_str_at(
        start_x,
        8,
        "2. Select your target disk",
        EFI_DARKGREEN,
        EFI_BLACK,
    );
    screen.put_str_at(
        start_x,
        9,
        "3. Create new partition (min 100MB, recommend 512MB)",
        EFI_DARKGREEN,
        EFI_BLACK,
    );
    screen.put_str_at(
        start_x,
        10,
        "4. Set partition type to 'EFI System'",
        EFI_DARKGREEN,
        EFI_BLACK,
    );
    screen.put_str_at(start_x, 11, "5. Format as FAT32", EFI_DARKGREEN, EFI_BLACK);
    screen.put_str_at(
        start_x,
        12,
        "6. Return here and rescan [R]",
        EFI_DARKGREEN,
        EFI_BLACK,
    );

    screen.put_str_at(
        start_x,
        15,
        "Press any key to return...",
        EFI_DARKGREEN,
        EFI_BLACK,
    );
    keyboard.wait_for_key();
}
