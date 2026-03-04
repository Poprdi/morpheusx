use super::*;

pub fn compositor_loop(fb: &Framebuffer, comp: &mut Compositor) -> i32 {
    let mut last_status = 0i32;

    while comp.has_children() {
        let mut kb = [0u8; 32];
        let avail = io::stdin_available();
        let n = if avail > 0 { io::read_stdin(&mut kb) } else { 0 };

        if n > 0 {
            let mut has_cycle = false;
            for b in kb.iter_mut().take(n) {
                if *b == 0x1D {
                    has_cycle = true;
                    *b = 0;
                }
            }
            if has_cycle {
                comp.cycle_focus();
            }

            let mut fwd = [0u8; 32];
            let mut fi = 0usize;
            for b in kb.iter().take(n) {
                if *b != 0 || !has_cycle {
                    fwd[fi] = *b;
                    fi += 1;
                }
            }
            if fi > 0 {
                comp.forward_keyboard(&fwd[..fi]);
            }
        }

        comp.forward_mouse();
        comp.update_surfaces();

        if comp.any_surface_mapped() {
            comp.compose(fb);
            let _ = hw::fb_present();
            comp.did_compose = true;
        }

        if let Some(code) = comp.reap_exited() {
            last_status = code;
        }

        process::yield_cpu();
    }

    last_status
}
