//! Per-object readiness + a true kernel blocking primitive (no busy-poll).
//!
//! A pollable object (socket/pipe end, epoll instance) is named by a stable `u64`
//! token (see [`socket_token`]/[`pipe_token`]/[`epoll_token`]) carrying a
//! level-triggered `EPOLL*` mask. Backends [`set_ready`]/[`clear_ready`]; readers
//! [`ready_mask`] for `epoll_wait`/`poll` or [`wait_ready`] to park.
//!
//! Locking discipline closing the lost-wakeup hole without a lock-order cycle: the
//! per-token mask is a lock-free `AtomicU32`. [`set_ready`] does the atomic OR
//! (globally visible) BEFORE taking `PROCESS_TABLE_LOCK` to wake; [`wait_ready`]
//! re-reads the mask UNDER `PROCESS_TABLE_LOCK` before parking. So a wakeup racing
//! a park is never lost: either the parker observes the OR and skips sleeping, or
//! it has already parked when the waker's lock-protected scan runs.

use crate::hal;
use crate::process::{BlockReason, ProcessState};
use crate::schedular::{
    dec_timed_block_count, get_earliest_deadline, inc_timed_block_count, try_set_earliest_deadline,
    PROCESS_TABLE, PROCESS_TABLE_LOCK, SCHEDULER,
};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use morpheus_foundation::flags::{EPOLLERR, EPOLLHUP, EPOLLIN, EPOLLOUT, EPOLLRDHUP};

/// Concurrently-pollable backend objects; sized past the 256-task / 64-fd envelope.
pub const MAX_READINESS_SOURCES: usize = 1024;

/// Token namespaces — keep socket/pipe/epoll ids from aliasing each other.
const CLASS_SHIFT: u64 = 56;
const CLASS_SOCKET: u64 = 1 << CLASS_SHIFT;
const CLASS_PIPE: u64 = 2 << CLASS_SHIFT;
const CLASS_EPOLL: u64 = 3 << CLASS_SHIFT;
const ID_MASK: u64 = (1 << CLASS_SHIFT) - 1;

#[inline]
pub fn socket_token(handle: u64) -> u64 {
    CLASS_SOCKET | (handle & ID_MASK)
}

#[inline]
pub fn pipe_token(pipe_idx: u8) -> u64 {
    CLASS_PIPE | pipe_idx as u64
}

#[inline]
pub fn epoll_token(epfd_cookie: u64) -> u64 {
    CLASS_EPOLL | (epfd_cookie & ID_MASK)
}

struct Source {
    /// 0 = free slot. Non-zero = the owning backend's token.
    token: AtomicU64,
    mask: AtomicU32,
}

#[allow(clippy::declare_interior_mutable_const)]
const SOURCE_INIT: Source = Source {
    token: AtomicU64::new(0),
    mask: AtomicU32::new(0),
};

static SOURCES: [Source; MAX_READINESS_SOURCES] = [SOURCE_INIT; MAX_READINESS_SOURCES];

/// Guards ONLY slot (de)allocation — the find/alloc/free races. Mask reads/writes
/// stay lock-free so the park path never nests this under `PROCESS_TABLE_LOCK`.
use crate::sync::RawSpinLock;
static REG_LOCK: RawSpinLock = RawSpinLock::new();

fn find(token: u64) -> Option<usize> {
    SOURCES
        .iter()
        .position(|s| s.token.load(Ordering::Acquire) == token)
}

/// Create or find `token`'s readiness slot (idempotent); `None` if the table is full.
pub fn register(token: u64) -> Option<usize> {
    if token == 0 {
        return None;
    }
    REG_LOCK.lock();
    let r = match find(token) {
        Some(i) => Some(i),
        None => match SOURCES
            .iter()
            .position(|s| s.token.load(Ordering::Relaxed) == 0)
        {
            Some(i) => {
                SOURCES[i].mask.store(0, Ordering::Relaxed);
                SOURCES[i].token.store(token, Ordering::Release);
                Some(i)
            },
            None => None,
        },
    };
    REG_LOCK.unlock();
    r
}

/// Drop `token`'s slot at object teardown. Parked waiters should be woken first
/// (e.g. `set_ready(token, EPOLLHUP|EPOLLERR)`), else they keep their stale park.
pub fn unregister(token: u64) {
    REG_LOCK.lock();
    if let Some(i) = find(token) {
        SOURCES[i].mask.store(0, Ordering::Relaxed);
        SOURCES[i].token.store(0, Ordering::Release);
    }
    REG_LOCK.unlock();
}

fn slot_for(token: u64) -> Option<usize> {
    find(token).or_else(|| register(token))
}

/// Current level-triggered readiness for `token` (0 if unregistered).
pub fn ready_mask(token: u64) -> u32 {
    match find(token) {
        Some(i) => SOURCES[i].mask.load(Ordering::Acquire),
        None => 0,
    }
}

/// OR `add` into `token`'s mask and wake every thread parked on it. The OR is
/// published before the lock-protected wake scan (see module lost-wakeup note).
pub fn set_ready(token: u64, add: u32) {
    let i = match slot_for(token) {
        Some(i) => i,
        None => return,
    };
    SOURCES[i].mask.fetch_or(add, Ordering::AcqRel);
    wake_token(token);
}

/// Clear `sub` from `token`'s mask — a level→edge transition, e.g. after a
/// non-blocking `recv` returns `EWOULDBLOCK` the socket layer clears `EPOLLIN`.
pub fn clear_ready(token: u64, sub: u32) {
    if let Some(i) = find(token) {
        SOURCES[i].mask.fetch_and(!sub, Ordering::AcqRel);
    }
}

/// Overwrite `token`'s mask wholesale (backend recomputed its state).
pub fn replace_ready(token: u64, mask: u32) {
    if let Some(i) = slot_for(token) {
        SOURCES[i].mask.store(mask, Ordering::Release);
        wake_token(token);
    }
}

/// Wake all threads parked on `token`. Public so a backend can re-arm waiters
/// (e.g. epoll level re-notify) without changing the mask.
pub fn wake_token(token: u64) {
    // SAFETY: PROCESS_TABLE mutated only under its lock.
    unsafe {
        PROCESS_TABLE_LOCK.lock();
        for slot in PROCESS_TABLE.iter_mut() {
            if let Some(proc) = slot.as_mut() {
                if let ProcessState::Blocked(BlockReason::IoReady(t)) = proc.state {
                    if t == token {
                        if proc.futex_deadline != 0 {
                            proc.futex_deadline = 0;
                            dec_timed_block_count();
                        }
                        proc.state = ProcessState::Ready;
                    }
                }
            }
        }
        PROCESS_TABLE_LOCK.unlock();
    }
}

/// Outcome of [`wait_ready`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WaitOutcome {
    /// `interest & current_mask`, non-zero — the matched ready bits.
    Ready(u32),
    /// Deadline expired with none of `interest` set.
    TimedOut,
}

/// Park the calling thread until `interest` intersects `token`'s readiness, or
/// `deadline_tsc` passes (0 = forever). LEVEL-triggered: every wakeup re-checks the
/// live mask, so a spurious wake just re-parks (epoll builds edge-triggering on top).
///
/// # Safety
/// Runs on the current thread's syscall stack; must be the active task.
pub unsafe fn wait_ready(token: u64, interest: u32, deadline_tsc: u64) -> WaitOutcome {
    slot_for(token);
    loop {
        PROCESS_TABLE_LOCK.lock();
        let m = ready_mask(token) & interest;
        if m != 0 {
            PROCESS_TABLE_LOCK.unlock();
            return WaitOutcome::Ready(m);
        }
        let pid = SCHEDULER.current_pid();
        if let Some(Some(proc)) = PROCESS_TABLE.get_mut(pid as usize) {
            proc.futex_timed_out = false;
            proc.state = ProcessState::Blocked(BlockReason::IoReady(token));
            if deadline_tsc != 0 {
                proc.futex_deadline = deadline_tsc;
                inc_timed_block_count();
                loop {
                    let earliest = get_earliest_deadline();
                    if deadline_tsc < earliest {
                        if try_set_earliest_deadline(earliest, deadline_tsc) {
                            break;
                        }
                    } else {
                        break;
                    }
                }
            }
        }
        PROCESS_TABLE_LOCK.unlock();

        hal().cpu().halt_wait_irq();

        let proc = SCHEDULER.current_process_mut();
        if proc.futex_timed_out {
            proc.futex_timed_out = false;
            return WaitOutcome::TimedOut;
        }
    }
}

/// Convenience mask for "either side of the connection can make progress".
pub const READ_WRITE: u32 = EPOLLIN | EPOLLOUT;
/// Error/close conditions epoll always reports regardless of interest.
pub const ERR_HUP: u32 = EPOLLERR | EPOLLHUP | EPOLLRDHUP;
