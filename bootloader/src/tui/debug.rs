// Runtime diagnostics and memory tracking

use crate::tui::renderer::{Screen, EFI_BLACK, EFI_GREEN, EFI_LIGHTGREEN, EFI_WHITE};
use core::sync::atomic::{AtomicUsize, Ordering};

// Global allocation tracking
static TOTAL_ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static TOTAL_FREED: AtomicUsize = AtomicUsize::new(0);
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static FREE_COUNT: AtomicUsize = AtomicUsize::new(0);

pub fn track_allocation(size: usize) {
    TOTAL_ALLOCATED.fetch_add(size, Ordering::Relaxed);
    ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
}

pub fn track_free(size: usize) {
    TOTAL_FREED.fetch_add(size, Ordering::Relaxed);
    FREE_COUNT.fetch_add(1, Ordering::Relaxed);
}

pub fn get_memory_stats() -> MemoryStats {
    MemoryStats {
        total_allocated: TOTAL_ALLOCATED.load(Ordering::Relaxed),
        total_freed: TOTAL_FREED.load(Ordering::Relaxed),
        alloc_count: ALLOC_COUNT.load(Ordering::Relaxed),
        free_count: FREE_COUNT.load(Ordering::Relaxed),
    }
}

pub struct MemoryStats {
    pub total_allocated: usize,
    pub total_freed: usize,
    pub alloc_count: usize,
    pub free_count: usize,
}

impl MemoryStats {
    pub fn current_usage(&self) -> usize {
        self.total_allocated.saturating_sub(self.total_freed)
    }

    pub fn leak_count(&self) -> usize {
        self.alloc_count.saturating_sub(self.free_count)
    }
}

// Debug overlay that can be toggled on any screen
pub struct DebugOverlay {
    enabled: bool,
}

impl DebugOverlay {
    pub fn new() -> Self {
        Self { enabled: false }
    }

    pub fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn render(&self, screen: &mut Screen) {
        if !self.enabled {
            return;
        }

        let stats = get_memory_stats();
        let x = screen.width().saturating_sub(35);
        let y = 0;

        // Memory stats box
        screen.put_str_at(x, y, "┌─ MEMORY STATS ─────────────┐", EFI_GREEN, EFI_BLACK);

        let allocated_kb = stats.total_allocated / 1024;
        let freed_kb = stats.total_freed / 1024;
        let current_kb = stats.current_usage() / 1024;

        screen.put_str_at(x, y + 1, "│ Allocated:", EFI_GREEN, EFI_BLACK);
        Self::render_number(screen, x + 21, y + 1, allocated_kb);
        screen.put_str_at(x + 27, y + 1, "KB │", EFI_GREEN, EFI_BLACK);

        screen.put_str_at(x, y + 2, "│ Freed:    ", EFI_GREEN, EFI_BLACK);
        Self::render_number(screen, x + 21, y + 2, freed_kb);
        screen.put_str_at(x + 27, y + 2, "KB │", EFI_GREEN, EFI_BLACK);

        let color = if current_kb > 1024 {
            EFI_WHITE
        } else {
            EFI_LIGHTGREEN
        };
        screen.put_str_at(x, y + 3, "│ Current:  ", color, EFI_BLACK);
        Self::render_number(screen, x + 21, y + 3, current_kb);
        screen.put_str_at(x + 27, y + 3, "KB │", color, EFI_BLACK);

        screen.put_str_at(x, y + 4, "│ Allocs:   ", EFI_GREEN, EFI_BLACK);
        Self::render_number(screen, x + 21, y + 4, stats.alloc_count);
        screen.put_str_at(x + 27, y + 4, "   │", EFI_GREEN, EFI_BLACK);

        screen.put_str_at(x, y + 5, "│ Frees:    ", EFI_GREEN, EFI_BLACK);
        Self::render_number(screen, x + 21, y + 5, stats.free_count);
        screen.put_str_at(x + 27, y + 5, "   │", EFI_GREEN, EFI_BLACK);

        let leak_color = if stats.leak_count() > 10 {
            EFI_WHITE
        } else {
            EFI_LIGHTGREEN
        };
        screen.put_str_at(x, y + 6, "│ Leaks:    ", leak_color, EFI_BLACK);
        Self::render_number(screen, x + 21, y + 6, stats.leak_count());
        screen.put_str_at(x + 27, y + 6, "   │", leak_color, EFI_BLACK);

        screen.put_str_at(
            x,
            y + 7,
            "└────────────────────────────┘",
            EFI_GREEN,
            EFI_BLACK,
        );
        screen.put_str_at(
            x,
            y + 8,
            " [D] Toggle Debug Overlay    ",
            EFI_GREEN,
            EFI_BLACK,
        );
    }

    fn render_number(screen: &mut Screen, x: usize, y: usize, num: usize) {
        let mut buf = [0u8; 10];
        let len = format_number(num, &mut buf);

        // Right align within 6 char field
        let padding = 6usize.saturating_sub(len);
        for i in 0..padding {
            screen.put_char_at(x + i, y, ' ', EFI_GREEN, EFI_BLACK);
        }

        let text = core::str::from_utf8(&buf[..len]).unwrap_or("?");
        screen.put_str_at(x + padding, y, text, EFI_LIGHTGREEN, EFI_BLACK);
    }
}

fn format_number(num: usize, buf: &mut [u8]) -> usize {
    if num == 0 {
        buf[0] = b'0';
        return 1;
    }

    let mut n = num;
    let mut digits = [0u8; 20];
    let mut count = 0;

    while n > 0 {
        digits[count] = b'0' + (n % 10) as u8;
        n /= 10;
        count += 1;
    }

    for i in 0..count {
        if i < buf.len() {
            buf[i] = digits[count - 1 - i];
        }
    }

    count
}
