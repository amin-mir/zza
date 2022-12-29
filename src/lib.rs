// TODO:
// 1. read and review the code and fix minor issues.
// 2. enable normal program finish, so when the main future finished
//    the program should exit.
// 3. read what-the-async and figure out how to write an IO reactor.

use std::future::Future;
use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;

use crossbeam::channel::{self, Receiver, Sender};
use lazy_static::lazy_static;
use tracing::error;

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
// efficient to use thread local storage. The reason why we need
// interior mutability is that `EXECUTOR_TX` is not initialized
// at first. It will only get initialized after `Executor::new`.
thread_local! {
    static EXECUTOR_TX: RefCell<Option<Sender<Arc<Task>>>> = RefCell::new(None);
}

// TODO: return a handle that can be awaited for result.
// In that case future's output cannot be () anymore.
pub fn spawn<F: Future<Output = ()> + Send + 'static>(f: F) {
    EXECUTOR_TX.with(|cell| {
        let borrow = cell.borrow();
        let tx = borrow
            .as_ref()
            .expect("Executor should be initialized first");
        if let Err(reason) = Task::spawn(f, tx.clone()) {
            error!(%reason, "spawning a new task failed");
        }
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

    // TODO: why should the future be Send + 'static??
    pub fn spawn<F: Future<Output = ()> + Send + 'static>(&self, f: F) {
        if let Err(reason) = Task::spawn(f, self.tx.clone()) {
            error!(%reason, "spawning a new task failed");
        }
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

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::future::Future;

    pub struct ResolvedFuture;

    impl Future for ResolvedFuture {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Ready(())
        }
    }
}
