use super::renderer::{Screen, EFI_DARKGREEN, EFI_LIGHTGREEN, EFI_BLACK};
use alloc::vec::Vec;

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
        if max == 0 { return 0; }
        self.next() % max
    }
}

pub struct RainColumn {
    pub x: usize,
    pub y: isize,
    pub length: usize,
    pub speed: usize,
}

impl RainColumn {
    pub fn new(x: usize, rng: &mut Rng) -> Self {
        Self {
            x,
            y: -(rng.range(15) as isize),
            length: (rng.range(6) + 4) as usize,
            speed: 1,  // Always 1 for slower, smoother fall
        }
    }

    pub fn update(&mut self, max_y: usize, rng: &mut Rng) {
        self.y += self.speed as isize;
        if self.y > max_y as isize + self.length as isize {
            self.y = -(rng.range(10) as isize);
            self.length = (rng.range(6) + 4) as usize;
        }
    }
}

pub struct MatrixRain {
    columns: Vec<RainColumn>,
    rng: Rng,
    screen_height: usize,
    screen_width: usize,
}

impl MatrixRain {
    pub fn new(screen_width: usize, screen_height: usize) -> Self {
        let mut rng = Rng::new(0x1337BEEF);
        let num_cols = (screen_width / 3).min(40).max(10);
        
        let mut columns = Vec::new();
        
        // Distribute columns across screen width
        let spacing = screen_width / num_cols;
        for i in 0..num_cols {
            let mut col = RainColumn::new(i * spacing, &mut rng);
            col.x = i * spacing;
            columns.push(col);
        }

        Self { 
            columns, 
            rng, 
            screen_height,
            screen_width,
        }
    }

    // Simple busy-wait delay - controls animation speed independently
    fn delay(&self) {
        for _ in 0..40000000 {  // slow drip for cinematic effect
            unsafe { core::ptr::read_volatile(&0); }
        }
    }

    pub fn render_frame(&mut self, screen: &mut Screen) {
        // Update all column positions (independent of any external frame counter)
        for col in &mut self.columns {
            col.update(self.screen_height, &mut self.rng);
        }

        // Render each column
        for col in &self.columns {
            for i in 0..col.length {
                let y = col.y - i as isize;
                
                if y >= 0 && y < self.screen_height as isize && col.x < self.screen_width {
                    let y_pos = y as usize;
                    let x_pos = col.x;
                    
                    // Check if this position is masked (has content)
                    let is_masked = if y_pos < screen.mask.len() && x_pos < screen.mask[0].len() {
                        screen.mask[y_pos][x_pos]
                    } else {
                        false
                    };
                    
                    // Only render rain if position is not masked
                    if !is_masked {
                        // Use different characters for depth perception
                        let (ch, color) = if i == 0 { 
                            // Head: bright green binary
                            (if self.rng.range(2) == 0 { '1' } else { '0' }, EFI_LIGHTGREEN)
                        } else if i < 3 {
                            // Middle: normal green binary  
                            (if self.rng.range(2) == 0 { '1' } else { '0' }, EFI_DARKGREEN)
                        } else {
                            // Tail: dim green with lighter characters for fade effect
                            (if self.rng.range(3) == 0 { '.' } else { ' ' }, EFI_DARKGREEN)
                        };
                        screen.put_char_at(x_pos, y_pos, ch, color, EFI_BLACK);
                    }
                }
            }
        }
        
        // Built-in delay for smooth animation
        self.delay();
    }
}
