use alloc::boxed::Box;
use alloc::format;
use alloc::vec::Vec;
use morpheus_display::types::FramebufferInfo;
use morpheus_hwinit::serial::puts;
use morpheus_ui::app::{App, AppRegistry, AppResult};
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

struct AppInstance {
    window_id: u32,
    app: Box<dyn App>,
}

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

    // App registry
    let mut registry = AppRegistry::new();
    crate::apps::register_all(&mut registry);

    // Shell window — maximized
    let shell_x = BORDER_WIDTH as i32;
    let shell_y = (TITLE_BAR_HEIGHT + BORDER_WIDTH) as i32;
    let shell_w = screen_w.saturating_sub(BORDER_WIDTH * 2);
    let shell_h = screen_h.saturating_sub(TITLE_BAR_HEIGHT + BORDER_WIDTH * 2);

    let shell_id = wm.create_window(
        "MorpheusX Shell", shell_x, shell_y, shell_w, shell_h,
    );
    let mut shell = Shell::new();
    let mut app_instances: Vec<AppInstance> = Vec::new();

    // Print available apps
    let names = registry.names();
    if !names.is_empty() {
        let mut list = alloc::string::String::from("Apps: ");
        for (i, name) in names.iter().enumerate() {
            if i > 0 { list.push_str(", "); }
            list.push_str(name);
        }
        shell.push_output(&list);
    }

    puts("[DESKTOP] rendering initial frame\n");

    render_shell(&mut shell, &mut wm, shell_id, &theme);
    wm.damage_all();
    wm.compose(&mut fb_canvas, &theme);

    puts("[DESKTOP] entering event loop\n");

    let mut keyboard = Keyboard::new();

    loop {
        let input = match keyboard.poll_key_with_delay() {
            Some(i) => i,
            None => {
                // Tick animations on idle
                let mut any_redraw = false;
                for inst in app_instances.iter_mut() {
                    let tick = morpheus_ui::event::Event::Tick;
                    if matches!(inst.app.handle_event(&tick), AppResult::Redraw) {
                        render_app(inst, &mut wm, &theme);
                        any_redraw = true;
                    }
                }
                if any_redraw {
                    wm.compose(&mut fb_canvas, &theme);
                }
                continue;
            }
        };

        let event = match translate_key(&input, &keyboard) {
            Some(e) => e,
            None => continue,
        };

        let wm_result = wm.dispatch_event(&event);

        if matches!(wm_result, EventResult::Consumed) {
            // WM consumed it (e.g. Alt+Tab) — re-render everything
            render_shell(&mut shell, &mut wm, shell_id, &theme);
            for inst in app_instances.iter_mut() {
                render_app(inst, &mut wm, &theme);
            }
            wm.compose(&mut fb_canvas, &theme);
            continue;
        }

        // Route event to focused window
        let focused_id = wm.focused_window().map(|w| w.id);

        if focused_id == Some(shell_id) {
            let ids = wm.window_ids();
            let action = shell.handle_event(&event, &ids);

            match action {
                ShellAction::None => {}
                ShellAction::OpenApp(name) => {
                    puts("[DESKTOP] OpenApp: looking up entry\n");
                    if let Some(entry) = registry.find(&name) {
                        puts("[DESKTOP] OpenApp: entry found, computing geometry\n");
                        let (def_w, def_h) = entry.default_size;
                        let app_w = def_w.min(screen_w.saturating_sub(40));
                        let app_h = def_h.min(screen_h.saturating_sub(60));
                        let app_x = ((screen_w - app_w) / 2) as i32;
                        let app_y = ((screen_h - app_h) / 2) as i32 + TITLE_BAR_HEIGHT as i32;

                        puts("[DESKTOP] OpenApp: creating window\n");
                        let win_id = wm.create_window(entry.title, app_x, app_y, app_w, app_h);
                        puts("[DESKTOP] OpenApp: window created, calling create()\n");
                        let mut app = (entry.create)();
                        puts("[DESKTOP] OpenApp: app constructed, calling init()\n");

                        if let Some(win) = wm.window_mut(win_id) {
                            app.init(&mut win.buffer, &theme);
                        }
                        puts("[DESKTOP] OpenApp: init() done, calling render_app()\n");

                        let mut inst = AppInstance { window_id: win_id, app };
                        render_app(&mut inst, &mut wm, &theme);
                        puts("[DESKTOP] OpenApp: render_app() done\n");
                        app_instances.push(inst);

                        shell.push_output(&format!("Opened '{}' [window {}]", name, win_id));
                    } else {
                        let names = registry.names();
                        if names.is_empty() {
                            shell.push_output(&format!("Unknown app: {}", name));
                        } else {
                            shell.push_output(&format!("Unknown app: {}. Available: {}", name, names.join(", ")));
                        }
                    }
                }
                ShellAction::CloseWindow(id) => {
                    if id == shell_id {
                        shell.push_output("Cannot close the shell.");
                    } else {
                        app_instances.retain(|inst| inst.window_id != id);
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
        } else if let Some(focused) = focused_id {
            // Route to focused app
            let mut close_app = false;
            if let Some(inst) = app_instances.iter_mut().find(|i| i.window_id == focused) {
                match inst.app.handle_event(&event) {
                    AppResult::Close => { close_app = true; }
                    AppResult::Redraw | AppResult::Continue => {
                        render_app(inst, &mut wm, &theme);
                    }
                }
            }
            if close_app {
                app_instances.retain(|inst| inst.window_id != focused);
                wm.close_window(focused);
                render_shell(&mut shell, &mut wm, shell_id, &theme);
            }
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

fn render_app(
    inst: &mut AppInstance,
    wm: &mut WindowManager,
    theme: &morpheus_ui::Theme,
) {
    if let Some(win) = wm.window_mut(inst.window_id) {
        inst.app.render(&mut win.buffer, theme);
        win.dirty = true;
    }
}
