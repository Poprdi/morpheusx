pub mod lifecycle;
pub mod spawn;
pub mod state;
pub mod tick;
pub mod wait;
pub mod wake;

pub use lifecycle::{
    exit_process, idle_tsc_total, inc_timed_block_count, init_scheduler, mark_kernel_hlt,
    set_tsc_frequency, spawn_kernel_thread, tsc_frequency,
};
pub use spawn::{spawn_user_process, spawn_user_thread};
pub use state::{
    get_earliest_deadline, get_kernel_cr3, try_set_earliest_deadline, ProcessInfo,
    Scheduler, SchedulerCoreState, SchedulerDebugInfo, SchedulerSystemState, SCHEDULER,
};
pub use tick::scheduler_tick;
pub use wait::{block_sleep, try_wait_child, wait_for_child};
pub use wake::{wake_futex_waiters, wake_input_reader, wake_pipe_readers, wake_stdin_waiters};

pub(crate) use state::{PROCESS_TABLE, PROCESS_TABLE_LOCK};
pub(crate) use state::{
    clear_input_waiter, mark_futex_waiter, mark_input_waiter, mark_pipe_waiter,
    mark_stdin_waiter,
};
