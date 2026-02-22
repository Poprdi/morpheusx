use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use morpheus_hwinit::serial::puts;
use morpheus_ui::app::{App, AppEntry, AppRegistry, AppResult};
use morpheus_ui::canvas::Canvas;
use morpheus_ui::color::Color;
use morpheus_ui::draw::glyph::draw_string;
use morpheus_ui::draw::shapes::{hline, rect_fill, rect_outline, rounded_rect_fill, vline};
use morpheus_ui::event::{Event, Key, KeyEvent};
use morpheus_ui::font;
use morpheus_ui::theme::Theme;

const TAB_COUNT: usize = 3;
const TAB_NAMES: [&str; TAB_COUNT] = ["Overview", "Memory Map", "Heap"];
const ANIM_SPEED: u32 = 3;
const BAR_HEIGHT: u32 = 14;
const SECTION_PAD: u32 = 8;

pub fn register(registry: &mut AppRegistry) {
    registry.register(AppEntry {
        name: "storage",
        title: "Storage & Memory Manager",
        default_size: (800, 500),
        create: || Box::new(StorageManager::new()),
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overview,
    MemoryMap,
    Heap,
}

impl Tab {
    fn index(self) -> usize {
        match self {
            Tab::Overview => 0,
            Tab::MemoryMap => 1,
            Tab::Heap => 2,
        }
    }

    fn from_index(i: usize) -> Self {
        match i {
            0 => Tab::Overview,
            1 => Tab::MemoryMap,
            _ => Tab::Heap,
        }
    }
}

struct MemStats {
    total_bytes: u64,
    free_bytes: u64,
    allocated_bytes: u64,
    bump_remaining: u64,
    region_count: usize,
}

struct HeapStats {
    total: usize,
    used: usize,
    free: usize,
}

struct MemRegion {
    index: usize,
    type_name: &'static str,
    start: u64,
    pages: u64,
    size_kb: u64,
    is_free: bool,
    is_allocated: bool,
}

pub struct StorageManager {
    active_tab: Tab,
    mem_stats: MemStats,
    heap_stats: HeapStats,
    regions: Vec<MemRegion>,
    map_scroll: usize,
    map_selected: usize,
    anim_tick: u32,
    anim_bars: [u32; 4],
    needs_refresh: bool,
}

impl StorageManager {
    pub fn new() -> Self {
        puts("[STORAGE] new() start\n");
        let mut s = Self {
            active_tab: Tab::Overview,
            mem_stats: MemStats {
                total_bytes: 0,
                free_bytes: 0,
                allocated_bytes: 0,
                bump_remaining: 0,
                region_count: 0,
            },
            heap_stats: HeapStats { total: 0, used: 0, free: 0 },
            regions: Vec::new(),
            map_scroll: 0,
            map_selected: 0,
            anim_tick: 0,
            anim_bars: [0; 4],
            needs_refresh: true,
        };
        puts("[STORAGE] new() struct built, calling refresh_data\n");
        s.refresh_data();
        puts("[STORAGE] new() done\n");
        s
    }

    fn refresh_data(&mut self) {
        puts("[STORAGE] refresh_data: checking registry\n");
        if morpheus_hwinit::is_registry_initialized() {
            puts("[STORAGE] refresh_data: registry initialized, reading stats\n");
            let reg = unsafe { morpheus_hwinit::global_registry() };
            self.mem_stats.total_bytes = reg.total_memory();
            self.mem_stats.free_bytes = reg.free_memory();
            self.mem_stats.allocated_bytes = reg.allocated_memory();
            self.mem_stats.bump_remaining = reg.bump_remaining();

            let (_key, count) = reg.get_memory_map();
            self.mem_stats.region_count = count;
            puts("[STORAGE] refresh_data: building region list\n");

            self.regions.clear();
            for i in 0..count {
                if let Some(desc) = reg.get_descriptor(i) {
                    self.regions.push(MemRegion {
                        index: i,
                        type_name: mem_type_name(desc.mem_type),
                        start: desc.physical_start,
                        pages: desc.number_of_pages,
                        size_kb: desc.size() / 1024,
                        is_free: desc.mem_type.is_free(),
                        is_allocated: matches!(desc.mem_type,
                            morpheus_hwinit::MemoryType::Allocated |
                            morpheus_hwinit::MemoryType::AllocatedDma |
                            morpheus_hwinit::MemoryType::AllocatedStack |
                            morpheus_hwinit::MemoryType::AllocatedPageTable |
                            morpheus_hwinit::MemoryType::AllocatedHeap
                        ),
                    });
                }
            }
            puts("[STORAGE] refresh_data: region list done\n");
        } else {
            puts("[STORAGE] refresh_data: registry NOT initialized\n");
        }

        puts("[STORAGE] refresh_data: reading heap stats\n");
        if let Some((total, used, free)) = morpheus_hwinit::heap_stats() {
            self.heap_stats = HeapStats { total, used, free };
            puts("[STORAGE] refresh_data: heap stats ok\n");
        } else {
            puts("[STORAGE] refresh_data: heap stats unavailable\n");
        }

        self.needs_refresh = false;
        puts("[STORAGE] refresh_data: done\n");
    }

    fn tick_animations(&mut self) {
        self.anim_tick = self.anim_tick.wrapping_add(1);

        let targets = [
            self.bar_fraction(self.mem_stats.allocated_bytes, self.mem_stats.total_bytes),
            self.bar_fraction(self.mem_stats.free_bytes, self.mem_stats.total_bytes),
            self.bar_fraction(self.heap_stats.used as u64, self.heap_stats.total as u64),
            self.bar_fraction(self.mem_stats.bump_remaining, self.mem_stats.total_bytes),
        ];

        for (bar, &target) in self.anim_bars.iter_mut().zip(targets.iter()) {
            if *bar < target {
                *bar = (*bar + ANIM_SPEED).min(target);
            } else if *bar > target {
                *bar = bar.saturating_sub(ANIM_SPEED).max(target);
            }
        }
    }

    fn bar_fraction(&self, value: u64, total: u64) -> u32 {
        if total == 0 { return 0; }
        ((value * 100) / total).min(100) as u32
    }

    fn render_tab_bar(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        let w = canvas.width();
        let tab_w = w / TAB_COUNT as u32;

        for (i, name) in TAB_NAMES.iter().enumerate() {
            let x = i as u32 * tab_w;
            let is_active = self.active_tab.index() == i;

            let (fg, bg) = if is_active {
                (theme.title_fg, theme.accent)
            } else {
                (theme.fg, theme.bg)
            };

            rect_fill(canvas, x, 0, tab_w, font::FONT_HEIGHT + 4, bg);

            let tx = x + (tab_w.saturating_sub(name.len() as u32 * font::FONT_WIDTH)) / 2;
            draw_string(canvas, tx, 2, name, fg, bg, &font::FONT_DATA);
        }

        hline(canvas, 0, font::FONT_HEIGHT + 4, w, theme.border);
    }

    fn render_overview(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        let w = canvas.width();
        let content_y = font::FONT_HEIGHT + 6;
        let usable_w = w.saturating_sub(SECTION_PAD * 2);
        let col_w = usable_w / 2;
        let left_x = SECTION_PAD;
        let right_x = SECTION_PAD + col_w + SECTION_PAD;

        // Left column: Physical Memory
        let mut y = content_y + SECTION_PAD;

        y = self.render_section_header(canvas, left_x, y, col_w, "Physical Memory", theme);
        y += 4;

        y = self.render_stat_line(canvas, left_x, y, col_w, "Total:", &format_size(self.mem_stats.total_bytes), theme.fg, theme);
        y = self.render_stat_line(canvas, left_x, y, col_w, "Free:", &format_size(self.mem_stats.free_bytes), COLOR_FREE, theme);
        y = self.render_stat_line(canvas, left_x, y, col_w, "Allocated:", &format_size(self.mem_stats.allocated_bytes), COLOR_ALLOC, theme);
        y = self.render_stat_line(canvas, left_x, y, col_w, "Bump Avail:", &format_size(self.mem_stats.bump_remaining), theme.fg, theme);
        y += 6;

        // Animated memory usage bar
        y = self.render_labeled_bar(canvas, left_x + 4, y, col_w.saturating_sub(8),
            "Memory Used", self.anim_bars[0], COLOR_ALLOC, theme);
        y += 4;
        y = self.render_labeled_bar(canvas, left_x + 4, y, col_w.saturating_sub(8),
            "Memory Free", self.anim_bars[1], COLOR_FREE, theme);
        y += SECTION_PAD;

        // Memory composition chart (stacked bar)
        y = self.render_section_header(canvas, left_x, y, col_w, "Composition", theme);
        y += 4;
        self.render_stacked_bar(canvas, left_x + 4, y, col_w.saturating_sub(8), BAR_HEIGHT + 4, theme);

        // Right column: System Info
        let mut y = content_y + SECTION_PAD;

        y = self.render_section_header(canvas, right_x, y, col_w, "System Info", theme);
        y += 4;

        y = self.render_stat_line(canvas, right_x, y, col_w, "Regions:", &format_u64(self.mem_stats.region_count as u64), theme.fg, theme);
        y = self.render_stat_line(canvas, right_x, y, col_w, "Pages:", &format_u64(self.mem_stats.total_bytes / 4096), theme.fg, theme);
        y += SECTION_PAD;

        // Heap section
        y = self.render_section_header(canvas, right_x, y, col_w, "Heap Allocator", theme);
        y += 4;

        y = self.render_stat_line(canvas, right_x, y, col_w, "Total:", &format_size(self.heap_stats.total as u64), theme.fg, theme);
        y = self.render_stat_line(canvas, right_x, y, col_w, "Used:", &format_size(self.heap_stats.used as u64), COLOR_ALLOC, theme);
        y = self.render_stat_line(canvas, right_x, y, col_w, "Free:", &format_size(self.heap_stats.free as u64), COLOR_FREE, theme);
        y += 6;

        y = self.render_labeled_bar(canvas, right_x + 4, y, col_w.saturating_sub(8),
            "Heap Usage", self.anim_bars[2], COLOR_HEAP, theme);
        y += SECTION_PAD;

        // Activity indicator
        y = self.render_section_header(canvas, right_x, y, col_w, "Activity", theme);
        y += 4;
        self.render_spinner(canvas, right_x + 4, y, theme);
        draw_string(canvas, right_x + 24, y,
            if self.needs_refresh { "Refreshing..." } else { "Live" },
            theme.accent, theme.bg, &font::FONT_DATA);
    }

    fn render_memory_map(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        let w = canvas.width();
        let content_y = font::FONT_HEIGHT + 6;
        let usable_w = w.saturating_sub(SECTION_PAD * 2);

        let mut y = content_y + 4;

        // Header
        let hdr = "  #   Type                   Address          Pages     Size";
        rect_fill(canvas, SECTION_PAD, y, usable_w, font::FONT_HEIGHT, theme.title_bg);
        draw_string(canvas, SECTION_PAD + 2, y, hdr, theme.title_fg, theme.title_bg, &font::FONT_DATA);
        y += font::FONT_HEIGHT;

        hline(canvas, SECTION_PAD, y, usable_w, theme.border);
        y += 1;

        let row_h = font::FONT_HEIGHT;
        let vis_rows = (canvas.height().saturating_sub(y + font::FONT_HEIGHT + 8)) / row_h;

        for i in 0..vis_rows as usize {
            let idx = self.map_scroll + i;
            if idx >= self.regions.len() {
                break;
            }

            let region = &self.regions[idx];
            let is_selected = idx == self.map_selected;

            let (fg, bg) = if is_selected {
                (theme.selection_fg, theme.selection_bg)
            } else if region.is_free {
                (COLOR_FREE, theme.bg)
            } else if region.is_allocated {
                (COLOR_ALLOC, theme.bg)
            } else {
                (theme.fg, theme.bg)
            };

            rect_fill(canvas, SECTION_PAD, y, usable_w, row_h, bg);

            // Color indicator bar
            let indicator_color = if region.is_free {
                COLOR_FREE
            } else if region.is_allocated {
                COLOR_ALLOC
            } else {
                COLOR_RESERVED
            };
            rect_fill(canvas, SECTION_PAD, y, 3, row_h, indicator_color);

            let line = format_region_line(region);
            draw_string(canvas, SECTION_PAD + 4, y, &line, fg, bg, &font::FONT_DATA);

            y += row_h;
        }

        // Footer / scrollbar hint
        let footer_y = canvas.height().saturating_sub(font::FONT_HEIGHT + 4);
        hline(canvas, SECTION_PAD, footer_y, usable_w, theme.border);
        let status = format!(
            " {}/{} regions  |  Up/Down: navigate  PgUp/PgDn: scroll  R: refresh",
            self.map_selected + 1, self.regions.len()
        );
        draw_string(canvas, SECTION_PAD + 2, footer_y + 2, &status, theme.fg, theme.bg, &font::FONT_DATA);

        // Scrollbar
        if self.regions.len() > vis_rows as usize {
            let sb_x = w.saturating_sub(SECTION_PAD + 6);
            let sb_top = content_y + font::FONT_HEIGHT + 5;
            let sb_h = footer_y.saturating_sub(sb_top + 2);

            rect_fill(canvas, sb_x, sb_top, 4, sb_h, theme.scrollbar_bg);

            let thumb_h = ((vis_rows as u64 * sb_h as u64) / self.regions.len().max(1) as u64)
                .max(8) as u32;
            let thumb_y = sb_top + ((self.map_scroll as u64 * sb_h.saturating_sub(thumb_h) as u64)
                / self.regions.len().saturating_sub(vis_rows as usize).max(1) as u64) as u32;

            rounded_rect_fill(canvas, sb_x, thumb_y, 4, thumb_h, 2, theme.scrollbar_fg);
        }
    }

    fn render_heap_tab(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        let w = canvas.width();
        let content_y = font::FONT_HEIGHT + 6;
        let usable_w = w.saturating_sub(SECTION_PAD * 2);

        let mut y = content_y + SECTION_PAD;

        // Big heap stats
        y = self.render_section_header(canvas, SECTION_PAD, y, usable_w, "Heap Allocator Details", theme);
        y += SECTION_PAD;

        // Big numbers display
        let total_str = format_size(self.heap_stats.total as u64);
        let used_str = format_size(self.heap_stats.used as u64);
        let free_str = format_size(self.heap_stats.free as u64);

        let big_y = y;
        let third_w = usable_w / 3;

        self.render_big_stat(canvas, SECTION_PAD, big_y, third_w, "TOTAL", &total_str, theme.fg, theme);
        self.render_big_stat(canvas, SECTION_PAD + third_w, big_y, third_w, "USED", &used_str, COLOR_ALLOC, theme);
        self.render_big_stat(canvas, SECTION_PAD + third_w * 2, big_y, third_w, "FREE", &free_str, COLOR_FREE, theme);

        y = big_y + font::FONT_HEIGHT * 3 + SECTION_PAD;

        // Full-width usage bar with animation
        y = self.render_section_header(canvas, SECTION_PAD, y, usable_w, "Usage", theme);
        y += 4;
        let bar_w = usable_w.saturating_sub(8);
        self.render_gradient_bar(canvas, SECTION_PAD + 4, y, bar_w, BAR_HEIGHT + 8, self.anim_bars[2], theme);

        y += BAR_HEIGHT + 8 + SECTION_PAD;

        // Fragmentation visualization
        y = self.render_section_header(canvas, SECTION_PAD, y, usable_w, "Heap Visualization", theme);
        y += 4;
        self.render_heap_blocks(canvas, SECTION_PAD + 4, y, bar_w, font::FONT_HEIGHT * 3, theme);

        y += font::FONT_HEIGHT * 3 + SECTION_PAD;

        // Utilization percentage
        let pct = if self.heap_stats.total > 0 {
            (self.heap_stats.used * 100) / self.heap_stats.total
        } else {
            0
        };

        y = self.render_section_header(canvas, SECTION_PAD, y, usable_w, "Efficiency", theme);
        y += 4;
        let eff_text = format!("  Utilization: {}%  |  Fragmentation: estimated", pct);
        draw_string(canvas, SECTION_PAD + 4, y, &eff_text, theme.fg, theme.bg, &font::FONT_DATA);
    }

    // ── Drawing helpers ─────────────────────────────────────────────────

    fn render_section_header(&self, canvas: &mut dyn Canvas, x: u32, y: u32, w: u32, title: &str, theme: &Theme) -> u32 {
        rect_fill(canvas, x, y, w, font::FONT_HEIGHT + 2, theme.title_bg);
        draw_string(canvas, x + 4, y + 1, title, theme.title_fg, theme.title_bg, &font::FONT_DATA);
        hline(canvas, x, y + font::FONT_HEIGHT + 2, w, theme.accent);
        y + font::FONT_HEIGHT + 3
    }

    fn render_stat_line(&self, canvas: &mut dyn Canvas, x: u32, y: u32, w: u32, label: &str, value: &str, value_color: Color, theme: &Theme) -> u32 {
        draw_string(canvas, x + 4, y, label, theme.fg, theme.bg, &font::FONT_DATA);
        let vx = x + w.saturating_sub((value.len() as u32 + 1) * font::FONT_WIDTH);
        draw_string(canvas, vx, y, value, value_color, theme.bg, &font::FONT_DATA);
        y + font::FONT_HEIGHT
    }

    fn render_labeled_bar(&self, canvas: &mut dyn Canvas, x: u32, y: u32, w: u32, label: &str, pct: u32, fill_color: Color, theme: &Theme) -> u32 {
        draw_string(canvas, x, y, label, theme.fg, theme.bg, &font::FONT_DATA);
        let pct_str = format_pct(pct);
        let px = x + w.saturating_sub((pct_str.len() as u32) * font::FONT_WIDTH);
        draw_string(canvas, px, y, &pct_str, fill_color, theme.bg, &font::FONT_DATA);
        let bar_y = y + font::FONT_HEIGHT;

        rect_outline(canvas, x, bar_y, w, BAR_HEIGHT, 1, theme.border);
        rect_fill(canvas, x + 1, bar_y + 1, w.saturating_sub(2), BAR_HEIGHT.saturating_sub(2), theme.input_bg);

        let fill_w = ((w.saturating_sub(2)) as u64 * pct as u64 / 100) as u32;
        if fill_w > 0 {
            rect_fill(canvas, x + 1, bar_y + 1, fill_w, BAR_HEIGHT.saturating_sub(2), fill_color);
        }

        bar_y + BAR_HEIGHT + 2
    }

    fn render_gradient_bar(&self, canvas: &mut dyn Canvas, x: u32, y: u32, w: u32, h: u32, pct: u32, theme: &Theme) {
        rect_outline(canvas, x, y, w, h, 1, theme.border);
        rect_fill(canvas, x + 1, y + 1, w.saturating_sub(2), h.saturating_sub(2), theme.input_bg);

        let fill_w = ((w.saturating_sub(2)) as u64 * pct as u64 / 100) as u32;
        if fill_w == 0 { return; }

        for col in 0..fill_w {
            let ratio = (col * 255 / fill_w.max(1)) as u8;
            let color = Color::rgb(
                ratio,
                (255u16).saturating_sub(ratio as u16) as u8,
                0,
            );
            vline(canvas, x + 1 + col, y + 1, h.saturating_sub(2), color);
        }

        // Percentage text centered on bar
        let text = format_pct(pct);
        let tw = text.len() as u32 * font::FONT_WIDTH;
        let tx = x + 1 + fill_w.saturating_sub(tw) / 2;
        let ty = y + (h.saturating_sub(font::FONT_HEIGHT)) / 2;
        draw_string(canvas, tx, ty, &text, Color::WHITE, Color::rgba(0, 0, 0, 0), &font::FONT_DATA);
    }

    fn render_stacked_bar(&self, canvas: &mut dyn Canvas, x: u32, y: u32, w: u32, h: u32, theme: &Theme) {
        rect_outline(canvas, x, y, w, h, 1, theme.border);
        let inner_w = w.saturating_sub(2);
        let inner_h = h.saturating_sub(2);
        rect_fill(canvas, x + 1, y + 1, inner_w, inner_h, theme.input_bg);

        let total = self.mem_stats.total_bytes;
        if total == 0 { return; }

        let alloc_w = ((inner_w as u64 * self.mem_stats.allocated_bytes) / total) as u32;
        let free_w = ((inner_w as u64 * self.mem_stats.free_bytes) / total) as u32;
        let reserved_w = inner_w.saturating_sub(alloc_w + free_w);

        let mut cx = x + 1;
        if alloc_w > 0 {
            rect_fill(canvas, cx, y + 1, alloc_w, inner_h, COLOR_ALLOC);
            cx += alloc_w;
        }
        if reserved_w > 0 {
            rect_fill(canvas, cx, y + 1, reserved_w, inner_h, COLOR_RESERVED);
            cx += reserved_w;
        }
        if free_w > 0 {
            rect_fill(canvas, cx, y + 1, free_w, inner_h, COLOR_FREE);
        }

        // Legend below
        let ly = y + h + 2;
        self.render_legend_item(canvas, x, ly, "Allocated", COLOR_ALLOC, theme);
        self.render_legend_item(canvas, x + 100, ly, "Reserved", COLOR_RESERVED, theme);
        self.render_legend_item(canvas, x + 200, ly, "Free", COLOR_FREE, theme);
    }

    fn render_legend_item(&self, canvas: &mut dyn Canvas, x: u32, y: u32, label: &str, color: Color, theme: &Theme) {
        rect_fill(canvas, x, y + 3, 8, 8, color);
        rect_outline(canvas, x, y + 3, 8, 8, 1, theme.border);
        draw_string(canvas, x + 12, y, label, theme.fg, theme.bg, &font::FONT_DATA);
    }

    fn render_big_stat(&self, canvas: &mut dyn Canvas, x: u32, y: u32, w: u32, label: &str, value: &str, color: Color, theme: &Theme) {
        rect_outline(canvas, x + 2, y, w.saturating_sub(4), font::FONT_HEIGHT * 3, 1, theme.border);
        rect_fill(canvas, x + 3, y + 1, w.saturating_sub(6), font::FONT_HEIGHT * 3 - 2, theme.input_bg);

        let lx = x + (w.saturating_sub(label.len() as u32 * font::FONT_WIDTH)) / 2;
        draw_string(canvas, lx, y + 4, label, theme.fg, theme.input_bg, &font::FONT_DATA);

        let vx = x + (w.saturating_sub(value.len() as u32 * font::FONT_WIDTH)) / 2;
        draw_string(canvas, vx, y + font::FONT_HEIGHT + 8, value, color, theme.input_bg, &font::FONT_DATA);
    }

    fn render_heap_blocks(&self, canvas: &mut dyn Canvas, x: u32, y: u32, w: u32, h: u32, theme: &Theme) {
        rect_outline(canvas, x, y, w, h, 1, theme.border);
        let inner_w = w.saturating_sub(2);
        let inner_h = h.saturating_sub(2);
        rect_fill(canvas, x + 1, y + 1, inner_w, inner_h, COLOR_FREE);

        if self.heap_stats.total == 0 { return; }

        let used_w = ((inner_w as u64 * self.heap_stats.used as u64) / self.heap_stats.total as u64) as u32;

        // Simulated fragmentation pattern using tick-based determinism
        let block_w = 6u32;
        let mut cx = 0u32;
        let mut block_i = 0u32;
        while cx + block_w <= inner_w {
            let in_used = cx < used_w;
            // Create visual fragmentation pattern
            let is_gap = in_used && (block_i % 7 == 3 || block_i % 11 == 5);
            let color = if in_used && !is_gap {
                COLOR_HEAP
            } else {
                COLOR_FREE
            };
            rect_fill(canvas, x + 1 + cx, y + 1, block_w.saturating_sub(1), inner_h, color);
            cx += block_w;
            block_i += 1;
        }
    }

    fn render_spinner(&self, canvas: &mut dyn Canvas, x: u32, y: u32, theme: &Theme) {
        let frames = ["|", "/", "-", "\\"];
        let frame = (self.anim_tick / 4) as usize % frames.len();
        draw_string(canvas, x, y, frames[frame], theme.accent, theme.bg, &font::FONT_DATA);
    }

    fn visible_map_rows(&self, canvas_h: u32) -> usize {
        let header_h = font::FONT_HEIGHT + 6 + 4 + font::FONT_HEIGHT + 1;
        let footer_h = font::FONT_HEIGHT + 8;
        let avail = canvas_h.saturating_sub(header_h + footer_h);
        (avail / font::FONT_HEIGHT) as usize
    }

    fn ensure_map_visible(&mut self, vis: usize) {
        if vis == 0 { return; }
        if self.map_selected < self.map_scroll {
            self.map_scroll = self.map_selected;
        } else if self.map_selected >= self.map_scroll + vis {
            self.map_scroll = self.map_selected.saturating_sub(vis) + 1;
        }
    }
}

impl App for StorageManager {
    fn title(&self) -> &str {
        "Storage & Memory Manager"
    }

    fn default_size(&self) -> (u32, u32) {
        (800, 500)
    }

    fn init(&mut self, _canvas: &mut dyn Canvas, _theme: &Theme) {
        puts("[STORAGE] init() start\n");
        self.refresh_data();
        puts("[STORAGE] init() done\n");
    }

    fn render(&self, canvas: &mut dyn Canvas, theme: &Theme) {
        puts("[STORAGE] render() start\n");
        canvas.clear(theme.bg);
        puts("[STORAGE] render() clear done, drawing tab bar\n");
        self.render_tab_bar(canvas, theme);
        puts("[STORAGE] render() tab bar done\n");

        match self.active_tab {
            Tab::Overview => {
                puts("[STORAGE] render() overview start\n");
                self.render_overview(canvas, theme);
                puts("[STORAGE] render() overview done\n");
            }
            Tab::MemoryMap => {
                puts("[STORAGE] render() memory map start\n");
                self.render_memory_map(canvas, theme);
                puts("[STORAGE] render() memory map done\n");
            }
            Tab::Heap => {
                puts("[STORAGE] render() heap tab start\n");
                self.render_heap_tab(canvas, theme);
                puts("[STORAGE] render() heap tab done\n");
            }
        }

        // Bottom status bar
        let h = canvas.height();
        let w = canvas.width();
        let status_y = h.saturating_sub(font::FONT_HEIGHT + 2);
        hline(canvas, 0, status_y, w, theme.border);
        let hint = "Tab/Shift+Tab: switch  |  R: refresh  |  Esc: close";
        draw_string(canvas, 4, status_y + 1, hint, theme.fg, theme.bg, &font::FONT_DATA);
        puts("[STORAGE] render() complete\n");
    }

    fn handle_event(&mut self, event: &Event) -> AppResult {
        let Event::KeyPress(KeyEvent { key, modifiers }) = event else {
            return AppResult::Continue;
        };

        match key {
            Key::Escape => return AppResult::Close,

            Key::Tab => {
                let idx = self.active_tab.index();
                let next = if modifiers.shift {
                    if idx == 0 { TAB_COUNT - 1 } else { idx - 1 }
                } else {
                    (idx + 1) % TAB_COUNT
                };
                self.active_tab = Tab::from_index(next);
                return AppResult::Redraw;
            }

            Key::Char('r') | Key::Char('R') => {
                self.needs_refresh = true;
                self.refresh_data();
                return AppResult::Redraw;
            }

            Key::Char('1') => { self.active_tab = Tab::Overview; return AppResult::Redraw; }
            Key::Char('2') => { self.active_tab = Tab::MemoryMap; return AppResult::Redraw; }
            Key::Char('3') => { self.active_tab = Tab::Heap; return AppResult::Redraw; }

            _ => {}
        }

        if self.active_tab == Tab::MemoryMap {
            match key {
                Key::Up => {
                    if self.map_selected > 0 {
                        self.map_selected -= 1;
                        self.ensure_map_visible(20);
                        return AppResult::Redraw;
                    }
                }
                Key::Down => {
                    if self.map_selected + 1 < self.regions.len() {
                        self.map_selected += 1;
                        self.ensure_map_visible(20);
                        return AppResult::Redraw;
                    }
                }
                Key::PageUp => {
                    self.map_selected = self.map_selected.saturating_sub(20);
                    self.map_scroll = self.map_scroll.saturating_sub(20);
                    return AppResult::Redraw;
                }
                Key::PageDown => {
                    self.map_selected = (self.map_selected + 20).min(self.regions.len().saturating_sub(1));
                    self.ensure_map_visible(20);
                    return AppResult::Redraw;
                }
                Key::Home => {
                    self.map_selected = 0;
                    self.map_scroll = 0;
                    return AppResult::Redraw;
                }
                Key::End => {
                    self.map_selected = self.regions.len().saturating_sub(1);
                    self.ensure_map_visible(20);
                    return AppResult::Redraw;
                }
                _ => {}
            }
        }

        self.tick_animations();
        AppResult::Redraw
    }
}

// ── Color constants ─────────────────────────────────────────────────────

const COLOR_FREE: Color = Color::rgb(0, 200, 0);
const COLOR_ALLOC: Color = Color::rgb(200, 100, 0);
const COLOR_RESERVED: Color = Color::rgb(100, 100, 100);
const COLOR_HEAP: Color = Color::rgb(60, 160, 220);

// ── Formatting helpers ──────────────────────────────────────────────────

fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        let gb = bytes / (1024 * 1024 * 1024);
        let mb = (bytes % (1024 * 1024 * 1024)) / (1024 * 1024);
        format!("{}.{:02} GB", gb, mb * 100 / 1024)
    } else if bytes >= 1024 * 1024 {
        let mb = bytes / (1024 * 1024);
        let kb = (bytes % (1024 * 1024)) / 1024;
        format!("{}.{:01} MB", mb, kb * 10 / 1024)
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{} B", bytes)
    }
}

fn format_u64(v: u64) -> String {
    if v >= 1_000_000 {
        format!("{}M", v / 1_000_000)
    } else if v >= 1_000 {
        format!("{}K", v / 1_000)
    } else {
        format!("{}", v)
    }
}

fn format_hex(v: u64) -> String {
    format!("0x{:016X}", v)
}

fn format_pct(pct: u32) -> String {
    format!("{}%", pct)
}

fn format_region_line(r: &MemRegion) -> String {
    format!("{:>3}  {:<22} {:>16X}  {:>8}  {:>8} KB",
        r.index,
        r.type_name,
        r.start,
        r.pages,
        r.size_kb,
    )
}

fn mem_type_name(t: morpheus_hwinit::MemoryType) -> &'static str {
    match t {
        morpheus_hwinit::MemoryType::Reserved => "Reserved",
        morpheus_hwinit::MemoryType::LoaderCode => "Loader Code",
        morpheus_hwinit::MemoryType::LoaderData => "Loader Data",
        morpheus_hwinit::MemoryType::BootServicesCode => "BS Code",
        morpheus_hwinit::MemoryType::BootServicesData => "BS Data",
        morpheus_hwinit::MemoryType::RuntimeServicesCode => "RT Code",
        morpheus_hwinit::MemoryType::RuntimeServicesData => "RT Data",
        morpheus_hwinit::MemoryType::Conventional => "Conventional",
        morpheus_hwinit::MemoryType::Unusable => "Unusable",
        morpheus_hwinit::MemoryType::AcpiReclaim => "ACPI Reclaim",
        morpheus_hwinit::MemoryType::AcpiNvs => "ACPI NVS",
        morpheus_hwinit::MemoryType::Mmio => "MMIO",
        morpheus_hwinit::MemoryType::MmioPortSpace => "MMIO Port",
        morpheus_hwinit::MemoryType::PalCode => "PAL Code",
        morpheus_hwinit::MemoryType::Persistent => "Persistent",
        morpheus_hwinit::MemoryType::Allocated => "Allocated",
        morpheus_hwinit::MemoryType::AllocatedDma => "Alloc DMA",
        morpheus_hwinit::MemoryType::AllocatedStack => "Alloc Stack",
        morpheus_hwinit::MemoryType::AllocatedPageTable => "Alloc PageTbl",
        morpheus_hwinit::MemoryType::AllocatedHeap => "Alloc Heap",
    }
}
