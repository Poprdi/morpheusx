use super::renderer::{Screen, EFI_BLACK, EFI_DARKGREEN, EFI_LIGHTGREEN};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

// Global rain toggle
static RAIN_ENABLED: AtomicBool = AtomicBool::new(false);
static mut GLOBAL_RAIN: Option<MatrixRain> = None;

pub fn toggle_rain(screen: &Screen) {
    let was_enabled = RAIN_ENABLED.load(Ordering::Relaxed);
    RAIN_ENABLED.store(!was_enabled, Ordering::Relaxed);

    unsafe {
        if !was_enabled {
            GLOBAL_RAIN = Some(MatrixRain::new(screen.width(), screen.height()));
        } else {
            GLOBAL_RAIN = None;
        }
    }
}

pub fn render_rain(screen: &mut Screen) {
    if RAIN_ENABLED.load(Ordering::Relaxed) {
        unsafe {
            if let Some(ref mut rain) = GLOBAL_RAIN {
                rain.render_frame(screen);
            }
        }
    }
}

// Simple PRNG for rain animation (LCG algorithm)
pub struct Rng {
    state: u32,
}

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    pub fn next(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(1103515245).wrapping_add(12345);
        self.state
    }

    pub fn range(&mut self, max: u32) -> u32 {
        if max == 0 {
            return 0;
        }
        self.next() % max
    }
}

pub struct RainColumn {
    pub x: usize,
    pub y: isize,
    pub length: usize,
    pub speed: usize,
    pub tick_delay: u32,   // Frames to wait before next update
    pub tick_counter: u32, // Current frame counter
}

impl RainColumn {
    pub fn new(x: usize, rng: &mut Rng) -> Self {
        Self {
            x,
            y: -(rng.range(15) as isize),
            length: (rng.range(6) + 4) as usize,
            speed: 1,
            tick_delay: if rng.range(3) == 0 { 2 } else { 1 }, // Mostly 1, occasionally 2
            tick_counter: 0,
        }
    }

    pub fn update(&mut self, max_y: usize, rng: &mut Rng) {
        // Increment tick counter
        self.tick_counter += 1;

        // Only update position when tick counter reaches delay
        if self.tick_counter >= self.tick_delay {
            self.tick_counter = 0; // Reset counter
            self.y += self.speed as isize;

            // Reset column when it goes off screen
            if self.y > max_y as isize + self.length as isize {
                self.y = -(rng.range(10) as isize);
                self.length = (rng.range(6) + 4) as usize;
                self.tick_delay = if rng.range(3) == 0 { 2 } else { 1 }; // Mostly 1, occasionally 2
            }
        }
    }
}

pub struct MatrixRain {
    columns: Vec<RainColumn>,
    rng: Rng,
    screen_height: usize,
    screen_width: usize,
    frame_count: u32,
}

impl MatrixRain {
    pub fn new(screen_width: usize, screen_height: usize) -> Self {
        let mut rng = Rng::new(0x1337BEEF);
        // One column per character position for dense rain
        let num_cols = screen_width;

        let mut columns = Vec::new();

        // Create a column for every x position with heavily staggered start positions
        for x in 0..num_cols {
            // Spread initial positions across a large range to create continuous flow
            let max_offset = screen_height * 2;
            let y_offset = (x * max_offset / num_cols) as isize;

            columns.push(RainColumn {
                x,
                y: -(rng.range(20) as isize) - y_offset,
                length: (rng.range(6) + 4) as usize,
                speed: 1,
                tick_delay: if rng.range(3) == 0 { 2 } else { 1 },
                tick_counter: 0,
            });
        }

        Self {
            columns,
            rng,
            screen_height,
            screen_width,
            frame_count: 0,
        }
    }

    // Simple busy-wait delay - controls animation speed independently
    fn delay(&self) {
        // Safe busy loop without dangerous volatile reads
        let mut counter = 0u32;
        for _ in 0..40000000 {
            counter = counter.wrapping_add(1);
            // Prevents optimizer from removing the loop
            core::hint::black_box(counter);
        }
    }

    pub fn render_frame(&mut self, screen: &mut Screen) {
        self.frame_count = self.frame_count.wrapping_add(1);

        // Update all column positions
        for col in &mut self.columns {
            col.update(self.screen_height, &mut self.rng);
        }

        // Render each column - IGNORING mask to eat through UI
        for col in &self.columns {
            for i in 0..col.length {
                let y = col.y - i as isize;

                if y >= 0 && y < self.screen_height as isize && col.x < self.screen_width {
                    let y_pos = y as usize;
                    let x_pos = col.x;

                    // Easter egg mode: render rain OVER everything, eating the UI
                    let (ch, color) = if i == 0 {
                        // Head: bright green binary
                        (
                            if self.rng.range(2) == 0 { '1' } else { '0' },
                            EFI_LIGHTGREEN,
                        )
                    } else if i < 3 {
                        // Middle: normal green binary
                        (
                            if self.rng.range(2) == 0 { '1' } else { '0' },
                            EFI_DARKGREEN,
                        )
                    } else {
                        // Tail: dim green dots for cosmic trail effect
                        (
                            if self.rng.range(3) == 0 { '.' } else { ' ' },
                            EFI_DARKGREEN,
                        )
                    };
                    screen.put_char_at(x_pos, y_pos, ch, color, EFI_BLACK);
                }
            }
        }

        // Built-in delay for smooth animation
        self.delay();
    }
}
