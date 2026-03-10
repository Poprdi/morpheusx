use super::state::{
    clear_futex_waiter, clear_input_waiter, clear_pipe_waiter, clear_stdin_waiter,
    futex_waiters_mask, input_waiters_mask, pipe_waiters_mask, stdin_waiters_mask, PROCESS_TABLE,
    PROCESS_TABLE_LOCK, TIMED_BLOCK_COUNT,
};
use crate::process::{BlockReason, ProcessState};
use core::sync::atomic::Ordering;

pub unsafe fn wake_stdin_waiters() {
    let mut waiters = stdin_waiters_mask();
    if waiters == 0 {
        return;
    }
    PROCESS_TABLE_LOCK.lock();
    while waiters != 0 {
        let bit = waiters.trailing_zeros() as u32;
        waiters &= waiters - 1;
        if let Some(Some(proc)) = PROCESS_TABLE.get_mut(bit as usize) {
            if matches!(proc.state, ProcessState::Blocked(BlockReason::StdinRead)) {
                proc.state = ProcessState::Ready;
                clear_stdin_waiter(bit);
            } else {
                clear_stdin_waiter(bit);
            }
        } else {
            clear_stdin_waiter(bit);
        }
    }
    PROCESS_TABLE_LOCK.unlock();
}

pub unsafe fn wake_pipe_readers(pipe_idx: u8) {
    let mut waiters = pipe_waiters_mask(pipe_idx);
    if waiters == 0 {
        return;
    }
    PROCESS_TABLE_LOCK.lock();
    while waiters != 0 {
        let bit = waiters.trailing_zeros() as u32;
        waiters &= waiters - 1;
        if let Some(Some(proc)) = PROCESS_TABLE.get_mut(bit as usize) {
            if let ProcessState::Blocked(BlockReason::PipeRead(idx)) = proc.state {
                if idx == pipe_idx {
                    proc.state = ProcessState::Ready;
                    clear_pipe_waiter(bit, pipe_idx);
                }
            } else {
                clear_pipe_waiter(bit, pipe_idx);
            }
        } else {
            clear_pipe_waiter(bit, pipe_idx);
        }
    }
    PROCESS_TABLE_LOCK.unlock();
}

pub unsafe fn wake_input_reader(pid: u32) {
    if input_waiters_mask() & (1u64 << (pid & 63)) == 0 {
        return;
    }
    PROCESS_TABLE_LOCK.lock();
    if let Some(Some(proc)) = PROCESS_TABLE.get_mut(pid as usize) {
        if matches!(proc.state, ProcessState::Blocked(BlockReason::InputRead)) {
            proc.state = ProcessState::Ready;
            clear_input_waiter(pid);
        } else {
            clear_input_waiter(pid);
        }
    } else {
        clear_input_waiter(pid);
    }
    PROCESS_TABLE_LOCK.unlock();
}

pub unsafe fn wake_futex_waiters(addr: u64, count: u32) -> u32 {
    let mut waiters = futex_waiters_mask(addr);
    if waiters == 0 {
        return 0;
    }
    PROCESS_TABLE_LOCK.lock();
    let mut woken = 0u32;
    while waiters != 0 {
        if woken >= count {
            break;
        }
        let bit = waiters.trailing_zeros() as u32;
        waiters &= waiters - 1;
        if let Some(Some(proc)) = PROCESS_TABLE.get_mut(bit as usize) {
            if let ProcessState::Blocked(BlockReason::FutexWait(wait_addr)) = proc.state {
                if wait_addr == addr {
                    if proc.futex_deadline != 0 {
                        proc.futex_deadline = 0;
                        TIMED_BLOCK_COUNT.fetch_sub(1, Ordering::Relaxed);
                    }
                    proc.state = ProcessState::Ready;
                    clear_futex_waiter(bit, addr);
                    woken += 1;
                }
            } else {
                clear_futex_waiter(bit, addr);
            }
        } else {
            clear_futex_waiter(bit, addr);
        }
    }
    PROCESS_TABLE_LOCK.unlock();
    woken
}
