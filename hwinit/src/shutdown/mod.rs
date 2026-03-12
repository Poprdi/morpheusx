pub mod handlers;
pub mod prepare;

pub use prepare::{
    register_poweroff_handler, register_prepare_handler, register_restart_handler,
    run_poweroff_handlers, run_prepare_handlers, run_restart_handlers, TransitionKind,
};

static SHUTDOWN_INIT: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

pub fn ensure_initialized() {
    if SHUTDOWN_INIT
        .compare_exchange(
            false,
            true,
            core::sync::atomic::Ordering::AcqRel,
            core::sync::atomic::Ordering::Acquire,
        )
        .is_ok()
    {
        handlers::register_builtin_handlers();
    }
}
