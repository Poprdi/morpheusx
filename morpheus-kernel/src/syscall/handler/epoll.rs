// epoll-like readiness multiplexing over unified fds with true kernel blocking.
//
// An epoll instance is itself an fd (FdKind::Epoll); its watch set lives in
// EPOLL_TABLE keyed by the instance id in the fd cookie. Readiness comes from
// io::readiness as a level-triggered EPOLL* mask (sockets/pipes/nested epoll;
// regular files are not pollable). The park primitive signals one token only, so
// epoll_wait re-scans member masks on each wake rather than via per-source
// callbacks; EPOLLET is layered on by diffing each watch's live vs last mask.

use super::common::*;
use crate::hal;
use crate::io::readiness::{
    epoll_token, pipe_token, ready_mask, register, socket_token, unregister, wait_ready,
};
use crate::schedular::{tsc_frequency, SCHEDULER};
use crate::storage::fs_api::{FdKind, FdState};
use crate::sync::SpinLock;
use morpheus_foundation::errno::EEXIST;
use morpheus_foundation::flags::{
    EPOLLERR, EPOLLET, EPOLLHUP, EPOLLIN, EPOLLONESHOT, EPOLLOUT, EPOLLPRI, EPOLLRDHUP,
    EPOLL_CLOEXEC, EPOLL_CTL_ADD, EPOLL_CTL_DEL, EPOLL_CTL_MOD,
};
use morpheus_foundation::types::EpollEvent;

const MAX_EPOLL_INSTANCES: usize = 64;
/// Per-instance watch capacity — matches the 64-slot fd table ceiling, so a
/// process can register every fd it can hold.
const MAX_EPOLL_WATCHES: usize = 64;
/// Upper bound on `maxevents` to keep the user-buffer size check from overflowing
/// and to cap a single wait's copy-out work.
const MAX_WAIT_EVENTS: u64 = 1024;

/// EPOLL* bits that name readiness (vs. the EPOLLET/EPOLLONESHOT control bits).
/// ERR/HUP are reported unconditionally per Linux, so they are folded into every
/// watch's interest regardless of what the caller requested.
const READINESS_BITS: u32 = EPOLLIN | EPOLLOUT | EPOLLPRI | EPOLLRDHUP;
const ALWAYS_BITS: u32 = EPOLLERR | EPOLLHUP;

#[derive(Clone, Copy)]
struct Watch {
    in_use: bool,
    fd: u32,
    token: u64,
    /// Requested readiness bits, ALWAYS_BITS folded in.
    interest: u32,
    data: u64,
    edge: bool,
    oneshot: bool,
    /// EPOLLONESHOT: set once delivered, silences the watch until a MOD re-arms it.
    disabled: bool,
    /// EPOLLET: last mask delivered, so only newly-set bits fire next time.
    last_mask: u32,
}

impl Watch {
    const fn empty() -> Self {
        Self {
            in_use: false,
            fd: 0,
            token: 0,
            interest: 0,
            data: 0,
            edge: false,
            oneshot: false,
            disabled: false,
            last_mask: 0,
        }
    }
}

struct Instance {
    in_use: bool,
    watches: [Watch; MAX_EPOLL_WATCHES],
}

impl Instance {
    const fn empty() -> Self {
        Self {
            in_use: false,
            watches: [Watch::empty(); MAX_EPOLL_WATCHES],
        }
    }
}

static EPOLL_TABLE: SpinLock<[Instance; MAX_EPOLL_INSTANCES]> =
    SpinLock::new([const { Instance::empty() }; MAX_EPOLL_INSTANCES]);

/// Instance id <-> slot is 1-based so a zero cookie can never alias instance 0.
#[inline]
fn id_to_slot(id: u64) -> Option<usize> {
    if id == 0 {
        return None;
    }
    let i = (id - 1) as usize;
    (i < MAX_EPOLL_INSTANCES).then_some(i)
}

/// Read the instance id an epoll fd carries in its cookie low 8 bytes.
#[inline]
fn instance_id(desc: &FdState) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&desc.cookie[..8]);
    u64::from_ne_bytes(b)
}

/// Readiness token for a pollable fd, or `None` for a non-pollable (regular) fd.
fn fd_token(desc: &FdState) -> Option<u64> {
    match desc.kind {
        FdKind::Socket => Some(socket_token(desc.socket_cookie())),
        // Pipe ends stash their pipe index in `mount_id` (see ipc::sys_pipe).
        FdKind::Pipe => Some(pipe_token(desc.mount_id as u8)),
        FdKind::Epoll => Some(epoll_token(instance_id(desc))),
        FdKind::Regular => None,
    }
}

/// Close-path entry: reclaim the instance an epoll fd refers to.
pub fn destroy_for(desc: &FdState) {
    destroy(instance_id(desc));
}

/// Drop an instance's watch set and readiness slot (fd close path).
pub fn destroy(id: u64) {
    if let Some(slot) = id_to_slot(id) {
        let mut t = EPOLL_TABLE.lock();
        t[slot] = Instance::empty();
    }
    unregister(epoll_token(id));
}

/// SYS_EPOLL_CREATE: `flags -> epfd | -errno`. The epoll instance is itself a
/// unified fd; `EPOLL_CLOEXEC` sets FD_CLOEXEC on it.
pub unsafe fn sys_epoll_create(flags: u64) -> u64 {
    let id = {
        let mut t = EPOLL_TABLE.lock();
        match t.iter().position(|i| !i.in_use) {
            Some(slot) => {
                t[slot] = Instance::empty();
                t[slot].in_use = true;
                slot as u64 + 1
            },
            None => return EMFILE,
        }
    };

    let fd_table = SCHEDULER.current_fd_table_mut();
    let fd = match fd_table.alloc() {
        Some(fd) => fd,
        None => {
            destroy(id);
            return EMFILE;
        },
    };

    let mut st = FdState::empty();
    st.kind = FdKind::Epoll;
    st.cloexec = flags & EPOLL_CLOEXEC != 0;
    st.cookie[..8].copy_from_slice(&id.to_ne_bytes());
    if !fd_table.set(fd, st) {
        destroy(id);
        return EMFILE;
    }
    register(epoll_token(id));
    fd as u64
}

/// SYS_EPOLL_CTL: `epfd,op,fd,*const EpollEvent -> 0 | -errno`.
pub unsafe fn sys_epoll_ctl(epfd: u64, op: u64, fd: u64, event: u64) -> u64 {
    if epfd == fd {
        return EINVAL;
    }

    let fd_table = SCHEDULER.current_fd_table_mut();

    let id = match fd_table.get(epfd as usize) {
        Some(d) if d.kind == FdKind::Epoll => instance_id(d),
        Some(_) => return EINVAL,
        None => return EBADF,
    };

    let token = match fd_table.get(fd as usize) {
        // Regular files are not pollable (Linux returns EPERM for them).
        Some(d) => match fd_token(d) {
            Some(t) => t,
            None => return EPERM,
        },
        None => return EBADF,
    };

    // EPOLL_CTL_DEL ignores the event pointer; ADD/MOD require a readable one.
    let ev = if op == EPOLL_CTL_DEL {
        EpollEvent::default()
    } else {
        if !validate_user_buf(event, core::mem::size_of::<EpollEvent>() as u64) {
            return EFAULT;
        }
        *(event as *const EpollEvent)
    };

    let slot = match id_to_slot(id) {
        Some(s) => s,
        None => return EBADF,
    };
    let mut t = EPOLL_TABLE.lock();
    let inst = &mut t[slot];
    if !inst.in_use {
        return EBADF;
    }

    let existing = inst
        .watches
        .iter()
        .position(|w| w.in_use && w.fd == fd as u32);

    match op {
        EPOLL_CTL_ADD => {
            if existing.is_some() {
                return EEXIST;
            }
            let free = match inst.watches.iter().position(|w| !w.in_use) {
                Some(i) => i,
                None => return ENOMEM,
            };
            inst.watches[free] = Watch {
                in_use: true,
                fd: fd as u32,
                token,
                interest: (ev.events & READINESS_BITS) | ALWAYS_BITS,
                data: ev.data,
                edge: ev.events & EPOLLET != 0,
                oneshot: ev.events & EPOLLONESHOT != 0,
                disabled: false,
                last_mask: 0,
            };
            0
        },
        EPOLL_CTL_MOD => {
            let i = match existing {
                Some(i) => i,
                None => return ENOENT,
            };
            let w = &mut inst.watches[i];
            w.interest = (ev.events & READINESS_BITS) | ALWAYS_BITS;
            w.data = ev.data;
            w.edge = ev.events & EPOLLET != 0;
            w.oneshot = ev.events & EPOLLONESHOT != 0;
            // MOD re-arms a oneshot and resets the edge baseline so currently-ready
            // state is reported afresh.
            w.disabled = false;
            w.last_mask = 0;
            0
        },
        EPOLL_CTL_DEL => match existing {
            Some(i) => {
                inst.watches[i] = Watch::empty();
                0
            },
            None => ENOENT,
        },
        _ => EINVAL,
    }
}

/// One non-blocking scan of an instance's watches, writing up to `maxevents` ready
/// events into the validated `out` buffer. Returns the count produced. Holds the
/// table lock only for the scan; never across a park.
unsafe fn scan(slot: usize, out: *mut EpollEvent, maxevents: usize) -> usize {
    let mut count = 0usize;
    let mut t = EPOLL_TABLE.lock();
    let inst = &mut t[slot];
    if !inst.in_use {
        return 0;
    }
    for w in inst.watches.iter_mut() {
        if count >= maxevents {
            break;
        }
        if !w.in_use || w.disabled {
            continue;
        }
        let live = ready_mask(w.token) & w.interest;
        let report = if w.edge {
            // Edge: only bits not already delivered. Track the live mask so a drop
            // to 0 re-arms the edge.
            let newly = live & !w.last_mask;
            w.last_mask = live;
            newly
        } else {
            live
        };
        if report != 0 {
            *out.add(count) = EpollEvent {
                events: report,
                _pad: 0,
                data: w.data,
            };
            count += 1;
            if w.oneshot {
                w.disabled = true;
            }
        }
    }
    count
}

/// Park the caller until the instance's aggregate token is signalled or `deadline`
/// passes. The token's mask is never set by member backends, so this is a bounded
/// sleep used to pace member re-scans (no busy spin: the CPU halts in wait_ready).
unsafe fn wait_for_ready(id: u64, deadline: u64) {
    let _ = wait_ready(epoll_token(id), u32::MAX, deadline);
}

/// SYS_EPOLL_WAIT: `epfd,*mut EpollEvent,maxevents,timeout_ms -> nready | -errno`.
/// `timeout_ms < 0` (as i64) blocks forever, `0` returns immediately.
pub unsafe fn sys_epoll_wait(epfd: u64, events: u64, maxevents: u64, timeout_ms: u64) -> u64 {
    if maxevents == 0 || maxevents > MAX_WAIT_EVENTS {
        return EINVAL;
    }
    let bytes = maxevents.saturating_mul(core::mem::size_of::<EpollEvent>() as u64);
    if !validate_user_buf(events, bytes) {
        return EFAULT;
    }

    let id = {
        let fd_table = SCHEDULER.current_fd_table_mut();
        match fd_table.get(epfd as usize) {
            Some(d) if d.kind == FdKind::Epoll => instance_id(d),
            Some(_) => return EINVAL,
            None => return EBADF,
        }
    };
    let slot = match id_to_slot(id) {
        Some(s) => s,
        None => return EBADF,
    };

    let out = events as *mut EpollEvent;
    let max = maxevents as usize;

    let timeout = timeout_ms as i64;
    let freq = tsc_frequency();
    let now = hal().timer().read_tsc();
    // overall == 0 marks "block forever"; otherwise the absolute TSC expiry.
    let overall = if timeout < 0 || freq == 0 {
        0
    } else {
        now.saturating_add((timeout as u64).saturating_mul(freq) / 1000)
    };
    // Re-scan cadence while parked: member readiness wakes the source token, not
    // the epoll token, so we pace re-scans rather than relying on a callback.
    let slice = (freq / 500).max(1);

    loop {
        let n = scan(slot, out, max);
        if n > 0 {
            return n as u64;
        }
        if timeout == 0 {
            return 0;
        }
        let now = hal().timer().read_tsc();
        if overall != 0 && now >= overall {
            return 0;
        }
        let mut deadline = now.saturating_add(slice);
        if overall != 0 && overall < deadline {
            deadline = overall;
        }
        wait_for_ready(id, deadline);
    }
}
