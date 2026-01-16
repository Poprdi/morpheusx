//! Input handling for the Distro Downloader UI.
//!
//! Handles keyboard input for all UI modes (Browse, Confirm, Download, Result, Manage).

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use super::helpers::ManageAction;
use super::render::{render_full, render_list_and_details, RenderContext};
use crate::tui::distro_downloader::catalog::{get_by_category, DistroEntry, CATEGORIES};
use crate::tui::distro_downloader::state::{DownloadState, DownloadStatus, UiMode, UiState};
use crate::tui::input::InputKey;
use crate::tui::renderer::Screen;
use morpheus_core::iso::{IsoStorageManager, MAX_ISOS};

/// Input handling context - mutable references for state modifications
pub struct InputContext<'a> {
    pub ui_state: &'a mut UiState,
    pub download_state: &'a mut DownloadState,
    pub current_distros: &'a mut Vec<&'static DistroEntry>,
    pub iso_storage: &'a mut IsoStorageManager,
    pub iso_names: &'a mut [[u8; 64]; MAX_ISOS],
    pub iso_name_lens: &'a mut [usize; MAX_ISOS],
    pub iso_sizes_mb: &'a mut [u64; MAX_ISOS],
    pub iso_complete: &'a mut [bool; MAX_ISOS],
    pub needs_full_redraw: &'a mut bool,
    pub boot_services: *const crate::BootServices,
    pub image_handle: *mut (),
}

impl<'a> InputContext<'a> {
    /// Get currently selected distro
    pub fn selected_distro(&self) -> Option<&'static DistroEntry> {
        self.current_distros
            .get(self.ui_state.selected_distro)
            .copied()
    }

    /// Refresh the distro list for current category
    pub fn refresh_distro_list(&mut self) {
        let category = self.ui_state.current_category();
        *self.current_distros = get_by_category(category).collect();
        *self.needs_full_redraw = true;
    }

    /// Refresh ISO cache from storage manager
    pub fn refresh_iso_cache(&mut self) {
        self.ui_state.update_iso_count(self.iso_storage.count());

        for (i, (_, entry)) in self.iso_storage.iter().enumerate() {
            if i >= MAX_ISOS {
                break;
            }
            let manifest = &entry.manifest;

            // Copy name
            let name_len = manifest.name_len.min(64);
            self.iso_names[i][..name_len].copy_from_slice(&manifest.name[..name_len]);
            self.iso_name_lens[i] = name_len;

            // Size in MB
            self.iso_sizes_mb[i] = manifest.total_size / (1024 * 1024);

            // Completion status
            self.iso_complete[i] = manifest.is_complete();
        }
    }

    /// Build a render context from current state
    pub fn render_context(&self) -> RenderContext<'_> {
        RenderContext {
            ui_state: self.ui_state,
            download_status: self.download_state.status,
            download_progress: self.download_state.progress_percent(),
            error_message: self.download_state.error_message,
            current_distros: self.current_distros,
            iso_names: self.iso_names,
            iso_name_lens: self.iso_name_lens,
            iso_sizes_mb: self.iso_sizes_mb,
            iso_complete: self.iso_complete,
        }
    }
}

/// Handle input and return action
pub fn handle_input(ctx: &mut InputContext, key: &InputKey, screen: &mut Screen) -> ManageAction {
    match ctx.ui_state.mode {
        UiMode::Browse => handle_browse_input(ctx, key, screen),
        UiMode::Confirm => handle_confirm_input(ctx, key, screen),
        UiMode::Downloading => handle_download_input(ctx, key, screen),
        UiMode::Result => handle_result_input(ctx, key, screen),
        UiMode::Manage => handle_manage_input(ctx, key, screen),
        UiMode::ConfirmDelete => handle_confirm_delete_input(ctx, key, screen),
    }
}

fn handle_browse_input(
    ctx: &mut InputContext,
    key: &InputKey,
    screen: &mut Screen,
) -> ManageAction {
    match key.scan_code {
        // Up arrow
        0x01 => {
            ctx.ui_state.prev_distro();
            let render_ctx = ctx.render_context();
            render_list_and_details(&render_ctx, screen);
        }
        // Down arrow
        0x02 => {
            let count = ctx.current_distros.len();
            ctx.ui_state.next_distro(count);
            let render_ctx = ctx.render_context();
            render_list_and_details(&render_ctx, screen);
        }
        // Left arrow - previous category
        0x04 => {
            ctx.ui_state.prev_category();
            ctx.refresh_distro_list();
            let render_ctx = ctx.render_context();
            render_full(&render_ctx, screen, true);
        }
        // Right arrow - next category
        0x03 => {
            ctx.ui_state.next_category(CATEGORIES.len());
            ctx.refresh_distro_list();
            let render_ctx = ctx.render_context();
            render_full(&render_ctx, screen, true);
        }
        // ESC - exit
        0x17 => {
            return ManageAction::Exit;
        }
        _ => {
            // Enter key - show confirm dialog
            if key.unicode_char == 0x0D && ctx.selected_distro().is_some() {
                ctx.ui_state.show_confirm();
                *ctx.needs_full_redraw = true;
                let render_ctx = ctx.render_context();
                render_full(&render_ctx, screen, true);
            }
            // 'm' or 'M' - switch to manage view
            else if key.unicode_char == b'm' as u16 || key.unicode_char == b'M' as u16 {
                ctx.refresh_iso_cache();
                ctx.ui_state.show_manage();
                *ctx.needs_full_redraw = true;
                let render_ctx = ctx.render_context();
                render_full(&render_ctx, screen, true);
            }
        }
    }
    ManageAction::Continue
}

fn handle_confirm_input(
    ctx: &mut InputContext,
    key: &InputKey,
    screen: &mut Screen,
) -> ManageAction {
    // ESC - cancel
    if key.scan_code == 0x17 {
        ctx.ui_state.return_to_browse();
        *ctx.needs_full_redraw = true;
        let render_ctx = ctx.render_context();
        render_full(&render_ctx, screen, true);
        return ManageAction::Continue;
    }

    // Y/y - confirm download
    if key.unicode_char == b'y' as u16 || key.unicode_char == b'Y' as u16 {
        if let Some(distro) = ctx.selected_distro() {
            start_download(ctx, distro, screen);
        }
        return ManageAction::Continue;
    }

    // N/n - cancel
    if key.unicode_char == b'n' as u16 || key.unicode_char == b'N' as u16 {
        ctx.ui_state.return_to_browse();
        *ctx.needs_full_redraw = true;
        let render_ctx = ctx.render_context();
        render_full(&render_ctx, screen, true);
    }

    ManageAction::Continue
}

fn handle_download_input(
    ctx: &mut InputContext,
    key: &InputKey,
    screen: &mut Screen,
) -> ManageAction {
    // ESC cancels download
    if key.scan_code == 0x17 {
        ctx.download_state.fail("Cancelled by user");
        ctx.ui_state.show_result("Download cancelled");
        *ctx.needs_full_redraw = true;
        let render_ctx = ctx.render_context();
        render_full(&render_ctx, screen, true);
    }
    ManageAction::Continue
}

fn handle_result_input(
    ctx: &mut InputContext,
    key: &InputKey,
    screen: &mut Screen,
) -> ManageAction {
    // Any key returns to browse
    if key.scan_code != 0 || key.unicode_char != 0 {
        ctx.ui_state.return_to_browse();
        ctx.download_state.reset();
        ctx.refresh_iso_cache(); // Refresh after download
        *ctx.needs_full_redraw = true;
        let render_ctx = ctx.render_context();
        render_full(&render_ctx, screen, true);
    }
    ManageAction::Continue
}

fn handle_manage_input(
    ctx: &mut InputContext,
    key: &InputKey,
    screen: &mut Screen,
) -> ManageAction {
    match key.scan_code {
        // Up arrow
        0x01 => {
            ctx.ui_state.prev_iso();
            let render_ctx = ctx.render_context();
            render_full(&render_ctx, screen, false);
        }
        // Down arrow
        0x02 => {
            ctx.ui_state.next_iso();
            let render_ctx = ctx.render_context();
            render_full(&render_ctx, screen, false);
        }
        // ESC - back to browse
        0x17 => {
            ctx.ui_state.return_from_manage();
            *ctx.needs_full_redraw = true;
            let render_ctx = ctx.render_context();
            render_full(&render_ctx, screen, true);
        }
        _ => {
            // 'd' or 'D' - delete
            if (key.unicode_char == b'd' as u16 || key.unicode_char == b'D' as u16)
                && ctx.ui_state.iso_count > 0
            {
                ctx.ui_state.show_confirm_delete();
                *ctx.needs_full_redraw = true;
                let render_ctx = ctx.render_context();
                render_full(&render_ctx, screen, true);
            }
            // 'r' or 'R' - refresh
            else if key.unicode_char == b'r' as u16 || key.unicode_char == b'R' as u16 {
                ctx.refresh_iso_cache();
                *ctx.needs_full_redraw = true;
                let render_ctx = ctx.render_context();
                render_full(&render_ctx, screen, true);
            }
        }
    }
    ManageAction::Continue
}

fn handle_confirm_delete_input(
    ctx: &mut InputContext,
    key: &InputKey,
    screen: &mut Screen,
) -> ManageAction {
    // Y/y - confirm delete
    if key.unicode_char == b'y' as u16 || key.unicode_char == b'Y' as u16 {
        let idx = ctx.ui_state.selected_iso;
        if ctx.iso_storage.remove_entry(idx).is_ok() {
            ctx.refresh_iso_cache();
        }
        ctx.ui_state.cancel_confirm();
        *ctx.needs_full_redraw = true;
        let render_ctx = ctx.render_context();
        render_full(&render_ctx, screen, true);
        return ManageAction::Continue;
    }

    // N/n/ESC - cancel
    if key.unicode_char == b'n' as u16 || key.unicode_char == b'N' as u16 || key.scan_code == 0x17 {
        ctx.ui_state.cancel_confirm();
        *ctx.needs_full_redraw = true;
        let render_ctx = ctx.render_context();
        render_full(&render_ctx, screen, true);
    }

    ManageAction::Continue
}

/// Start downloading a distribution
///
/// This triggers the commit flow that exits UEFI boot services and
/// enters bare-metal mode. hwinit takes ownership of the machine.
/// This function will NEVER RETURN - we leave UEFI forever.
fn start_download(ctx: &mut InputContext, distro: &'static DistroEntry, screen: &mut Screen) {
    ctx.ui_state.start_download();
    ctx.download_state.start_check(distro.filename);
    *ctx.needs_full_redraw = true;
    let render_ctx = ctx.render_context();
    render_full(&render_ctx, screen, true);

    // Build download configuration
    let config = crate::tui::distro_downloader::commit::DownloadCommitConfig {
        iso_url: String::from(distro.url),
        iso_size: distro.size_bytes,
        distro_name: String::from(distro.name),
    };

    // ═══════════════════════════════════════════════════════════════════════
    // POINT OF NO RETURN - Exit UEFI, hwinit owns the world
    // ═══════════════════════════════════════════════════════════════════════
    unsafe {
        crate::tui::distro_downloader::commit::commit_to_download_selfcontained(
            ctx.boot_services,
            ctx.image_handle,
            screen,
            config,
        );
    }
    // NOTE: We never reach here - UEFI is gone, hwinit owns the machine
}
