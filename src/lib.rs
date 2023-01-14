use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use std::{future::Future, sync::Mutex};

use crossbeam::{
    channel::{self, Receiver, Sender},
    select,
};
use futures::pin_mut;
use futures::{channel::oneshot, future::BoxFuture, FutureExt};
use once_cell::sync::OnceCell;
use tracing::{error, info, warn};

mod reactor;
use reactor::sleep::Spawner;

mod future;
use future::SleepFuture;

mod task;
use task::{Handle, SimpleWaker, Task};

pub mod shutdown;

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

fn future_id() -> usize {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

type Futures = Arc<Mutex<HashMap<usize, BoxFuture<'static, ()>>>>;

static EXECUTOR_FUTURES: OnceCell<(Futures, Sender<Arc<Task>>)> = OnceCell::new();
static SLEEP_SPAWNER: OnceCell<Spawner> = OnceCell::new();

pub fn block_on<F: Future>(fut: F) -> F::Output {
    // Pin the future on stack.
    pin_mut!(fut);

    let simple_waker = SimpleWaker::new();
    let waker = simple_waker.clone().waker();
    let mut cx = Context::from_waker(&waker);

    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(o) => return o,
            Poll::Pending => simple_waker.wait_woken(),
        }
    }
}

// TODO: return a handle that can be awaited for result.
// In that case future's output cannot be () anymore.
pub fn spawn<F>(fut: F) -> Handle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send,
{
    let (futures, tx) = EXECUTOR_FUTURES.get().expect("Executor is not initialized");

    let (fut_output_tx, fut_output_rx) = oneshot::channel();

    let fut = fut.map(|o| {
        if fut_output_tx.send(o).is_err() {
            error!("failed to send future result because oneshot receiver is dropped")
        }
    });

    let mut futures = futures.lock().unwrap();
    let fut_id = future_id();
    futures.insert(fut_id, Box::pin(fut));
    info!(
        remaining_futures = futures.len(),
        "a new future was spawned"
    );
    drop(futures);

    Task::spawn(fut_id, tx.clone(), fut_output_rx).expect("spawning a new task failed")
}

/// Single-threaded executor.
/// Executor goes in a loop polling tasks in the polling queue.
/// As soon as they are polled, they are also removed from queue.
pub struct Executor {
    // For now using Pin<Box<dyn Future>> to ignore the internals
    // of HashMap. The future contract says it should not be moved
    // in memory once pinned and polled. If we Box and then Pin, it's
    // going to stay in the same location regardless of how Vector
    // manages its underlying memory.
    // TODO: can we completely get rid of the Mutex? Previously it was
    // part of the Task and now here.
    futures: Futures,
    task_rx: Receiver<Arc<Task>>,
    task_tx: Sender<Arc<Task>>,
    done_rx: Receiver<()>,
}

impl Executor {
    pub fn new(done_rx: Receiver<()>) -> Self {
        let (task_tx, task_rx) = channel::unbounded();

        let futures = Arc::new(Mutex::new(HashMap::new()));
        if EXECUTOR_FUTURES
            .set((futures.clone(), task_tx.clone()))
            .is_err()
        {
            panic!("EXECUTOR_FUTURES is already set");
        }

        SLEEP_SPAWNER
            .set(reactor::sleep::run(done_rx.clone()))
            .unwrap();

        Executor {
            futures,
            task_tx,
            task_rx,
            done_rx,
        }
    }

    // TODO: why should the future be Send + 'static??
    pub fn spawn<F>(&self, fut: F) -> Handle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send,
    {
        let (fut_output_tx, fut_output_rx) = oneshot::channel();

        let fut = fut.map(|o| {
            if fut_output_tx.send(o).is_err() {
                error!("failed to send future result because oneshot receiver is dropped")
            }
        });

        let mut futures = self.futures.lock().unwrap();
        let fut_id = future_id();
        futures.insert(fut_id, Box::pin(fut));
        info!(
            remaining_futures = futures.len(),
            "a new future was spawned"
        );
        drop(futures);

        Task::spawn(fut_id, self.task_tx.clone(), fut_output_rx)
            .expect("spawning a new task failed")
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

                    let mut futures = self.futures.lock().unwrap();
                    let fut = match futures.get_mut(&task.future_id()){
                        None => {
                            warn!(future_id = task.future_id(), "executor doesn't have a corresponding future");
                            break;
                        }
                        Some(fut) => fut,
                    };

                    let waker = futures::task::waker_ref(&task);
                    let mut cx = Context::from_waker(&waker);
                    if fut.as_mut().poll(&mut cx).is_ready() {
                        // The future has finished executing so we can remove it from
                        // the map. Ignoring the result from remove which will cause the
                        // heap allocation for future to be deallocated.
                        futures.remove(&task.future_id());
                        info!(remaining_futures = futures.len(), "a future completed execution");
                    }
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
