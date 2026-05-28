use crate::hal;
use crate::sync::RawSpinLock;

const MAX_PREPARE_HANDLERS: usize = 8;
const MAX_FINAL_HANDLERS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TransitionKind {
    RebootGraceful,
    RebootForce,
    ShutdownGraceful,
    ShutdownForce,
}

pub type PrepareHandler = fn(kind: TransitionKind) -> bool;
pub type FinalHandler = fn(kind: TransitionKind);

static SHUTDOWN_HANDLER_LOCK: RawSpinLock = RawSpinLock::new();

static mut PREPARE_HANDLERS: [Option<PrepareHandler>; MAX_PREPARE_HANDLERS] =
    [None; MAX_PREPARE_HANDLERS];
static mut RESTART_HANDLERS: [Option<FinalHandler>; MAX_FINAL_HANDLERS] =
    [None; MAX_FINAL_HANDLERS];
static mut POWEROFF_HANDLERS: [Option<FinalHandler>; MAX_FINAL_HANDLERS] =
    [None; MAX_FINAL_HANDLERS];

fn register_in_table<T: Copy + PartialEq>(table: &mut [Option<T>], handler: T) {
    for existing in table.iter_mut().flatten() {
        if *existing == handler {
            return;
        }
    }
    for slot in table.iter_mut() {
        if slot.is_none() {
            *slot = Some(handler);
            return;
        }
    }
}

pub fn register_prepare_handler(handler: PrepareHandler) {
    SHUTDOWN_HANDLER_LOCK.lock();
    unsafe {
        register_in_table(&mut PREPARE_HANDLERS, handler);
    }
    SHUTDOWN_HANDLER_LOCK.unlock();
}

pub fn register_restart_handler(handler: FinalHandler) {
    SHUTDOWN_HANDLER_LOCK.lock();
    unsafe {
        register_in_table(&mut RESTART_HANDLERS, handler);
    }
    SHUTDOWN_HANDLER_LOCK.unlock();
}

pub fn register_poweroff_handler(handler: FinalHandler) {
    SHUTDOWN_HANDLER_LOCK.lock();
    unsafe {
        register_in_table(&mut POWEROFF_HANDLERS, handler);
    }
    SHUTDOWN_HANDLER_LOCK.unlock();
}

fn deadline_from_timeout_ms(timeout_ms: u64) -> Option<u64> {
    let tsc_hz = hal().timer().tsc_frequency();
    if tsc_hz == 0 {
        return None;
    }
    let ticks_per_ms = (tsc_hz / 1000).max(1);
    Some(
        hal()
            .timer()
            .read_tsc()
            .saturating_add(timeout_ms.saturating_mul(ticks_per_ms)),
    )
}

pub fn run_prepare_handlers(kind: TransitionKind, timeout_ms: u64) -> bool {
    let deadline = deadline_from_timeout_ms(timeout_ms);

    let mut all_ok = true;

    SHUTDOWN_HANDLER_LOCK.lock();
    unsafe {
        for handler in PREPARE_HANDLERS.iter().flatten() {
            if let Some(d) = deadline {
                if hal().timer().read_tsc() >= d {
                    all_ok = false;
                    hal().serial().puts("shutdown-prepare-timeout\n");
                    break;
                }
            }
            let ok = handler(kind);
            if !ok {
                all_ok = false;
            }
        }
    }
    SHUTDOWN_HANDLER_LOCK.unlock();

    all_ok
}

pub fn run_restart_handlers(kind: TransitionKind) {
    SHUTDOWN_HANDLER_LOCK.lock();
    unsafe {
        for handler in RESTART_HANDLERS.iter().flatten() {
            handler(kind);
        }
    }
    SHUTDOWN_HANDLER_LOCK.unlock();
}

pub fn run_poweroff_handlers(kind: TransitionKind) {
    SHUTDOWN_HANDLER_LOCK.lock();
    unsafe {
        for handler in POWEROFF_HANDLERS.iter().flatten() {
            handler(kind);
        }
    }
    SHUTDOWN_HANDLER_LOCK.unlock();
}
