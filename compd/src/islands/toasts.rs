//! Notification toasts — compd's consumer of the `whisperd` feed (notifications epic, C3).
//!
//! `whisperd` owns the notification *queue* and publishes the currently-visible set over the
//! `whisperd.feed` persist key; compd owns *only* two things here — drawing that set as a stack of
//! toasts in the top-right corner (above every window, like the context menu) and turning a click on
//! a toast back into a dismiss. It never decides lifetime/policy; it reflects whisperd's published
//! truth and reports the one user gesture (dismiss) back. The placement, hit-testing and word-wrap
//! are the host-tested `phosphor_notify::layout` geometry — the same split `wm_geom` gives windows —
//! so this island is just the binding to compd's framebuffer + persist + mouse.
//!
//! The two channels (see `whisperd`): the **feed** (`whisperd.feed`, daemon → compositor) is raw
//! `encode_feed` bytes — read each frame, re-decoded only when the bytes change, so a quiet desktop
//! costs nothing. The **dismiss** channel (`whisperd.dismiss`, compositor → daemon) is raw
//! `encode_dismiss` bytes — a click writes one. Both are bare domain codecs (no `phosphor_de::ipc`
//! envelope), so compd depends on `phosphor_notify` alone, never on the desktop core.

use alloc::vec::Vec;

use phosphor_notify::layout::{stack_layout, toast_height, toast_hit, wrap_lines, Rect};
use phosphor_notify::{Notification, ToastMetrics, Urgency};

use crate::islands::CompState;

/// daemon → compositor: the visible set as raw `encode_feed` bytes. MUST match
/// `platform_morpheusx::whisperd::IPC_KEY_NOTIFY_FEED` (compd can't import that crate; the key string
/// is the cross-process contract, like `de.desk.cmd` in `desk.rs`).
const FEED_KEY: &str = "whisperd.feed";
/// compositor → daemon: a dismiss request as raw `encode_dismiss` bytes. MUST match
/// `platform_morpheusx::whisperd::IPC_KEY_NOTIFY_DISMISS`.
const DISMISS_KEY: &str = "whisperd.dismiss";

/// Read buffer for the feed. The feed is capped at a handful of short one-liners (whisperd's
/// `FEED_MAX`), so this is generous; an over-long feed would truncate on read and fail to decode,
/// leaving the last good set in place — never a crash.
const FEED_BUF: usize = 4096;

/// Toast geometry in pixels. Width holds a 35-char line at the 8px font cell; `line_h` gives the
/// 16px glyph 2px of leading. The corner `margin` clears the screen edges; the stack descends from
/// the top-right (`phosphor_notify::layout::stack_layout`).
pub const METRICS: ToastMetrics = ToastMetrics {
    width: 300,
    pad: 12,
    line_h: 18,
    gap: 8,
    margin: 12,
    close: 12,
    max_visible: 5,
};

/// Width of the left accent stripe (the quiet urgency cue down a toast's left edge). Shared with the
/// renderer so the stripe and the text inset agree.
pub const ACCENT_W: i32 = 4;

/// Characters that fit on one text line inside the padding (and clear of the accent stripe).
#[inline]
fn max_text_chars() -> usize {
    (((METRICS.width - 2 * METRICS.pad - ACCENT_W) / 8).max(1)) as usize
}

/// The body text wrapped to the toast width (empty for an empty body). Shared by height and draw so
/// the box always holds exactly the lines that get painted.
pub fn body_lines(n: &Notification) -> Vec<alloc::string::String> {
    wrap_lines(&n.body, max_text_chars())
}

/// Number of text lines a toast occupies: an app header (only when the source named itself), the
/// always-present summary, then the wrapped body.
fn line_count(n: &Notification) -> usize {
    let head = usize::from(!n.app.is_empty());
    head + 1 + body_lines(n).len()
}

/// Whether `id` is in the recently-dismissed ring (so it is suppressed locally until whisperd drops
/// it from the feed). `0` is never a real id (whisperd ids start at 1) so it can't match.
fn is_dismissed(state: &CompState, id: u32) -> bool {
    id != 0 && state.toast_dismissed.contains(&id)
}

/// Read the feed and, when it changed, decode it into `state.toasts`. On an absent key (boot, before
/// whisperd's first publish) the set clears; on a decode failure the last good set is kept (a
/// half-written key must never blank or crash the overlay). The dismiss ring is pruned against the
/// fresh set so an id reused after a whisperd restart is not wrongly suppressed.
pub fn consume_feed(state: &mut CompState) {
    let mut buf = [0u8; FEED_BUF];
    let bytes: &[u8] = match libmorpheus::persist::get(FEED_KEY, &mut buf) {
        Ok(n) => &buf[..n],
        Err(_) => &[], // absent ⇒ empty feed.
    };
    if bytes == state.toast_feed_raw.as_slice() {
        return; // unchanged since last frame — nothing to do.
    }
    // Remember the raw bytes even on a decode failure, so we don't re-decode the same bad blob every
    // frame; a later real change produces different bytes and is retried.
    state.toast_feed_raw.clear();
    state.toast_feed_raw.extend_from_slice(bytes);

    if bytes.is_empty() {
        state.toasts.clear();
    } else if let Some(items) = phosphor_notify::wire::decode_feed(bytes) {
        state.toasts = items;
    }
    prune_ring(state);
}

/// Clear dismiss-ring entries whose id is no longer in the feed: once whisperd has dropped a
/// dismissed notification, the local suppression is done, and clearing it means a future id that
/// happens to reuse that number (after a whisperd restart resets the id counter) shows normally.
fn prune_ring(state: &mut CompState) {
    for k in 0..crate::islands::TOAST_DISMISS_RING {
        let id = state.toast_dismissed[k];
        if id != 0 && !state.toasts.iter().any(|n| n.id == id) {
            state.toast_dismissed[k] = 0;
        }
    }
}

/// The toasts to show this frame (indices into `state.toasts`, dismissed ones filtered out, capped to
/// `max_visible`) paired 1:1 with their on-screen rects. Shared by the renderer and the hit-test so
/// the drawn boxes are exactly the clickable boxes. The feed arrives already ordered (sticky-pinned,
/// newest-first) and capped by whisperd; the cap here is belt-and-suspenders.
pub fn visible(state: &CompState) -> (Vec<usize>, Vec<Rect>) {
    let mut idxs: Vec<usize> = Vec::new();
    for (i, n) in state.toasts.iter().enumerate() {
        if is_dismissed(state, n.id) {
            continue;
        }
        idxs.push(i);
        if idxs.len() >= METRICS.max_visible {
            break;
        }
    }
    let heights: Vec<i32> = idxs
        .iter()
        .map(|&i| toast_height(METRICS, line_count(&state.toasts[i])))
        .collect();
    let rects = stack_layout(METRICS, state.fb_w as i32, state.fb_h as i32, &heights);
    (idxs, rects)
}

/// `true` if the pointer is over any toast — so the renderer keeps an arrow cursor over a toast
/// rather than letting a window beneath it drive a move/resize shape.
pub fn hovering(state: &CompState, mx: i32, my: i32) -> bool {
    let (_idxs, rects) = visible(state);
    toast_hit(&rects, mx, my).is_some()
}

/// Handle a left press at `(mx, my)`: if it lands on a toast, dismiss that toast and return `true`
/// (the click is consumed — it does not also fall through to the window beneath). A click that misses
/// every toast returns `false`, so the press routes normally (toasts are non-modal). Click-anywhere
/// on a toast dismisses, the conventional gesture.
pub fn dismiss_at(state: &mut CompState, mx: i32, my: i32) -> bool {
    let (idxs, rects) = visible(state);
    match toast_hit(&rects, mx, my) {
        Some(hit) => {
            let id = state.toasts[idxs[hit]].id;
            dismiss(state, id);
            true
        },
        None => false,
    }
}

/// Dismiss the notification `id`: record it in the local ring so it disappears on the next frame, and
/// write a dismiss request to whisperd (which republishes the feed without it shortly after). The
/// request token is minted from the monotonic clock, clamped strictly increasing, so whisperd
/// services each dismiss once even across a compd restart.
fn dismiss(state: &mut CompState, id: u32) {
    state.toast_dismissed[state.toast_dismiss_w] = id;
    state.toast_dismiss_w = (state.toast_dismiss_w + 1) % crate::islands::TOAST_DISMISS_RING;

    let now = libmorpheus::time::uptime_ms();
    let token = now.max(state.toast_dismiss_token.saturating_add(1));
    state.toast_dismiss_token = token;
    let blob = phosphor_notify::wire::encode_dismiss(token, id);
    let _ = libmorpheus::persist::put(DISMISS_KEY, &blob);
    libmorpheus::debug!("toast dismiss #{}", id);
}

/// The accent colour for an urgency: a danger red for Critical, the theme focus accent for Normal, a
/// muted grey for Low. Used for the left stripe and the app-name header.
pub fn accent_rgb(state: &CompState, urgency: Urgency) -> (u8, u8, u8) {
    match urgency {
        Urgency::Critical => (210, 86, 86),
        Urgency::Normal => state.border_focused_rgb,
        Urgency::Low => (118, 118, 134),
    }
}
