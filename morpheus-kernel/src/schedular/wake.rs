use super::state::{
    clear_futex_waiter, clear_input_waiter, clear_pipe_waiter, clear_stdin_waiter, futex_waiters,
    input_waiters, pipe_waiters, stdin_waiters, PROCESS_TABLE, PROCESS_TABLE_LOCK,
    TIMED_BLOCK_COUNT,
};
use crate::process::{BlockReason, ProcessState};
use core::sync::atomic::Ordering;

pub unsafe fn wake_stdin_waiters() {
    if stdin_waiters().is_empty() {
        return;
    }
    PROCESS_TABLE_LOCK.lock();
    stdin_waiters().for_each(|bit| {
        if let Some(Some(proc)) = PROCESS_TABLE.get_mut(bit as usize) {
            if matches!(proc.state, ProcessState::Blocked(BlockReason::StdinRead)) {
                proc.state = ProcessState::Ready;
            }
        }
        clear_stdin_waiter(bit);
    });
    PROCESS_TABLE_LOCK.unlock();
}

pub unsafe fn wake_pipe_readers(pipe_idx: u8) {
    if pipe_waiters(pipe_idx).is_empty() {
        return;
    }
    PROCESS_TABLE_LOCK.lock();
    pipe_waiters(pipe_idx).for_each(|bit| {
        if let Some(Some(proc)) = PROCESS_TABLE.get_mut(bit as usize) {
            if let ProcessState::Blocked(BlockReason::PipeRead(idx)) = proc.state {
                if idx == pipe_idx {
                    proc.state = ProcessState::Ready;
                }
            }
        }
        clear_pipe_waiter(bit, pipe_idx);
    });
    PROCESS_TABLE_LOCK.unlock();
}

pub unsafe fn wake_input_reader(pid: u32) {
    if !input_waiters().contains(pid) {
        return;
    }
    PROCESS_TABLE_LOCK.lock();
    if let Some(Some(proc)) = PROCESS_TABLE.get_mut(pid as usize) {
        if matches!(proc.state, ProcessState::Blocked(BlockReason::InputRead)) {
            proc.state = ProcessState::Ready;
        }
    }
    clear_input_waiter(pid);
    PROCESS_TABLE_LOCK.unlock();
}

/// Wake up to `count` futex waiters parked on `addr`. The compare-and-enqueue in
/// `sys_futex(FUTEX_WAIT)` runs under PROCESS_TABLE_LOCK and the waker's userspace
/// store happens-before this call, so taking the lock unconditionally (NO
/// lock-free early-out on the waiter set) is what closes the SMP lost-wakeup: a
/// waiter that has not yet enqueued will, once it acquires the lock, re-read the
/// futex word the waker already changed and bail with EAGAIN instead of sleeping.
pub unsafe fn wake_futex_waiters(addr: u64, count: u32) -> u32 {
    PROCESS_TABLE_LOCK.lock();
    let mut woken = 0u32;
    futex_waiters(addr).for_each(|bit| {
        if woken >= count {
            return;
        }
        let mut matched = false;
        if let Some(Some(proc)) = PROCESS_TABLE.get_mut(bit as usize) {
            if let ProcessState::Blocked(BlockReason::FutexWait(wait_addr)) = proc.state {
                if wait_addr == addr {
                    if proc.futex_deadline != 0 {
                        proc.futex_deadline = 0;
                        TIMED_BLOCK_COUNT.fetch_sub(1, Ordering::Relaxed);
                    }
                    proc.state = ProcessState::Ready;
                    woken += 1;
                    matched = true;
                }
            }
        }
        if matched
            || PROCESS_TABLE
                .get(bit as usize)
                .map_or(true, |s| s.is_none())
        {
            clear_futex_waiter(bit, addr);
        }
    });
    PROCESS_TABLE_LOCK.unlock();
    woken
}
