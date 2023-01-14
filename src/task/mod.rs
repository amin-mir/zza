use std::error;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;
use std::task::Wake;
use std::task::Waker;

use crossbeam::channel;
use crossbeam::channel::Receiver;
use crossbeam::channel::Sender;
use futures::channel::oneshot;
use futures::future::FutureExt;
use futures::task::ArcWake;
use tracing::info;

/// Task implements the Wake functionality. It's what connects
/// the Reactor to Executor.
// TODO: can make Task not be Arc??
// Maybe use RawWaker instead of ArcWake??
pub struct Task {
    fut_id: usize,
    schedule_tx: crossbeam::channel::Sender<Arc<Task>>,
}

impl fmt::Debug for Task {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Task{{ fut_id = {} }}", self.fut_id)
    }
}

impl Task {
    pub fn future_id(&self) -> usize {
        self.fut_id
    }

    pub fn spawn<O>(
        fut_id: usize,
        schedule_tx: crossbeam::channel::Sender<Arc<Task>>,
        output_rx: oneshot::Receiver<O>,
    ) -> Result<Handle<O>, SpawnError> {
        let handle = Handle::new(output_rx);

        let task = Task {
            fut_id,
            schedule_tx: schedule_tx.clone(),
        };
        let task = Arc::new(task);

        if schedule_tx.send(task).is_err() {
            return Err(SpawnError);
        }

        Ok(handle)
    }
}

impl ArcWake for Task {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        info!(fut_id = arc_self.fut_id, "calling wake_by_ref on Task");
        arc_self.schedule_tx.send(Arc::clone(arc_self)).unwrap();
    }
}

pub struct SimpleWaker {
    woken: AtomicBool,
    wake_tx: Sender<()>,
    wake_rx: Receiver<()>,
}

impl SimpleWaker {
    pub fn new() -> Arc<Self> {
        let (tx, rx) = channel::bounded(1);
        let waker = Self {
            woken: AtomicBool::new(false),
            wake_tx: tx,
            wake_rx: rx,
        };
        Arc::new(waker)
    }

    pub fn waker(self: Arc<Self>) -> Waker {
        Waker::from(self)
    }

    pub fn is_woken(self: &Arc<Self>) -> bool {
        self.woken.load(Ordering::Relaxed)
    }

    pub fn wait_woken(self: &Arc<Self>) {
        self.wake_rx.recv().unwrap();
        assert!(self.is_woken())
    }
}

impl Wake for SimpleWaker {
    fn wake(self: Arc<Self>) {
        self.woken.store(true, Ordering::Relaxed);
        self.wake_tx.send(()).unwrap();
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct SpawnError;

impl error::Error for SpawnError {}

impl fmt::Display for SpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        "channel for receiving Tasks on Executor side is closed".fmt(f)
    }
}

#[derive(Debug)]
pub struct Handle<O> {
    output_rx: oneshot::Receiver<O>,
}

impl<O> Handle<O> {
    fn new(output_rx: oneshot::Receiver<O>) -> Self {
        Handle { output_rx }
    }
}

impl<O> Future for Handle<O> {
    type Output = O;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Receiver resolves to Result<T, Canceled> and we unwrap the result here.
        self.output_rx.poll_unpin(cx).map(Result::unwrap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_error() {
        let (schedule_tx, schedule_rx) = crossbeam::channel::bounded(1);
        let (_output_tx, output_rx) = oneshot::channel::<()>();

        // Close the receiving side immediately.
        drop(schedule_rx);

        let err = Task::spawn(10, schedule_tx, output_rx).unwrap_err();
        assert_eq!(err, SpawnError);
    }

    #[test]
    fn spawn_handle_resolves_correctly() {
        // We're testing at the same time that the Handle implements
        // Future trait correctly.
        let (schedule_tx, _schedule_rx) = crossbeam::channel::bounded(1);

        let (output_tx, output_rx) = oneshot::channel();
        output_tx.send("some data".to_owned()).unwrap();

        let handle = Task::spawn(326, schedule_tx, output_rx).unwrap();
        let o = crate::block_on(handle);
        assert_eq!("some data", o);
    }
}

/*
Sample `RawWaker` implementation:

VTABLE implementation influenced by the following:
https://docs.rs/crate/waker-fn/1.1.0/source/src/lib.rs
https://docs.rs/futures-task/0.3.24/src/futures_task/waker.rs.html#19-21
https://github.com/cfsamson/examples-futures/blob/bd23e73bc3e54da4c6cfff781d58a71c9f477ed4/src/main.rs#L85-L92

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
*/
