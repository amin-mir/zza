// TODO:
// 1. read and review the code and fix minor issues.
// 2. enable normal program finish, so when the main future finished
//    the program should exit.
// 3. enable spawning several sleeps at the same time.
// 4. using TLS and lazy_static! make it super easy to setup.
// 5. figure out how to write an IO reactor.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use crossbeam::channel::{self, Receiver, Sender};
use lazy_static::lazy_static;

// So that main can use the reactor.
mod reactor;
use reactor::sleep::{self, Spawner};

mod future;
use future::SleepFuture;

mod task;
use task::Task;

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
        let (tx, rx) = channel::unbounded();
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
