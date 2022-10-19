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
use std::future::Future;
use std::mem::ManuallyDrop;
use std::sync::mpsc::{channel, Sender, Receiver};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use futures::future::BoxFuture;

pub fn spawn<F: Future>(f: F) {}

/// Single-threaded executor.
/// Executor goes in a loop polling tasks in the polling queue.
/// As soon as they are polled, they are also removed from queue.
struct Executor {
    rx: Receiver<Arc<Task>>,
    tx: Sender<Arc<Task>>,
}

impl Executor {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Executor {
            tx,
            rx,
        }
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
    waker: Option<Waker>,
}

impl Task {
    fn spawn<F>(future: F, schedule_tx: Sender<Arc<Task>>) -> Arc<Task>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let task = Task {
            future: Mutex::new(Box::pin(future)),
            schedule_tx,
            waker: None,
        };
        let mut task = Arc::new(task);
        task.waker = Some(task.to_waker());
        // TODO: convert SendError to some custom error with `thiserr`.
        // TODO: use tracing to log the channel is closed on the other side.
        schedule_tx.send(task).unwrap();
        task
    }

    fn poll(self: Arc<Self>) {
        let mut future = self.future.lock().unwrap();
        let mut cx = Context::from_waker(&self.waker.as_ref().unwrap());
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

// Separating the executor sender channel
// into a separate tuple struct. This will also implement
// the RawWaker functionality and allows us to build the RawWaker only
// once. Because the task itself is used behind an Arc we can't assign
// the waker after the task has been created except by wrapping it in a Mutex.
#[derive(Clone)]
struct TaskSend(Sender<Arc<Task>>);

impl TaskSend {
    fn to_waker() {}
    fn new(sender: Sender<Arc<Task>>) {}
}

// VTABLE implementation influenced by the following:
// https://docs.rs/crate/waker-fn/1.1.0/source/src/lib.rs
// https://docs.rs/futures-task/0.3.24/src/futures_task/waker.rs.html#19-21
// https://github.com/cfsamson/examples-futures/blob/bd23e73bc3e54da4c6cfff781d58a71c9f477ed4/src/main.rs#L85-L92
const VTABLE: RawWakerVTable = RawWakerVTable::new(
    clone_raw_waker,
    wake_raw_waker,
    wake_by_ref_raw_waker,
    drop_raw_waker,
);

fn clone_raw_waker(ptr: *const ()) -> RawWaker {
    let task = ptr as *const Task;
    unsafe { Arc::increment_strong_count(task); }
    RawWaker::new(ptr, &VTABLE)
}

fn wake_raw_waker(ptr: *const ()) {
    let task = ptr as *const Task;
    let task = unsafe { Arc::from_raw(task) };
    task.schedule_tx.send(task.clone()).unwrap();
}

fn wake_by_ref_raw_waker(ptr: *const ()) {
    let task = ptr as *const Task;
    let task = unsafe { ManuallyDrop::new(Arc::from_raw(task)) };
    task.schedule_tx.send(Arc::clone(&task)).unwrap();
}

fn drop_raw_waker(ptr: *const ()) {
    let task = ptr as *const Task;
    let task = unsafe { Arc::from_raw(task) };
    drop(task);
}

struct SleepReactor {

}
