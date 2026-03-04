use super::*;

impl Compositor {
    pub fn forward_keyboard(&self, data: &[u8]) {
        if let Some(idx) = self.focused {
            if let Some(win) = &self.windows[idx] {
                let _ = compsys::forward_input(win.pid, data);
            }
        }
    }

    pub fn forward_mouse(&mut self) {
        let ms = hw::mouse_read();
        if ms.dx == 0 && ms.dy == 0 && ms.buttons == 0 {
            return;
        }

        self.mouse_x = (self.mouse_x + ms.dx as i32).clamp(0, self.fb_w as i32 - 1);
        self.mouse_y = (self.mouse_y + ms.dy as i32).clamp(0, self.fb_h as i32 - 1);

        let left = (ms.buttons & 1) != 0;
        let left_was = (self.last_buttons & 1) != 0;
        let left_pressed = left && !left_was;
        let left_released = !left && left_was;
        let mut route_to_child = true;

        if left_released {
            self.capture = None;
        }

        if left_pressed {
            if let Some((idx, region)) = self.hit_test(self.mouse_x, self.mouse_y) {
                self.focused = Some(idx);
                match region {
                    HitRegion::Close => {
                        if let Some(win) = &self.windows[idx] {
                            let _ = process::kill(win.pid, process::signal::SIGKILL);
                        }
                        self.capture = None;
                        route_to_child = false;
                    }
                    HitRegion::Title => {
                        if let Some(win) = &self.windows[idx] {
                            self.capture = Some(MouseCapture::Move {
                                idx,
                                off_x: self.mouse_x - win.x,
                                off_y: self.mouse_y - win.y,
                            });
                        }
                        route_to_child = false;
                    }
                    HitRegion::Resize => {
                        if let Some(win) = &self.windows[idx] {
                            self.capture = Some(MouseCapture::Resize {
                                idx,
                                start_mx: self.mouse_x,
                                start_my: self.mouse_y,
                                start_w: win.w,
                                start_h: win.h,
                            });
                        }
                        route_to_child = false;
                    }
                    HitRegion::Content => {}
                }
            }
        }

        if left {
            if let Some(capture) = self.capture {
                match capture {
                    MouseCapture::Move { idx, off_x, off_y } => {
                        if let Some(win) = self.windows[idx].as_mut() {
                            let nx = self.mouse_x - off_x;
                            let ny = self.mouse_y - off_y;
                            let max_x = (self.fb_w as i32 - win.w as i32).max(0);
                            let max_y = (self.fb_h as i32 - win.h as i32).max(TITLE_H as i32);
                            win.x = nx.clamp(0, max_x);
                            win.y = ny.clamp(TITLE_H as i32, max_y);
                        }
                        route_to_child = false;
                    }
                    MouseCapture::Resize {
                        idx,
                        start_mx,
                        start_my,
                        start_w,
                        start_h,
                    } => {
                        if let Some(win) = self.windows[idx].as_mut() {
                            let dx = self.mouse_x - start_mx;
                            let dy = self.mouse_y - start_my;
                            let max_w = self.fb_w.saturating_sub(win.x.max(0) as u32).max(160);
                            let max_h = self.fb_h.saturating_sub(win.y.max(0) as u32).max(120);
                            let nw = (start_w as i32 + dx).clamp(160, max_w as i32);
                            let nh = (start_h as i32 + dy).clamp(120, max_h as i32);
                            win.w = nw as u32;
                            win.h = nh as u32;
                        }
                        route_to_child = false;
                    }
                }
            }
        }

        self.last_buttons = ms.buttons;

        if route_to_child {
            if let Some(idx) = self.focused {
                if let Some(win) = &self.windows[idx] {
                    let _ = compsys::mouse_forward(win.pid, ms.dx, ms.dy, ms.buttons);
                }
            }
        }
    }

    pub fn cycle_focus(&mut self) {
        let active: alloc::vec::Vec<usize> = self
            .windows
            .iter()
            .enumerate()
            .filter_map(|(i, w)| if w.is_some() { Some(i) } else { None })
            .collect();

        if active.len() < 2 {
            return;
        }

        let cur = self.focused.unwrap_or(0);
        let pos = active.iter().position(|&i| i == cur).unwrap_or(0);
        let next = (pos + 1) % active.len();
        self.focused = Some(active[next]);
    }

    fn hit_test(&self, mx: i32, my: i32) -> Option<(usize, HitRegion)> {
        let mut candidates: [Option<usize>; MAX_WINDOWS] = [None; MAX_WINDOWS];
        let mut cn = 0usize;

        if let Some(fi) = self.focused {
            candidates[cn] = Some(fi);
            cn += 1;
        }
        for (i, w) in self.windows.iter().enumerate().rev() {
            if w.is_some() && self.focused != Some(i) {
                candidates[cn] = Some(i);
                cn += 1;
            }
        }

        for &c in &candidates[..cn] {
            if let Some(idx) = c {
                if let Some(win) = &self.windows[idx] {
                    let outer_x = win.x - BORDER as i32;
                    let outer_y = win.y - TITLE_H as i32 - BORDER as i32;
                    let outer_w = win.w as i32 + BORDER as i32 * 2;
                    let outer_h = win.h as i32 + TITLE_H as i32 + BORDER as i32 * 2;

                    if mx < outer_x || mx >= outer_x + outer_w || my < outer_y || my >= outer_y + outer_h {
                        continue;
                    }

                    let tb_x = outer_x + BORDER as i32;
                    let tb_y = outer_y + BORDER as i32;
                    let tb_w = win.w as i32;
                    let close_x = tb_x + tb_w - 34;
                    let close_w = 30;
                    if my >= tb_y
                        && my < tb_y + TITLE_H as i32
                        && mx >= close_x
                        && mx < close_x + close_w
                    {
                        return Some((idx, HitRegion::Close));
                    }

                    let resize_x = win.x + win.w as i32 - 14;
                    let resize_y = win.y + win.h as i32 - 14;
                    if mx >= resize_x && my >= resize_y {
                        return Some((idx, HitRegion::Resize));
                    }

                    if my >= tb_y && my < tb_y + TITLE_H as i32 {
                        return Some((idx, HitRegion::Title));
                    }

                    if mx >= win.x
                        && mx < win.x + win.w as i32
                        && my >= win.y
                        && my < win.y + win.h as i32
                    {
                        return Some((idx, HitRegion::Content));
                    }
                }
            }
        }

        None
    }
}
