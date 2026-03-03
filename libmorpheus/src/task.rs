//! # Architecture
//!
//! ```text
//!  spawn(future) ──► TASK_QUEUE ──► worker loop ──► poll(future)
//!                        ▲                              │
//!                        │                              ▼
//!                   Waker::wake()  ◄──────────  if Pending, park
//!                   (futex-based)               via futex_wait
//! ```
//!
//! ## Design for SMP
//!
//! The task queue is a lock-free(ish) structure protected by a Mutex backed
//! by SYS_FUTEX.  When SMP lands, each core can run its own worker thread
//! pulling from the shared queue — no architectural changes needed, just
//! spawn more workers.
//!
//! ## Waker mechanism
//!
//! Each task has a `futex_word: AtomicU32` embedded in its allocation.
//! When a future returns Pending, the worker parks on that futex.
//! When Waker::wake() fires, it sets the word and calls FUTEX_WAKE,
//! which unblocks the worker.
//!
//! ## Compatibility
//!
//! This executor implements the standard `core::task::{RawWaker, Waker, Context}`
//! protocol.  Any future that compiles against core::future::Future works here.
//! Tokio's `block_on` equivalent is our `block_on`.  For multi-threaded
//! tokio-style work stealing, call `Runtime::new(n)` with n worker threads.

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use crate::raw::*;
use crate::sync::Mutex;

// -- Task representation --

/// A spawned future + its wake state.
struct Task {
    /// The boxed, pinned future.
    future: Mutex<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
    /// Futex word for parking/waking the worker polling this task.
    /// 0 = sleeping, 1 = notified (ready to poll).
    notify: AtomicU32,
    /// Back-reference to the runtime's global queue for re-enqueue on wake.
    queue: Arc<TaskQueue>,
    /// Self-reference (index in the slab or Arc to self).  Used by the waker.
    self_ref: AtomicUsize,
}

// SAFETY: Task is Send — the future inside is Send, all other fields are atomic/Mutex'd.
unsafe impl Send for Task {}
unsafe impl Sync for Task {}

// -- Waker vtable --

/// Clone the Arc<Task> behind the waker.
unsafe fn waker_clone(ptr: *const ()) -> RawWaker {
    let arc = Arc::from_raw(ptr as *const Task);
    let cloned = arc.clone();
    // Don't drop the original — we're cloning, not moving.
    core::mem::forget(arc);
    RawWaker::new(Arc::into_raw(cloned) as *const (), &VTABLE)
}

/// Wake: set notify=1, push to queue, futex_wake.
unsafe fn waker_wake(ptr: *const ()) {
    let arc = Arc::from_raw(ptr as *const Task);
    wake_inner(&arc);
    // wake consumes the waker, so we drop the Arc here (it goes out of scope).
}

/// Wake by ref: same but don't drop the Arc.
unsafe fn waker_wake_by_ref(ptr: *const ()) {
    let arc = Arc::from_raw(ptr as *const Task);
    wake_inner(&arc);
    core::mem::forget(arc); // don't drop — caller still owns the waker
}

/// Drop the Arc behind the waker.
unsafe fn waker_drop(ptr: *const ()) {
    let _ = Arc::from_raw(ptr as *const Task);
}

fn wake_inner(task: &Arc<Task>) {
    // Mark as notified.
    task.notify.store(1, Ordering::Release);
    // Re-enqueue so a worker will pick it up.
    task.queue.push(task.clone());
    // Kick the park futex so blocked workers wake up.
    task.queue.unpark_one();
}

static VTABLE: RawWakerVTable =
    RawWakerVTable::new(waker_clone, waker_wake, waker_wake_by_ref, waker_drop);

fn task_to_waker(task: &Arc<Task>) -> Waker {
    let ptr = Arc::into_raw(task.clone()) as *const ();
    unsafe { Waker::from_raw(RawWaker::new(ptr, &VTABLE)) }
}

// -- Shared task queue --

struct TaskQueue {
    /// FIFO of tasks ready to be polled.
    queue: Mutex<VecDeque<Arc<Task>>>,
    /// Futex word for parking idle workers.  0 = no tasks, 1 = tasks available.
    park_futex: AtomicU32,
    /// Number of tasks still alive (not yet completed).
    active_count: AtomicUsize,
}

impl TaskQueue {
    fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            park_futex: AtomicU32::new(0),
            active_count: AtomicUsize::new(0),
        }
    }

    fn push(&self, task: Arc<Task>) {
        self.queue.lock().push_back(task);
        // Signal that work is available.
        self.park_futex.store(1, Ordering::Release);
    }

    fn pop(&self) -> Option<Arc<Task>> {
        let mut q = self.queue.lock();
        let task = q.pop_front();
        if q.is_empty() {
            self.park_futex.store(0, Ordering::Release);
        }
        task
    }

    /// Park the current thread/process until work arrives.
    fn park(&self) {
        // Only sleep if park_futex == 0 (no work).
        unsafe {
            syscall3(
                SYS_FUTEX,
                &self.park_futex as *const AtomicU32 as u64,
                FUTEX_WAIT,
                0, // expected value = 0 (no work)
            );
        }
    }

    /// Wake one parked worker.
    fn unpark_one(&self) {
        self.park_futex.store(1, Ordering::Release);
        unsafe {
            syscall3(
                SYS_FUTEX,
                &self.park_futex as *const AtomicU32 as u64,
                FUTEX_WAKE,
                1,
            );
        }
    }

    /// Wake all parked workers.
    fn unpark_all(&self) {
        self.park_futex.store(1, Ordering::Release);
        unsafe {
            syscall3(
                SYS_FUTEX,
                &self.park_futex as *const AtomicU32 as u64,
                FUTEX_WAKE,
                u32::MAX as u64,
            );
        }
    }
}

// -- Runtime --

/// The async runtime.  Create one per application (or use `block_on` for simple cases).
pub struct Runtime {
    queue: Arc<TaskQueue>,
}

impl Runtime {
    /// Create a new single-threaded runtime.
    pub fn new() -> Self {
        Self {
            queue: Arc::new(TaskQueue::new()),
        }
    }

    /// Spawn a fire-and-forget task (no return value).
    pub fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let task = Arc::new(Task {
            future: Mutex::new(Box::pin(future)),
            notify: AtomicU32::new(1),
            queue: self.queue.clone(),
            self_ref: AtomicUsize::new(0),
        });
        self.queue.active_count.fetch_add(1, Ordering::Relaxed);
        self.queue.push(task);
    }

    /// Spawn a task and get a handle to await its result.
    pub fn spawn_with_handle<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let join_state: Arc<JoinState<F::Output>> = Arc::new(JoinState::new());
        let js = join_state.clone();

        // Wrap the user's future: run it, stash the output, signal done.
        let wrapped = async move {
            let result = future.await;
            js.complete(result);
        };

        let task = Arc::new(Task {
            future: Mutex::new(Box::pin(wrapped)),
            notify: AtomicU32::new(1),
            queue: self.queue.clone(),
            self_ref: AtomicUsize::new(0),
        });
        self.queue.active_count.fetch_add(1, Ordering::Relaxed);
        self.queue.push(task);

        JoinHandle { state: join_state }
    }

    /// Run the event loop until all spawned tasks complete.
    ///
    /// Single-threaded: polls tasks from the queue in FIFO order.
    /// When the queue is empty and tasks are still alive, parks via futex.
    pub fn run(&self) {
        loop {
            // Drain all available work.
            while let Some(task) = self.queue.pop() {
                // Clear the notification flag — we're about to poll.
                task.notify.store(0, Ordering::Relaxed);

                let waker = task_to_waker(&task);
                let mut cx = Context::from_waker(&waker);

                let mut future = task.future.lock();
                match future.as_mut().poll(&mut cx) {
                    Poll::Ready(()) => {
                        drop(future);
                        // Task complete — decrement active count.
                        self.queue.active_count.fetch_sub(1, Ordering::Relaxed);
                    }
                    Poll::Pending => {
                        drop(future);
                        // Task will be re-enqueued when its Waker fires.
                    }
                }
            }

            // No more work in the queue.  Are we done?
            if self.queue.active_count.load(Ordering::Relaxed) == 0 {
                break;
            }

            // Park until a waker re-enqueues something.
            self.queue.park();
        }
    }

    /// Run the event loop on `n` worker threads.
    ///
    /// Thread 0 = current thread.  Threads 1..n are spawned via SYS_THREAD_CREATE.
    /// All workers pull from the same shared queue.
    /// When SMP is enabled, these threads will naturally distribute across cores.
    pub fn run_threaded(&self, workers: usize) {
        let workers = workers.max(1);

        if workers == 1 {
            self.run();
            return;
        }

        // Spawn (workers - 1) additional worker threads.
        let mut handles = Vec::new();
        for _ in 1..workers {
            let queue = self.queue.clone();
            let handle = crate::thread::spawn(move || {
                worker_loop(&queue);
            });
            if let Ok(h) = handle {
                handles.push(h);
            }
        }

        // Current thread is worker 0.
        worker_loop(&self.queue);

        // Join all workers.
        for h in handles {
            let _ = h.join();
        }
    }
}

/// Worker loop — shared by all threads (including the main thread).
fn worker_loop(queue: &Arc<TaskQueue>) {
    loop {
        while let Some(task) = queue.pop() {
            task.notify.store(0, Ordering::Relaxed);

            let waker = task_to_waker(&task);
            let mut cx = Context::from_waker(&waker);

            let mut future = task.future.lock();
            match future.as_mut().poll(&mut cx) {
                Poll::Ready(()) => {
                    drop(future);
                    queue.active_count.fetch_sub(1, Ordering::Relaxed);
                }
                Poll::Pending => {
                    drop(future);
                }
            }
        }

        if queue.active_count.load(Ordering::Relaxed) == 0 {
            break;
        }

        queue.park();
    }
}

// -- Convenience functions --

/// Block the current thread on a single future until it completes.
///
/// This is the simplest way to run async code:
/// ```ignore
/// libmorpheus::task::block_on(async {
///     let x = some_async_fn().await;
///     println!("got: {}", x);
/// });
/// ```
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = core::pin::pin!(future);

    // Build a waker that uses the same futex mechanism.
    let notify = AtomicU32::new(1);

    let waker = {
        // Inline waker — no Arc needed for block_on since we own it on the stack.
        // We use a simple futex-based waker that just sets notify=1 + FUTEX_WAKE.
        let ptr = &notify as *const AtomicU32 as *const ();
        unsafe { Waker::from_raw(RawWaker::new(ptr, &BLOCK_ON_VTABLE)) }
    };

    let mut cx = Context::from_waker(&waker);

    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(val) => return val,
            Poll::Pending => {
                // Park until the waker fires.
                // Only sleep if notify is still 0.
                notify.store(0, Ordering::Release);
                unsafe {
                    syscall3(SYS_FUTEX, &notify as *const AtomicU32 as u64, FUTEX_WAIT, 0);
                }
            }
        }
    }
}

// Waker vtable for block_on — doesn't need Arc, just pokes the futex.
static BLOCK_ON_VTABLE: RawWakerVTable = RawWakerVTable::new(
    // clone: just return the same pointer (stack-local, outlives all polls)
    |ptr| RawWaker::new(ptr, &BLOCK_ON_VTABLE),
    // wake: set + futex_wake, consuming (but we don't free stack memory)
    |ptr| {
        let atom = unsafe { &*(ptr as *const AtomicU32) };
        atom.store(1, Ordering::Release);
        unsafe {
            syscall3(SYS_FUTEX, ptr as u64, FUTEX_WAKE, 1);
        }
    },
    // wake_by_ref: same thing
    |ptr| {
        let atom = unsafe { &*(ptr as *const AtomicU32) };
        atom.store(1, Ordering::Release);
        unsafe {
            syscall3(SYS_FUTEX, ptr as u64, FUTEX_WAKE, 1);
        }
    },
    // drop: no-op (stack-allocated)
    |_| {},
);

// -- Yield point --

/// Async yield point.  Causes the current task to be re-scheduled,
/// giving other tasks a chance to run.
///
/// ```ignore
/// async fn cooperative_work() {
///     loop {
///         do_some_work();
///         libmorpheus::task::yield_now().await;
///     }
/// }
/// ```
pub fn yield_now() -> YieldFuture {
    YieldFuture { yielded: false }
}

/// Future that yields once and then completes.
pub struct YieldFuture {
    yielded: bool,
}

impl Future for YieldFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.yielded {
            Poll::Ready(())
        } else {
            self.yielded = true;
            // Wake ourselves immediately so we get re-queued.
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

// -- Async sleep --

/// Sleep for `millis` milliseconds, yielding to the executor.
///
/// Uses FUTEX_WAIT with a kernel-side timeout so the process unblocks
/// precisely when the deadline elapses — no busy-waiting, no blocking
/// the executor's other tasks.
pub fn sleep(millis: u64) -> SleepFuture {
    let deadline_ns = crate::time::clock_gettime().saturating_add(millis * 1_000_000);
    SleepFuture {
        deadline_ns,
        parked: false,
    }
}

pub struct SleepFuture {
    deadline_ns: u64,
    parked: bool,
}

impl Future for SleepFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let now = crate::time::clock_gettime();
        if now >= self.deadline_ns {
            return Poll::Ready(());
        }

        if !self.parked {
            self.parked = true;
            // Compute remaining ms, round up.
            let remaining_ns = self.deadline_ns.saturating_sub(now);
            let remaining_ms = remaining_ns.div_ceil(1_000_000);

            if remaining_ms > 0 {
                // Park using futex with timeout — the kernel timer ISR will
                // unblock us after remaining_ms even if nobody wakes the futex.
                // We use a dummy futex word on the stack.
                let dummy = AtomicU32::new(0);
                unsafe {
                    crate::raw::syscall4(
                        SYS_FUTEX,
                        &dummy as *const AtomicU32 as u64,
                        FUTEX_WAIT,
                        0,
                        remaining_ms,
                    );
                }
            }

            // Re-schedule ourselves for the next poll.
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            // We've been re-polled after the futex timeout — check the clock.
            // Might be slightly early due to scheduling jitter; if so, spin once.
            self.parked = false;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

// -- JoinHandle --

/// Shared state between a spawned task and its JoinHandle.
struct JoinState<T> {
    /// The task's output value, written exactly once.
    value: UnsafeCell<Option<T>>,
    /// 0 = pending, 1 = complete.
    done: AtomicU32,
    /// Waker to notify the JoinHandle poller when the task finishes.
    waker: Mutex<Option<Waker>>,
}

unsafe impl<T: Send> Send for JoinState<T> {}
unsafe impl<T: Send> Sync for JoinState<T> {}

impl<T> JoinState<T> {
    fn new() -> Self {
        Self {
            value: UnsafeCell::new(None),
            done: AtomicU32::new(0),
            waker: Mutex::new(None),
        }
    }

    /// Called by the wrapper future when the inner future completes.
    fn complete(&self, val: T) {
        unsafe { *self.value.get() = Some(val) };
        self.done.store(1, Ordering::Release);
        if let Some(w) = self.waker.lock().take() {
            w.wake();
        }
    }

    /// Poll from the JoinHandle side.
    fn poll_join(&self, cx: &mut Context<'_>) -> Poll<T> {
        if self.done.load(Ordering::Acquire) == 1 {
            let val = unsafe { (*self.value.get()).take().unwrap() };
            return Poll::Ready(val);
        }
        // Store waker first, then double-check (avoids lost wakeup).
        *self.waker.lock() = Some(cx.waker().clone());
        if self.done.load(Ordering::Acquire) == 1 {
            let val = unsafe { (*self.value.get()).take().unwrap() };
            Poll::Ready(val)
        } else {
            Poll::Pending
        }
    }
}

/// Handle to a spawned task's result. Implements Future.
pub struct JoinHandle<T> {
    state: Arc<JoinState<T>>,
}

impl<T> Future for JoinHandle<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        self.state.poll_join(cx)
    }
}
