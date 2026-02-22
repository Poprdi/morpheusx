use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use morpheus_display::types::FramebufferInfo;
use morpheus_hwinit::serial::puts;
use morpheus_ui::app::{App, AppRegistry, AppResult};
use morpheus_ui::Canvas;
use morpheus_ui::EventResult;
use morpheus_ui::shell::commands::{self, FsOp};
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

/// Try to load an ELF binary from the root filesystem at `/bin/<name>`.
///
/// Returns the raw bytes if found, or `None`.
fn load_elf_from_fs(name: &str) -> Option<Vec<u8>> {
    use alloc::string::String;

    let mut path = String::from("/bin/");
    path.push_str(name);

    let fs = unsafe { morpheus_helix::vfs::global::fs_global_mut()? };
    let mount = &fs.mount_table;

    // Check if the file exists via stat.
    let stat = morpheus_helix::vfs::vfs_stat(mount, &path).ok()?;
    if stat.size == 0 {
        return None;
    }

    // Open, read, close.
    let mut fd_table = morpheus_helix::vfs::FdTable::new();
    let ts = morpheus_hwinit::cpu::tsc::read_tsc();
    let fd = morpheus_helix::vfs::vfs_open(
        &mut fs.device, &mut fs.mount_table, &mut fd_table,
        &path, morpheus_helix::types::open_flags::O_READ, ts,
    ).ok()?;

    let mut buf = alloc::vec![0u8; stat.size as usize];
    let n = morpheus_helix::vfs::vfs_read(
        &mut fs.device, &fs.mount_table, &mut fd_table,
        fd, &mut buf,
    ).ok()?;
    buf.truncate(n);

    let _ = morpheus_helix::vfs::vfs_close(&mut fd_table, fd);
    Some(buf)
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

        // Feed printable ASCII characters to the kernel stdin buffer
        // so that user-space processes can read them via SYS_READ(fd=0).
        if input.unicode_char > 0 && input.unicode_char < 128 {
            morpheus_hwinit::stdin::push(input.unicode_char as u8);
        }

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
                ShellAction::SpawnProcess(name) => {
                    puts("[DESKTOP] SpawnProcess: ");
                    puts(&name);
                    puts("\n");

                    // Try to load the ELF from the root filesystem.
                    let elf_data = load_elf_from_fs(&name);

                    match elf_data {
                        Some(data) => {
                            match unsafe {
                                morpheus_hwinit::process::scheduler::spawn_user_process(
                                    &name, &data,
                                )
                            } {
                                Ok(pid) => {
                                    shell.push_output(&format!(
                                        "Spawned '{}' as PID {}", name, pid
                                    ));
                                }
                                Err(e) => {
                                    shell.push_output(&format!(
                                        "Failed to spawn '{}': {}", name, e
                                    ));
                                }
                            }
                        }
                        None => {
                            shell.push_output(&format!(
                                "Binary not found: {}. Place ELF in /bin/{}", name, name
                            ));
                        }
                    }
                }
                ShellAction::FsCommand(op) => {
                    handle_fs_command(op, &mut shell);
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

// ═══════════════════════════════════════════════════════════════════════
// Filesystem command execution
// ═══════════════════════════════════════════════════════════════════════

fn handle_fs_command(op: FsOp, shell: &mut Shell) {
    match op {
        FsOp::Ls { path, long } => {
            let fs = match unsafe { morpheus_helix::vfs::global::fs_global() } {
                Some(fs) => fs,
                None => { shell.push_output("ls: filesystem not initialized"); return; }
            };
            match morpheus_helix::vfs::vfs_readdir(&fs.mount_table, &path) {
                Ok(entries) => {
                    if entries.is_empty() { return; }
                    let out = if long {
                        format_ls_long(&entries)
                    } else {
                        format_ls_short(&entries)
                    };
                    shell.push_output(&out);
                }
                Err(_) => shell.push_output(&format!("ls: cannot access '{}'", path)),
            }
        }

        FsOp::Cd { path } => {
            // Root always exists.
            if path == "/" {
                shell.set_cwd("/");
                return;
            }
            let fs = match unsafe { morpheus_helix::vfs::global::fs_global() } {
                Some(fs) => fs,
                None => { shell.push_output("cd: filesystem not initialized"); return; }
            };
            match morpheus_helix::vfs::vfs_stat(&fs.mount_table, &path) {
                Ok(stat) if stat.is_dir => shell.set_cwd(&path),
                Ok(_) => shell.push_output(&format!("cd: not a directory: {}", path)),
                Err(_) => shell.push_output(&format!("cd: no such directory: {}", path)),
            }
        }

        FsOp::Mkdir { path } => {
            let fs = match unsafe { morpheus_helix::vfs::global::fs_global_mut() } {
                Some(fs) => fs,
                None => { shell.push_output("mkdir: filesystem not initialized"); return; }
            };
            let ts = morpheus_hwinit::cpu::tsc::read_tsc();
            match morpheus_helix::vfs::vfs_mkdir(&mut fs.mount_table, &path, ts) {
                Ok(()) => {}
                Err(e) => shell.push_output(&format!("mkdir: '{}': {:?}", path, e)),
            }
        }

        FsOp::Touch { path } => {
            let fs = match unsafe { morpheus_helix::vfs::global::fs_global_mut() } {
                Some(fs) => fs,
                None => { shell.push_output("touch: filesystem not initialized"); return; }
            };
            // No-op if already exists.
            if morpheus_helix::vfs::vfs_stat(&fs.mount_table, &path).is_ok() {
                return;
            }
            let mut fd_table = morpheus_helix::vfs::FdTable::new();
            let ts = morpheus_hwinit::cpu::tsc::read_tsc();
            let flags = morpheus_helix::types::open_flags::O_WRITE
                      | morpheus_helix::types::open_flags::O_CREATE;
            match morpheus_helix::vfs::vfs_open(
                &mut fs.device, &mut fs.mount_table, &mut fd_table,
                &path, flags, ts,
            ) {
                Ok(fd) => { let _ = morpheus_helix::vfs::vfs_close(&mut fd_table, fd); }
                Err(e) => shell.push_output(&format!("touch: '{}': {:?}", path, e)),
            }
        }

        FsOp::Cat { path } => {
            let fs = match unsafe { morpheus_helix::vfs::global::fs_global_mut() } {
                Some(fs) => fs,
                None => { shell.push_output("cat: filesystem not initialized"); return; }
            };
            let stat = match morpheus_helix::vfs::vfs_stat(&fs.mount_table, &path) {
                Ok(s) => s,
                Err(_) => { shell.push_output(&format!("cat: {}: not found", path)); return; }
            };
            if stat.is_dir {
                shell.push_output(&format!("cat: {}: is a directory", path));
                return;
            }
            if stat.size == 0 { return; }

            let mut fd_table = morpheus_helix::vfs::FdTable::new();
            let ts = morpheus_hwinit::cpu::tsc::read_tsc();
            let fd = match morpheus_helix::vfs::vfs_open(
                &mut fs.device, &mut fs.mount_table, &mut fd_table,
                &path, morpheus_helix::types::open_flags::O_READ, ts,
            ) {
                Ok(fd) => fd,
                Err(_) => { shell.push_output(&format!("cat: {}: cannot open", path)); return; }
            };
            let mut buf = alloc::vec![0u8; stat.size as usize];
            let n = match morpheus_helix::vfs::vfs_read(
                &mut fs.device, &fs.mount_table, &mut fd_table, fd, &mut buf,
            ) {
                Ok(n) => n,
                Err(_) => {
                    let _ = morpheus_helix::vfs::vfs_close(&mut fd_table, fd);
                    shell.push_output(&format!("cat: {}: read error", path));
                    return;
                }
            };
            let _ = morpheus_helix::vfs::vfs_close(&mut fd_table, fd);
            buf.truncate(n);
            match core::str::from_utf8(&buf) {
                Ok(s) => shell.push_output(s),
                Err(_) => shell.push_output(&format!("cat: {}: binary file ({} bytes)", path, n)),
            }
        }

        FsOp::Rm { path } => {
            let fs = match unsafe { morpheus_helix::vfs::global::fs_global_mut() } {
                Some(fs) => fs,
                None => { shell.push_output("rm: filesystem not initialized"); return; }
            };
            let ts = morpheus_hwinit::cpu::tsc::read_tsc();
            match morpheus_helix::vfs::vfs_unlink(&mut fs.mount_table, &path, ts) {
                Ok(()) => {}
                Err(e) => shell.push_output(&format!("rm: '{}': {:?}", path, e)),
            }
        }

        FsOp::Mv { src, dst } => {
            let fs = match unsafe { morpheus_helix::vfs::global::fs_global_mut() } {
                Some(fs) => fs,
                None => { shell.push_output("mv: filesystem not initialized"); return; }
            };
            let ts = morpheus_hwinit::cpu::tsc::read_tsc();
            match morpheus_helix::vfs::vfs_rename(&mut fs.mount_table, &src, &dst, ts) {
                Ok(()) => {}
                Err(e) => shell.push_output(&format!("mv: '{}' -> '{}': {:?}", src, dst, e)),
            }
        }

        FsOp::Write { path, content } => {
            let fs = match unsafe { morpheus_helix::vfs::global::fs_global_mut() } {
                Some(fs) => fs,
                None => { shell.push_output("write: filesystem not initialized"); return; }
            };
            let mut fd_table = morpheus_helix::vfs::FdTable::new();
            let ts = morpheus_hwinit::cpu::tsc::read_tsc();
            let flags = morpheus_helix::types::open_flags::O_WRITE
                      | morpheus_helix::types::open_flags::O_CREATE
                      | morpheus_helix::types::open_flags::O_TRUNC;
            let fd = match morpheus_helix::vfs::vfs_open(
                &mut fs.device, &mut fs.mount_table, &mut fd_table,
                &path, flags, ts,
            ) {
                Ok(fd) => fd,
                Err(e) => {
                    shell.push_output(&format!("write: {}: {:?}", path, e));
                    return;
                }
            };
            match morpheus_helix::vfs::vfs_write(
                &mut fs.device, &mut fs.mount_table, &mut fd_table,
                fd, content.as_bytes(), ts,
            ) {
                Ok(n) => {
                    let _ = morpheus_helix::vfs::vfs_close(&mut fd_table, fd);
                    shell.push_output(&format!("{}: {} bytes written", path, n));
                }
                Err(e) => {
                    let _ = morpheus_helix::vfs::vfs_close(&mut fd_table, fd);
                    shell.push_output(&format!("write: {}: {:?}", path, e));
                }
            }
        }

        FsOp::Stat { path } => {
            let fs = match unsafe { morpheus_helix::vfs::global::fs_global() } {
                Some(fs) => fs,
                None => { shell.push_output("stat: filesystem not initialized"); return; }
            };
            match morpheus_helix::vfs::vfs_stat(&fs.mount_table, &path) {
                Ok(stat) => {
                    let kind = if stat.is_dir { "directory" } else { "file" };
                    shell.push_output(&format!(
                        "  File: {}\n  Type: {}\n  Size: {} bytes\n   Key: {:#018x}\n   LSN: {} (first: {})\n  Vers: {}\n Flags: {:#x}",
                        path, kind, stat.size, stat.key,
                        stat.lsn, stat.first_lsn,
                        stat.version_count, stat.flags,
                    ));
                }
                Err(_) => shell.push_output(&format!("stat: cannot stat '{}'", path)),
            }
        }

        FsOp::Sync => {
            let fs = match unsafe { morpheus_helix::vfs::global::fs_global_mut() } {
                Some(fs) => fs,
                None => { shell.push_output("sync: filesystem not initialized"); return; }
            };
            match morpheus_helix::vfs::vfs_sync(&mut fs.device, &mut fs.mount_table) {
                Ok(()) => shell.push_output("filesystem synced"),
                Err(e) => shell.push_output(&format!("sync: {:?}", e)),
            }
        }
    }
}

// ── ls formatting ────────────────────────────────────────────────────

fn format_ls_short(entries: &[morpheus_helix::types::DirEntry]) -> String {
    let mut out = String::new();
    for entry in entries {
        let name = core::str::from_utf8(&entry.name[..entry.name_len as usize])
            .unwrap_or("?");
        if !out.is_empty() { out.push_str("  "); }
        out.push_str(name);
        if entry.is_dir { out.push('/'); }
    }
    out
}

fn format_ls_long(entries: &[morpheus_helix::types::DirEntry]) -> String {
    let mut out = String::new();
    for entry in entries {
        let name = core::str::from_utf8(&entry.name[..entry.name_len as usize])
            .unwrap_or("?");
        let type_char = if entry.is_dir { 'd' } else { '-' };
        let size_str = if entry.is_dir {
            String::from("-")
        } else {
            format_size(entry.size)
        };
        // HelixFS long listing: type  size  version_count  name
        // No permissions — HelixFS has no permission model.
        out.push_str(&format!(
            "{}  {:>8}  v{:<3}  {}{}\n",
            type_char,
            size_str,
            entry.version_count,
            name,
            if entry.is_dir { "/" } else { "" },
        ));
    }
    if out.ends_with('\n') { out.pop(); }
    out
}

/// Format a byte count as a compact human-readable string.
/// No floating point — pure integer arithmetic.
fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        let kb = bytes / 1024;
        let frac = (bytes % 1024) * 10 / 1024;
        if frac > 0 { format!("{}.{}K", kb, frac) } else { format!("{}K", kb) }
    } else if bytes < 1024 * 1024 * 1024 {
        let mb = bytes / (1024 * 1024);
        let frac = (bytes % (1024 * 1024)) * 10 / (1024 * 1024);
        if frac > 0 { format!("{}.{}M", mb, frac) } else { format!("{}M", mb) }
    } else {
        let gb = bytes / (1024 * 1024 * 1024);
        format!("{}G", gb)
    }
}
