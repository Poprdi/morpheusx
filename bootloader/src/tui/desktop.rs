use alloc::format;
use morpheus_display::types::FramebufferInfo;
use morpheus_hwinit::serial::puts;
use morpheus_ui::Canvas;
use morpheus_ui::EventResult;
use morpheus_ui::shell::commands;
use morpheus_ui::shell::{Shell, ShellAction};
use morpheus_ui::theme::THEME_DEFAULT;
use morpheus_ui::window::{BORDER_WIDTH, TITLE_BAR_HEIGHT};
use morpheus_ui::wm::WindowManager;

use super::event_adapter::translate_key;
use super::fb_canvas::FbCanvas;
use super::input::Keyboard;

pub fn run_desktop(display_info: &FramebufferInfo) -> ! {
    puts("[DESKTOP] initializing window manager\n");

    let theme = THEME_DEFAULT;
    let mut fb_canvas = unsafe { FbCanvas::new(display_info) };
    let screen_w = fb_canvas.width();
    let screen_h = fb_canvas.height();

    let mut wm = WindowManager::new(
        screen_w, screen_h,
        fb_canvas.pixel_format(),
        &theme,
    );

    let shell_x = BORDER_WIDTH as i32;
    let shell_y = (TITLE_BAR_HEIGHT + BORDER_WIDTH) as i32;
    let shell_w = screen_w.saturating_sub(BORDER_WIDTH * 2);
    let shell_h = screen_h.saturating_sub(TITLE_BAR_HEIGHT + BORDER_WIDTH * 2);

    let shell_id = wm.create_window(
        "MorpheusX Shell", shell_x, shell_y, shell_w, shell_h,
    );
    let mut shell = Shell::new();

    puts("[DESKTOP] rendering initial frame\n");

    render_shell(&mut shell, &mut wm, shell_id, &theme);
    wm.damage_all();
    wm.compose(&mut fb_canvas, &theme);

    puts("[DESKTOP] entering event loop\n");

    let mut keyboard = Keyboard::new();

    loop {
        let input = match keyboard.poll_key_with_delay() {
            Some(i) => i,
            None => continue,
        };

        let event = match translate_key(&input, &keyboard) {
            Some(e) => e,
            None => continue,
        };

        let wm_result = wm.dispatch_event(&event);

        if matches!(wm_result, EventResult::Ignored) {
            let ids = wm.window_ids();
            let action = shell.handle_event(&event, &ids);

            match action {
                ShellAction::None => {}
                ShellAction::OpenApp(name) => {
                    shell.push_output(&format!("No app '{}' registered yet.", name));
                }
                ShellAction::CloseWindow(id) => {
                    if id == shell_id {
                        shell.push_output("Cannot close the shell.");
                    } else {
                        wm.close_window(id);
                        shell.push_output(&format!("Closed window {}.", id));
                    }
                }
                ShellAction::ListWindows => {
                    let ids = wm.window_ids();
                    let list = commands::format_window_list(&ids);
                    shell.push_output(&list);
                }
                ShellAction::Exit => {
                    shell.push_output("Halting...");
                    render_shell(&mut shell, &mut wm, shell_id, &theme);
                    wm.compose(&mut fb_canvas, &theme);
                    puts("[DESKTOP] halt requested\n");
                    loop { core::hint::spin_loop(); }
                }
            }

            render_shell(&mut shell, &mut wm, shell_id, &theme);
        }

        wm.compose(&mut fb_canvas, &theme);
    }
}

fn render_shell(
    shell: &mut Shell,
    wm: &mut WindowManager,
    shell_id: u32,
    theme: &morpheus_ui::Theme,
) {
    if let Some(win) = wm.window_mut(shell_id) {
        shell.render(&mut win.buffer, theme);
        win.dirty = true;
    }
}
