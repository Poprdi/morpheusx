//! Futex-driven async executor. Workers pull from a shared queue; pending
//! tasks park on per-task notify words via `SYS_FUTEX`. Adding cores = adding
//! workers; standard `core::task` vtable so any `Future` works.

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

struct Task {
    future: Mutex<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
    /// 0 = sleeping, 1 = notified.
    notify: AtomicU32,
    queue: Arc<TaskQueue>,
    self_ref: AtomicUsize,
}

// SAFETY: future is Send; everything else is atomic or under Mutex.
unsafe impl Send for Task {}
unsafe impl Sync for Task {}

unsafe fn waker_clone(ptr: *const ()) -> RawWaker {
    let arc = Arc::from_raw(ptr as *const Task);
    let cloned = arc.clone();
    core::mem::forget(arc);
    RawWaker::new(Arc::into_raw(cloned) as *const (), &VTABLE)
}

unsafe fn waker_wake(ptr: *const ()) {
    let arc = Arc::from_raw(ptr as *const Task);
    wake_inner(&arc);
}

unsafe fn waker_wake_by_ref(ptr: *const ()) {
    let arc = Arc::from_raw(ptr as *const Task);
    wake_inner(&arc);
    core::mem::forget(arc);
}

unsafe fn waker_drop(ptr: *const ()) {
    let _ = Arc::from_raw(ptr as *const Task);
}

fn wake_inner(task: &Arc<Task>) {
    task.notify.store(1, Ordering::Release);
    task.queue.push(task.clone());
    task.queue.unpark_one();
}

static VTABLE: RawWakerVTable =
    RawWakerVTable::new(waker_clone, waker_wake, waker_wake_by_ref, waker_drop);

fn task_to_waker(task: &Arc<Task>) -> Waker {
    let ptr = Arc::into_raw(task.clone()) as *const ();
    unsafe { Waker::from_raw(RawWaker::new(ptr, &VTABLE)) }
}

struct TaskQueue {
    queue: Mutex<VecDeque<Arc<Task>>>,
    /// 0 = empty, 1 = work pending.
    park_futex: AtomicU32,
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

    fn park(&self) {
        unsafe {
            syscall3(
                SYS_FUTEX,
                &self.park_futex as *const AtomicU32 as u64,
                FUTEX_WAIT,
                0,
            );
        }
    }

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

/// Async runtime; one per application, or use [`block_on`].
pub struct Runtime {
    queue: Arc<TaskQueue>,
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            queue: Arc::new(TaskQueue::new()),
        }
    }

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

    /// Spawn and return a join handle for the result.
    pub fn spawn_with_handle<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let join_state: Arc<JoinState<F::Output>> = Arc::new(JoinState::new());
        let js = join_state.clone();

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

    /// Single-threaded loop. FIFO poll; park on empty queue while tasks remain.
    pub fn run(&self) {
        loop {
            while let Some(task) = self.queue.pop() {
                task.notify.store(0, Ordering::Relaxed);

                let waker = task_to_waker(&task);
                let mut cx = Context::from_waker(&waker);

                let mut future = task.future.lock();
                match future.as_mut().poll(&mut cx) {
                    Poll::Ready(()) => {
                        drop(future);
                        self.queue.active_count.fetch_sub(1, Ordering::Relaxed);
                    },
                    Poll::Pending => {
                        drop(future);
                    },
                }
            }

            if self.queue.active_count.load(Ordering::Relaxed) == 0 {
                break;
            }

            self.queue.park();
        }
    }

    /// Run on `workers` threads sharing one queue. Current thread is worker 0;
    /// the rest are spawned via `SYS_THREAD_CREATE`.
    pub fn run_threaded(&self, workers: usize) {
        let workers = workers.max(1);

        if workers == 1 {
            self.run();
            return;
        }

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

        worker_loop(&self.queue);

        for h in handles {
            let _ = h.join();
        }
    }
}

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
                },
                Poll::Pending => {
                    drop(future);
                },
            }
        }

        if queue.active_count.load(Ordering::Relaxed) == 0 {
            break;
        }

        queue.park();
    }
}

/// Block the current thread on a single future.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = core::pin::pin!(future);

    let notify = AtomicU32::new(1);

    let waker = {
        let ptr = &notify as *const AtomicU32 as *const ();
        unsafe { Waker::from_raw(RawWaker::new(ptr, &BLOCK_ON_VTABLE)) }
    };

    let mut cx = Context::from_waker(&waker);

    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(val) => return val,
            Poll::Pending => {
                notify.store(0, Ordering::Release);
                unsafe {
                    syscall3(SYS_FUTEX, &notify as *const AtomicU32 as u64, FUTEX_WAIT, 0);
                }
            },
        }
    }
}

// Stack-local notify word; vtable just pokes the futex.
static BLOCK_ON_VTABLE: RawWakerVTable = RawWakerVTable::new(
    |ptr| RawWaker::new(ptr, &BLOCK_ON_VTABLE),
    |ptr| {
        let atom = unsafe { &*(ptr as *const AtomicU32) };
        atom.store(1, Ordering::Release);
        unsafe {
            syscall3(SYS_FUTEX, ptr as u64, FUTEX_WAKE, 1);
        }
    },
    |ptr| {
        let atom = unsafe { &*(ptr as *const AtomicU32) };
        atom.store(1, Ordering::Release);
        unsafe {
            syscall3(SYS_FUTEX, ptr as u64, FUTEX_WAKE, 1);
        }
    },
    |_| {},
);

/// Cooperative reschedule point.
pub fn yield_now() -> YieldFuture {
    YieldFuture { yielded: false }
}

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
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

/// Async sleep via timed `FUTEX_WAIT` — kernel timer unblocks at the deadline.
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
            let remaining_ns = self.deadline_ns.saturating_sub(now);
            let remaining_ms = remaining_ns.div_ceil(1_000_000);

            if remaining_ms > 0 {
                // Dummy stack futex; the timeout is what matters.
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

            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            // Spurious wake or scheduling jitter; recheck the clock next poll.
            self.parked = false;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

struct JoinState<T> {
    value: UnsafeCell<Option<T>>,
    /// 0 = pending, 1 = complete.
    done: AtomicU32,
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

    fn complete(&self, val: T) {
        unsafe { *self.value.get() = Some(val) };
        self.done.store(1, Ordering::Release);
        if let Some(w) = self.waker.lock().take() {
            w.wake();
        }
    }

    fn poll_join(&self, cx: &mut Context<'_>) -> Poll<T> {
        if self.done.load(Ordering::Acquire) == 1 {
            let val = unsafe { (*self.value.get()).take().unwrap() };
            return Poll::Ready(val);
        }
        // Register waker, then recheck — avoids lost wakeup.
        *self.waker.lock() = Some(cx.waker().clone());
        if self.done.load(Ordering::Acquire) == 1 {
            let val = unsafe { (*self.value.get()).take().unwrap() };
            Poll::Ready(val)
        } else {
            Poll::Pending
        }
    }
}

pub struct JoinHandle<T> {
    state: Arc<JoinState<T>>,
}

impl<T> Future for JoinHandle<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        self.state.poll_join(cx)
    }
}
