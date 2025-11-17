use super::entry::BootEntry;
use crate::tui::renderer::{Screen, EFI_BLACK, EFI_DARKGREEN, EFI_GREEN, EFI_LIGHTGREEN};

const START_Y: usize = 7;
const ENTRY_HEIGHT: usize = 3;

pub struct EntryRenderer;

impl EntryRenderer {
    pub fn render_header(screen: &mut Screen) {
        let title = "=== DISTRO LAUNCHER ===";
        let title_x = (screen.width() - title.len()) / 2;
        screen.put_str_at(title_x, 2, title, EFI_LIGHTGREEN, EFI_BLACK);

        let info = "Use UP/DOWN to select, ENTER to boot, ESC to return";
        let info_x = (screen.width() - info.len()) / 2;
        screen.put_str_at(info_x, 4, info, EFI_DARKGREEN, EFI_BLACK);
    }

    pub fn render_entries(screen: &mut Screen, entries: &[BootEntry], selected: usize) {
        for (i, entry) in entries.iter().enumerate() {
            let y = START_Y + (i * ENTRY_HEIGHT);
            Self::render_entry(screen, entry, y, i == selected);
        }
    }

    fn render_entry(screen: &mut Screen, entry: &BootEntry, y: usize, is_selected: bool) {
        let (fg, bg, marker) = if is_selected {
            (EFI_BLACK, EFI_LIGHTGREEN, "> ")
        } else {
            (EFI_GREEN, EFI_BLACK, "  ")
        };

        screen.put_str_at(10, y, marker, fg, bg);
        screen.put_str_at(12, y, &entry.name, fg, bg);

        screen.put_str_at(10, y + 1, "  Path: ", EFI_DARKGREEN, EFI_BLACK);
        screen.put_str_at(18, y + 1, &entry.kernel_path, EFI_DARKGREEN, EFI_BLACK);
    }

    pub fn render_footer(screen: &mut Screen) {
        let bottom_y = screen.height() - 2;
        screen.put_str_at(
            5,
            bottom_y,
            "Entries auto-discovered from ESP partition",
            EFI_DARKGREEN,
            EFI_BLACK,
        );
    }
}
