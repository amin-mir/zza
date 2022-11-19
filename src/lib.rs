// TODO:
// 1. read and review the code and fix minor issues.
// 2. enable normal program finish, so when the main future finished
//    the program should exit.
// 3. enable spawning several sleeps at the same time.
// 4. using TLS and lazy_static! make it super easy to setup.
// 5. figure out how to write an IO reactor.

use std::sync::Arc;
use std::time::Duration;
use std::{cell::RefCell, future::Future};

use crossbeam::channel::{self, Receiver, Sender};
use lazy_static::lazy_static;

mod reactor;
use reactor::sleep::Spawner;

mod future;
use future::SleepFuture;

mod task;
use task::Task;

lazy_static! {
    static ref SLEEP_SPAWNER: Spawner = reactor::sleep::run();
}

// Because we need interior mutability in this case it's more
// efficient to thread local storage. The reason why we need
// interior mutability is that `EXECUTOR_TX` is not initialized
// at first. It will only get initialized after `Executor::new`.
thread_local! {
    static EXECUTOR_TX: RefCell<Option<Sender<Arc<Task>>>> = RefCell::new(None);
}

pub fn spawn<F: Future<Output = ()> + Send + 'static>(f: F) {
    EXECUTOR_TX.with(|cell| {
        let borrow = cell.borrow();
        let tx = borrow
            .as_ref()
            .expect("Executor should be initialized first");
        Task::spawn(f, tx.clone());
    })
}

/// Single-threaded executor.
/// Executor goes in a loop polling tasks in the polling queue.
/// As soon as they are polled, they are also removed from queue.
pub struct Executor {
    rx: Receiver<Arc<Task>>,
    tx: Sender<Arc<Task>>,
}

impl Executor {
    pub fn new() -> Self {
        let (tx, rx) = channel::unbounded();

        EXECUTOR_TX.with(|cell| {
            *cell.borrow_mut() = Some(tx.clone());
        });

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

pub fn sleep(dur: Duration) -> impl Future<Output = ()> {
    SleepFuture::new(dur)
}
