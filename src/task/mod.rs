use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::Context;

use crossbeam::channel::Sender;
use futures::future::BoxFuture;
use futures::task::{self, ArcWake};

/// Should Task be run only on a single thread in which case
/// it's LocalBoxFuture or be Send so it can be transmitted to
/// different threads thus BoxFuture?
///
/// Task implements the Wake functionality. It's what connects
/// the Reactor to Executor.
pub struct Task {
    // TODO: can we get rid of Mutex?
    // TODO: can we get rid of Box? probably yes, we can pin to stack
    // before polling. Future contract says it should not be moved after
    // it's been polled first time. Or we can add an Unpin restriction??
    future: Mutex<BoxFuture<'static, ()>>,
    schedule_tx: Sender<Arc<Task>>,
}

impl Task {
    pub fn spawn<F>(future: F, schedule_tx: Sender<Arc<Task>>) -> Arc<Task>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let task = Task {
            future: Mutex::new(Box::pin(future)),
            schedule_tx: schedule_tx.clone(),
        };
        let task = Arc::new(task);
        // TODO: convert SendError to some custom error with `thiserr`.
        // TODO: use tracing to log the channel is closed on the other side.
        schedule_tx.send(task.clone()).unwrap();
        task
    }

    pub fn poll(self: Arc<Self>) {
        let mut future = self.future.lock().unwrap();

        let waker = task::waker_ref(&self);
        let mut cx = Context::from_waker(&waker);

        let _ = future.as_mut().poll(&mut cx);
    }
}

impl ArcWake for Task {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        arc_self.schedule_tx.send(Arc::clone(arc_self)).unwrap();
    }
}

/*
Sample RawWaker implementation
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
