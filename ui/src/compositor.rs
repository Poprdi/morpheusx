use alloc::vec::Vec;
use crate::canvas::Canvas;
use crate::color::Color;
use crate::rect::Rect;
use crate::theme::Theme;
use crate::window::Window;

const MAX_DAMAGE_RECTS: usize = 16;

pub struct Compositor {
    damage: Vec<Rect>,
    desktop_color: Color,
}

impl Compositor {
    pub fn new(desktop_color: Color) -> Self {
        Self {
            damage: Vec::with_capacity(MAX_DAMAGE_RECTS),
            desktop_color,
        }
    }

    pub fn set_desktop_color(&mut self, color: Color) {
        self.desktop_color = color;
    }

    pub fn damage_rect(&mut self, rect: Rect) {
        if rect.is_empty() {
            return;
        }
        self.damage.push(rect);
    }

    pub fn damage_full(&mut self, screen_w: u32, screen_h: u32) {
        self.damage.clear();
        self.damage.push(Rect::new(0, 0, screen_w, screen_h));
    }

    pub fn has_damage(&self) -> bool {
        !self.damage.is_empty()
    }

    pub fn compose(
        &mut self,
        canvas: &mut dyn Canvas,
        windows: &[Window],
        _theme: &Theme,
    ) {
        if self.damage.is_empty() {
            return;
        }

        if self.damage.len() > MAX_DAMAGE_RECTS {
            let screen = canvas.bounds();
            self.damage.clear();
            self.damage.push(screen);
        }

        let merged = merge_rects(&self.damage);

        let screen = canvas.bounds();

        for dmg in &merged {
            let dmg = match dmg.intersect(screen) {
                Some(d) => d,
                None => continue,
            };

            canvas.fill_rect(dmg.x, dmg.y, dmg.w, dmg.h, self.desktop_color);

            for win in windows.iter() {
                if !win.visible {
                    continue;
                }

                self.blit_window_to_canvas(canvas, win, &dmg);
            }
        }

        self.damage.clear();
    }

    fn blit_window_to_canvas(
        &self,
        canvas: &mut dyn Canvas,
        win: &Window,
        damage: &Rect,
    ) {
        let content_x = win.x.max(0) as u32;
        let content_y = win.y.max(0) as u32;
        let content_rect = Rect::new(content_x, content_y, win.width, win.height);

        let isect = match content_rect.intersect(*damage) {
            Some(i) => i,
            None => return,
        };

        let src_x = isect.x.saturating_sub(content_x);
        let src_y = isect.y.saturating_sub(content_y);

        let src = win.buffer.as_slice();
        let src_w = win.width;
        let format = win.buffer.pixel_format();

        if win.alpha == 255 {
            for row in 0..isect.h {
                let sy = src_y + row;
                let src_start = (sy * src_w + src_x) as usize;
                let src_end = src_start + isect.w as usize;
                if src_end > src.len() {
                    break;
                }
                canvas.blit(
                    isect.x,
                    isect.y + row,
                    &src[src_start..src_end],
                    isect.w,
                    1,
                );
            }
        } else {
            for row in 0..isect.h {
                let sy = src_y + row;
                let src_start = (sy * src_w + src_x) as usize;
                let src_end = src_start + isect.w as usize;
                if src_end > src.len() {
                    break;
                }
                canvas.blit_blend(
                    isect.x,
                    isect.y + row,
                    &src[src_start..src_end],
                    isect.w,
                    1,
                    format,
                );
            }
        }
    }
}

fn merge_rects(rects: &[Rect]) -> Vec<Rect> {
    if rects.is_empty() {
        return Vec::new();
    }

    let mut merged: Vec<Rect> = Vec::with_capacity(rects.len());

    for &r in rects {
        if r.is_empty() {
            continue;
        }
        let mut absorbed = false;
        for m in merged.iter_mut() {
            if rects_overlap_or_touch(m, &r) {
                *m = m.union(r);
                absorbed = true;
                break;
            }
        }
        if !absorbed {
            merged.push(r);
        }
    }

    merged
}

fn rects_overlap_or_touch(a: &Rect, b: &Rect) -> bool {
    a.x <= b.right() && b.x <= a.right() && a.y <= b.bottom() && b.y <= a.bottom()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_overlapping() {
        let rects = [
            Rect::new(0, 0, 10, 10),
            Rect::new(5, 5, 10, 10),
        ];
        let merged = merge_rects(&rects);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0], Rect::new(0, 0, 15, 15));
    }

    #[test]
    fn merge_disjoint() {
        let rects = [
            Rect::new(0, 0, 5, 5),
            Rect::new(20, 20, 5, 5),
        ];
        let merged = merge_rects(&rects);
        assert_eq!(merged.len(), 2);
    }
}
