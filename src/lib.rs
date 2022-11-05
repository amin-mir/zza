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
// 1. implement a SleepReactor that handles many SleepFutures
//    with a single thread. Basically the future will submit its
//    request (waker + duration) to the reactor instead of own thread.
//    *** binary search + thread parking??? Then the future itself
//    is very simple as it just checks the instant to know if it should resolve.
// 2. enable normal program finish, so when the main future finished
//    the program should exit.
// 3. enable spawning several sleeps at the same time.
use std::future::Future;
use std::mem::ManuallyDrop;
use std::pin::Pin;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use std::thread;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;

mod reactor;

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

struct SleepFuture {
    when: Instant,
    waker: Option<Arc<Mutex<Waker>>>,
}

impl SleepFuture {
    fn new(dur: Duration) -> Self {
        let when = Instant::now() + dur;
        SleepFuture {
            when: when,
            waker: None,
        }
    }
}

impl Future for SleepFuture {
    type Output = ();

    // CONCERNS: what if poll is called for the first time and a thread is spawned.
    // then the timeout passes but before the thread is woken up, the poll is somehow
    // called again (??? not possible because future cannot be polled again) and
    // the Future is resolved. But then thread wakes up and uses the
    // waker to the task which might be assigned to another Future by then.
    // Even if this happens, if the next future is not ready it will return pending.
    //
    // Solution: each time a task is driving a future it has to have a unique ID for that
    // which is used for confirming the correct wake up.
    // But currently tasks are dropped after they finish their work. So for each Future we
    // create a new task. But the Future contract says that futures should not be polled
    // after they have completed.
    // TODO: a pool of created tasks??
    //
    // Why change SleepFuture to contain when instead of dur??
    // Because as soon as an instance of it is created we calculate
    // the time it should wake up so it is resilient against the
    // cases where the executor is overloaded and the Future is
    // stuck in the queue to be processed.
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let now = Instant::now();
        let when = self.when;

        if when < now {
            return Poll::Ready(());
        }

        if let Some(waker) = &self.waker {
            let mut waker = waker.lock().unwrap();
            if !waker.will_wake(cx.waker()) {
                *waker = cx.waker().clone();
            }
        } else {
            // Spawn a new waker thread.
            let waker = cx.waker().clone();
            let waker = Arc::new(Mutex::new(waker));
            self.waker = Some(waker.clone());

            thread::spawn(move || {
                // We should not forget to call the waker.
                let now = Instant::now();

                if now < when {
                    thread::sleep(when - now);
                }

                let waker = waker.lock().unwrap();
                waker.wake_by_ref();
            });
        }

        Poll::Pending
    }
}

struct SleepReactor {}
