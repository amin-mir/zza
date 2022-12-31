use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use crossbeam::{
    channel::{self, Receiver, Sender},
    select,
};
use once_cell::sync::OnceCell;
use tracing::{error, info};

mod reactor;
use reactor::sleep::Spawner;

mod future;
use future::SleepFuture;

mod task;
use task::Task;

pub mod shutdown;

static EXECUTOR_TX: OnceCell<Sender<Arc<Task>>> = OnceCell::new();
static SLEEP_SPAWNER: OnceCell<Spawner> = OnceCell::new();

// TODO: return a handle that can be awaited for result.
// In that case future's output cannot be () anymore.
pub fn spawn<F: Future<Output = ()> + Send + 'static>(f: F) {
    let tx = EXECUTOR_TX.get().expect("Executor is not initialized");
    if let Err(reason) = Task::spawn(f, tx.clone()) {
        error!(%reason, "spawning a new task failed");
    }
}

/// Single-threaded executor.
/// Executor goes in a loop polling tasks in the polling queue.
/// As soon as they are polled, they are also removed from queue.
pub struct Executor {
    task_rx: Receiver<Arc<Task>>,
    task_tx: Sender<Arc<Task>>,
    done_rx: Receiver<()>,
}

impl Executor {
    pub fn new(done_rx: Receiver<()>) -> Self {
        let (task_tx, task_rx) = channel::unbounded();

        EXECUTOR_TX.set(task_tx.clone()).unwrap();
        SLEEP_SPAWNER
            .set(reactor::sleep::run(done_rx.clone()))
            .unwrap();

        Executor {
            task_tx,
            task_rx,
            done_rx,
        }
    }

    // TODO: why should the future be Send + 'static??
    pub fn spawn<F: Future<Output = ()> + Send + 'static>(&self, f: F) {
        if let Err(reason) = Task::spawn(f, self.task_tx.clone()) {
            error!(%reason, "spawning a new task failed");
        }
    }

    pub fn run(&self) {
        loop {
            select! {
                recv(self.task_rx) -> msg => {
                    let task = match msg {
                        Ok(task) => task,
                        Err(_) => {
                            info!("receive on task_rx failed, sender disconnected, quitting the loop");
                            break;
                        }
                    };
                    task.poll();
                }
                recv(self.done_rx) -> _ => {
                    info!("executor received done signal, quitting the loop");
                    break;
                }
            }
        }
    }
}

pub fn sleep(dur: Duration) -> impl Future<Output = ()> {
    SleepFuture::new(dur)
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    pub struct ResolvedFuture;

    impl Future for ResolvedFuture {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Ready(())
        }
    }
}
