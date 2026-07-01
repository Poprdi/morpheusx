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
//!
//! Global net-wake (K8): `epoll_wait` cannot park per-member (the block primitive
//! signals one token), so every epoll waiter parks on ONE process-global
//! [`net_wake_token`]. It is signalled by [`net_wake`] (a NIC RX IRQ — staged — or a
//! smoltcp-timer expiry) and, via the same lost-wakeup discipline, by every member
//! [`set_ready`]/[`replace_ready`] (so a socket becoming readable or a cross-thread
//! pipe write breaks the park). The wake is GLOBAL — an IRQ landing on another core
//! still wakes the parked waiter — and the [`NET_WAKE_BIT`] edge is published before
//! the lock-protected scan exactly like a member mask, so it is never lost.

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
const CLASS_NETWAKE: u64 = 4 << CLASS_SHIFT;
const ID_MASK: u64 = (1 << CLASS_SHIFT) - 1;

/// Private edge bit for the net-wake token. High bit so it never collides with the
/// low `EPOLL*` readiness bits; this token is never scanned as an epoll member.
const NET_WAKE_BIT: u32 = 1 << 31;

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

/// The single process-global token every `epoll_wait` parks on (K8). Signalled by
/// [`net_wake`] (NIC RX IRQ / stack timer) and by every member [`set_ready`].
#[inline]
pub fn net_wake_token() -> u64 {
    CLASS_NETWAKE
}

/// True if `token` names a source whose readiness only becomes visible after a
/// stack pump (socket = RX arrival) or a re-scan (nested epoll), so an `epoll_wait`
/// set containing it needs the bounded net re-poll. Pipe tokens return false: a pipe
/// write already breaks the park via the [`set_ready`] net-wake piggyback.
#[inline]
pub fn token_needs_repoll(token: u64) -> bool {
    let class = token & !ID_MASK;
    class == CLASS_SOCKET || class == CLASS_EPOLL
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

/// OR `add` into `token`'s mask and wake every thread parked on it — plus every
/// epoll waiter parked on the global net-wake token, since member readiness is one
/// of the things an `epoll_wait` park must break on. Both edges (the member OR and
/// [`NET_WAKE_BIT`]) are published before the single lock-protected wake scan (see
/// module lost-wakeup note), so neither wake is lost against a racing park.
pub fn set_ready(token: u64, add: u32) {
    let i = match slot_for(token) {
        Some(i) => i,
        None => return,
    };
    SOURCES[i].mask.fetch_or(add, Ordering::AcqRel);
    publish_net_wake();
    wake_tokens(token, net_wake_token());
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
        publish_net_wake();
        wake_tokens(token, net_wake_token());
    }
}

/// Publish the global net-wake edge into its slot mask BEFORE any lock-protected
/// wake scan, so an epoll waiter that re-reads it under `PROCESS_TABLE_LOCK` right
/// before parking can never miss it (mirrors the member-mask OR discipline).
#[inline]
fn publish_net_wake() {
    if let Some(ni) = slot_for(net_wake_token()) {
        SOURCES[ni].mask.fetch_or(NET_WAKE_BIT, Ordering::AcqRel);
    }
}

/// Wake all threads parked on `token`. Public so a backend can re-arm waiters
/// (e.g. epoll level re-notify) without changing the mask.
pub fn wake_token(token: u64) {
    wake_tokens(token, token);
}

/// One `PROCESS_TABLE_LOCK` scan waking every thread parked on `a` OR `b` (`a == b`
/// degenerates to a single token). Lets a member `set_ready` wake both the member's
/// own waiters and the global net-wake (epoll) waiters in a single pass.
fn wake_tokens(a: u64, b: u64) {
    // SAFETY: PROCESS_TABLE mutated only under its lock.
    unsafe {
        PROCESS_TABLE_LOCK.lock();
        for slot in PROCESS_TABLE.iter_mut() {
            if let Some(proc) = slot.as_mut() {
                if let ProcessState::Blocked(BlockReason::IoReady(t)) = proc.state {
                    if t == a || t == b {
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

/// Signal the global net-wake token: publish the edge, then wake every epoll waiter
/// parked on it. This is the entry point a NIC RX ISR (staged — see below) and any
/// stack-timer path calls.
///
/// SAFETY / IRQ discipline: this takes ONLY `PROCESS_TABLE_LOCK` (an
/// `IsrSafeRawSpinLock`, so an IRQ can never land on a core already holding it) and
/// touches NO smoltcp/NIC state, so it is safe to call from interrupt context and
/// cannot alias the `&mut USER_NET_STACK` an interrupted syscall thread may hold.
/// The wake is GLOBAL, so an RX IRQ delivered to a different core than the parked
/// `epoll_wait` still wakes it (SMP-correct).
///
/// STAGED — NIC RX IRQ (K8, disabled): `morpheus-nic` currently masks all e1000e
/// interrupts (IMS=0, polled) and no vector is installed, so `net_wake` is not yet
/// called from an ISR — the bounded `poll_at` re-poll in `epoll_wait` covers RX
/// meanwhile. To enable end-to-end (real-HW only; QEMU IRQ != Intel silicon): (1)
/// stash the NIC MMIO base in a raw `AtomicU64` at init (NOT via `&mut` NIC, to
/// avoid aliasing); (2) `enable_msi_single(dev, apic_id, vec)` + `set_handler(vec,
/// nic_isr, ..)` in the HAL; (3) unmask IMS RXT0 (0x80); (4) the ISR reads ICR
/// (read-to-clear) via the raw base, sends LAPIC EOI, and calls `net_wake()` — it
/// must NOT pump the stack. Then delete the `poll_at` re-poll cap in `epoll_wait`.
pub fn net_wake() {
    publish_net_wake();
    wake_token(net_wake_token());
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

pub unsafe fn wait_net_wake(deadline_tsc: u64) -> WaitOutcome {
    wait_ready(net_wake_token(), NET_WAKE_BIT, deadline_tsc)
}

pub fn net_wake_reset() {
    clear_ready(net_wake_token(), NET_WAKE_BIT);
}

/// Convenience mask for "either side of the connection can make progress".
pub const READ_WRITE: u32 = EPOLLIN | EPOLLOUT;
/// Error/close conditions epoll always reports regardless of interest.
pub const ERR_HUP: u32 = EPOLLERR | EPOLLHUP | EPOLLRDHUP;
