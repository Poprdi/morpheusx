use super::*;

impl Compositor {
    pub fn update_surfaces(&mut self) {
        let count = compsys::surface_list(&mut self.surface_buf);

        for entry in &self.surface_buf[..count] {
            for win in self.windows.iter_mut().flatten() {
                if win.pid == entry.pid && !win.mapped {
                    if let Ok(ptr) = compsys::surface_map(entry.pid) {
                        win.surface_ptr = ptr as *const u32;
                        win.surface_vaddr = ptr as u64;
                        win.surface_pages = entry.pages;
                        win.src_w = entry.width;
                        win.src_h = entry.height;
                        win.src_stride = (entry.stride / 4).max(entry.width.max(1));
                        win.mapped = true;
                    }
                }
            }
        }
    }

    pub fn reap_exited(&mut self) -> Option<i32> {
        let mut focused_exit: Option<i32> = None;

        for (i, slot) in self.windows.iter_mut().enumerate() {
            let exited = if let Some(win) = slot {
                match process::try_wait(win.pid) {
                    Ok(Some(code)) => {
                        if win.mapped && win.surface_vaddr != 0 && win.surface_pages != 0 {
                            let _ = mem::munmap(win.surface_vaddr, win.surface_pages);
                        }
                        if self.focused == Some(i) {
                            focused_exit = Some(code);
                        }
                        true
                    }
                    _ => false,
                }
            } else {
                false
            };
            if exited {
                *slot = None;
            }
        }

        if let Some(fi) = self.focused {
            if self.windows[fi].is_none() {
                self.focused = self.windows.iter().rposition(|w| w.is_some());
            }
        }

        focused_exit
    }
}
