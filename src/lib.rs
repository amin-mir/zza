//! Tasks has () as the output.
//! Could optionally implement Future for Task???
//!
//! Task should be a waker. Allocated on the Heap registers itself
//! with the reactor for specific resources. When those resources
//! become available the Task is woken up.
//!
//! Inspired by the following resources:
//! 1 - tokio examples (executor and wakers)
//! 2 - futures ArcWaker
//! 3 - waker-fn crate
//! 4 - name-needed game runtime implementation
//! 5 - Futures explained in 200 lines book

// Steps:
// 0. convert the raw waker into arc waker, add an id to each waker.
//    read and review the code and fix minor issues.
// 1. enable normal program finish, so when the main future finished
//    the program should exit.
// 2. enable spawning several sleeps at the same time.
// 3. using TLS and lazy_static! make it super easy to setup.
// 4. figure out how to write an IO reactor.

use std::future::Future;
use std::mem::ManuallyDrop;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use lazy_static::lazy_static;
use tracing::debug;

// So that main can use the reactor.
mod reactor;
use reactor::sleep::{self, Spawner};

lazy_static! {
    static ref SLEEP_SPAWNER: Spawner = sleep::run();
}

pub fn spawn<F: Future>(f: F) {}

/// Single-threaded executor.
/// Executor goes in a loop polling tasks in the polling queue.
/// As soon as they are polled, they are also removed from queue.
pub struct Executor {
    rx: Receiver<Arc<Task>>,
    tx: Sender<Arc<Task>>,
}

impl Executor {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Executor { tx, rx }
    }

    // DEISGN THOUGHT: why should the future be Send + 'static.
    pub fn spawn<F: Future<Output = ()> + Send + 'static>(&self, f: F) {
        Task::spawn(f, self.tx.clone());
    }

    pub fn run(&self) {
        for task in self.rx.iter() {
            task.poll();
        }
    }
}

/// Should Task be run only on a single thread in which case
/// it's LocalBoxFuture or be Send so it can be transmitted to
/// different threads thus BoxFuture.
///
/// Task implements the Wake functionality. It's what connects
/// the Reactor to Executor.
struct Task {
    // TODO: can we get rid of Mutex?
    // TODO: can we get rid of Box?
    future: Mutex<BoxFuture<'static, ()>>,
    schedule_tx: Sender<Arc<Task>>,
}

impl Task {
    fn spawn<F>(future: F, schedule_tx: Sender<Arc<Task>>) -> Arc<Task>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let task = Task {
            future: Mutex::new(Box::pin(future)),
            schedule_tx: schedule_tx.clone(),
        };
        let mut task = Arc::new(task);
        // TODO: convert SendError to some custom error with `thiserr`.
        // TODO: use tracing to log the channel is closed on the other side.
        schedule_tx.send(task.clone()).unwrap();
        task
    }

    fn poll(self: Arc<Self>) {
        let mut future = self.future.lock().unwrap();
        // TODO: how to avoid clone every time poll is called.
        let waker = self.clone().to_waker();
        let mut cx = Context::from_waker(&waker);
        let _ = future.as_mut().poll(&mut cx);
    }

    // DESIGN THOUGHT: should it get a reference to Arc and clone it
    // manually or should it get an already cloned Task (currently latter).
    fn to_waker(self: Arc<Self>) -> Waker {
        let task = Arc::into_raw(self);
        let raw_waker = RawWaker::new(task as *const (), &VTABLE);
        unsafe { Waker::from_raw(raw_waker) }
    }
}

// VTABLE implementation influenced by the following:
// https://docs.rs/crate/waker-fn/1.1.0/source/src/lib.rs
// https://docs.rs/futures-task/0.3.24/src/futures_task/waker.rs.html#19-21
// https://github.com/cfsamson/examples-futures/blob/bd23e73bc3e54da4c6cfff781d58a71c9f477ed4/src/main.rs#L85-L92
const VTABLE: RawWakerVTable = RawWakerVTable::new(
    clone_arc_raw,
    wake_arc_raw,
    wake_by_ref_arc_raw,
    drop_arc_raw,
);

fn clone_arc_raw(ptr: *const ()) -> RawWaker {
    let task = ptr as *const Task;
    unsafe {
        Arc::increment_strong_count(task);
    }
    RawWaker::new(ptr, &VTABLE)
}

fn wake_arc_raw(ptr: *const ()) {
    let task = ptr as *const Task;
    let task = unsafe { Arc::from_raw(task) };
    task.schedule_tx.send(task.clone()).unwrap();
}

fn wake_by_ref_arc_raw(ptr: *const ()) {
    let task = ptr as *const Task;
    let task = unsafe { ManuallyDrop::new(Arc::from_raw(task)) };
    task.schedule_tx.send(Arc::clone(&task)).unwrap();
}

fn drop_arc_raw(ptr: *const ()) {
    let task = ptr as *const Task;
    let task = unsafe { Arc::from_raw(task) };
    drop(task);
}

pub fn sleep(dur: Duration) -> impl Future<Output = ()> {
    SleepFuture::new(dur)
}

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

fn sleep_id() -> usize {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

struct SleepFuture {
    id: usize,

    // We use when instead of dur because it is resilient against the
    // cases where the executor is overloaded and the Future is
    // stuck in the queue to be processed. So if the deadline is passed
    // it will be immediately resolved.
    until: Instant,

    registered_with_reactor: bool,
}

impl SleepFuture {
    fn new(dur: Duration) -> Self {
        let until = Instant::now() + dur;
        SleepFuture {
            id: sleep_id(),
            until,
            registered_with_reactor: false,
        }
    }
}

impl Future for SleepFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let now = Instant::now();
        let until = self.until;

        debug!(id = self.id, ?until, ?now, "future is being polled");
        if until <= now {
            debug!(id = self.id, ?until, "future is resolved");
            return Poll::Ready(());
        }

        if !self.registered_with_reactor {
            let waker = cx.waker().clone();
            self.registered_with_reactor = true;
            SLEEP_SPAWNER.spawn(self.id, until, waker);
        } else {
            // TODO: how to update the waker previously registered
            // with the reactor. This will be necessary when futures
            // are passed between different threads.
            //
            // let mut waker = waker.lock().unwrap();
            // if !waker.will_wake(cx.waker()) {
            //     *waker = cx.waker().clone();
            // }
        }

        Poll::Pending
    }
}
