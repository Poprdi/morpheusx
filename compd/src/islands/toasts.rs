//! Notification toasts — compd's binding to the `whisperd` feed.
//! compd renders `whisperd.feed` as a top-right toast stack and writes click-dismisses to
//! `whisperd.dismiss`; all lifetime/policy stays in whisperd.

use alloc::vec::Vec;

use phosphor_notify::layout::{stack_layout, toast_height, toast_hit, wrap_lines, Rect};
use phosphor_notify::{Notification, ToastMetrics, Urgency};

use crate::islands::CompState;

/// MUST match `platform_morpheusx::whisperd::IPC_KEY_NOTIFY_FEED` (cross-process byte contract).
const FEED_KEY: &str = "whisperd.feed";
/// MUST match `platform_morpheusx::whisperd::IPC_KEY_NOTIFY_DISMISS`.
const DISMISS_KEY: &str = "whisperd.dismiss";

/// Feed read buffer; an over-long feed truncates and fails to decode, leaving the last good set.
const FEED_BUF: usize = 4096;

/// Toast geometry: 300px wide (35 chars at 8px), 2px leading, top-right margin 12px.
pub const METRICS: ToastMetrics = ToastMetrics {
    width: 300,
    pad: 12,
    line_h: 18,
    gap: 8,
    margin: 12,
    close: 12,
    max_visible: 5,
};

/// Left accent stripe width; shared with the renderer so stripe and text inset agree.
pub const ACCENT_W: i32 = 4;

#[inline]
fn max_text_chars() -> usize {
    (((METRICS.width - 2 * METRICS.pad - ACCENT_W) / 8).max(1)) as usize
}

/// Body text wrapped to toast width; shared by height calculation and draw so the box matches.
pub fn body_lines(n: &Notification) -> Vec<alloc::string::String> {
    wrap_lines(&n.body, max_text_chars())
}

fn line_count(n: &Notification) -> usize {
    let head = usize::from(!n.app.is_empty());
    head + 1 + body_lines(n).len()
}

/// `true` if `id` is in the local dismiss ring (suppressed until whisperd drops it from the feed).
fn is_dismissed(state: &CompState, id: u32) -> bool {
    id != 0 && state.toast_dismissed.contains(&id)
}

/// Re-decode `whisperd.feed` only when bytes change; on decode failure keep the last good set.
pub fn consume_feed(state: &mut CompState) {
    let mut buf = [0u8; FEED_BUF];
    let bytes: &[u8] = match libmorpheus::persist::get(FEED_KEY, &mut buf) {
        Ok(n) => &buf[..n],
        Err(_) => &[], // absent ⇒ empty feed.
    };
    if bytes == state.toast_feed_raw.as_slice() {
        return; // unchanged since last frame — nothing to do.
    }
    // Cache raw bytes even on failure so we don't re-decode the same bad blob each frame.
    state.toast_feed_raw.clear();
    state.toast_feed_raw.extend_from_slice(bytes);

    if bytes.is_empty() {
        state.toasts.clear();
    } else if let Some(items) = phosphor_notify::wire::decode_feed(bytes) {
        state.toasts = items;
    }
    prune_ring(state);
}

/// Remove dismiss-ring entries no longer in the feed so reused ids (after whisperd restart) show normally.
fn prune_ring(state: &mut CompState) {
    for k in 0..crate::islands::TOAST_DISMISS_RING {
        let id = state.toast_dismissed[k];
        if id != 0 && !state.toasts.iter().any(|n| n.id == id) {
            state.toast_dismissed[k] = 0;
        }
    }
}

/// Indices of visible toasts (dismiss-filtered, capped to `max_visible`) paired with their rects.
/// Shared by renderer and hit-test so drawn boxes == clickable boxes.
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

/// `true` if the pointer is over any toast (keeps the arrow cursor; prevents move/resize shape from beneath).
pub fn hovering(state: &CompState, mx: i32, my: i32) -> bool {
    let (_idxs, rects) = visible(state);
    toast_hit(&rects, mx, my).is_some()
}

/// Dismiss the toast at `(mx,my)` and return `true` (click consumed); `false` if no hit (non-modal).
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

/// Record `id` in the local ring and write a dismiss request to whisperd.
/// Token is monotonic clock, strictly increasing across restarts — whisperd services each once.
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

/// Urgency accent: red for Critical, theme focus for Normal, muted grey for Low.
pub fn accent_rgb(state: &CompState, urgency: Urgency) -> (u8, u8, u8) {
    match urgency {
        Urgency::Critical => (210, 86, 86),
        Urgency::Normal => state.border_focused_rgb,
        Urgency::Low => (118, 118, 134),
    }
}
