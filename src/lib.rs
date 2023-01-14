use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use crossbeam::{
    channel::{self, Receiver, Sender},
    select,
};
use flurry::HashMap;
use futures::{channel::oneshot, pin_mut, FutureExt};
use once_cell::sync::OnceCell;
use tracing::{debug, error, info};

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

struct STFuture(RefCell<Pin<Box<dyn Future<Output = ()>>>>);

impl STFuture {
    fn new<F>(fut: F) -> Self
    where
        F: Future<Output = ()> + 'static,
    {
        STFuture(RefCell::new(Box::pin(fut)))
    }
}

// Because the runtime is single-threaded. Only a single instance
// of the executor will have exclusive access to all the futures
// and modifications to those futures are synchronized through
// channels in a single loop within run function.
unsafe impl Send for STFuture {}
unsafe impl Sync for STFuture {}

static EXECUTOR: OnceCell<Arc<Executor>> = OnceCell::new();
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

pub fn spawn<F>(fut: F) -> Handle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send,
{
    let executor = EXECUTOR.get().expect("Executor is not initialized");
    executor.spawn(fut)
}

/// Single-threaded executor.
/// Executor goes in a loop polling tasks in the polling queue.
pub struct Executor {
    futures: HashMap<usize, STFuture>,
    task_rx: Receiver<Arc<Task>>,
    task_tx: Sender<Arc<Task>>,
    done_rx: Receiver<()>,
}

impl Executor {
    pub fn new(done_rx: Receiver<()>) -> Arc<Self> {
        let (task_tx, task_rx) = channel::unbounded();
        let futures = HashMap::new();

        let executor = Executor {
            futures,
            task_tx,
            task_rx,
            done_rx: done_rx.clone(),
        };
        let executor = Arc::new(executor);

        if EXECUTOR.set(executor.clone()).is_err() {
            panic!("EXECUTOR_FUTURES is already set");
        }

        SLEEP_SPAWNER.set(reactor::sleep::run(done_rx)).unwrap();

        executor
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

        let futures = self.futures.pin();
        let fut_id = future_id();
        futures.insert(fut_id, STFuture::new(fut));

        debug!(
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
                            error!("receive on task_rx failed, sender disconnected, quitting the loop");
                            break;
                        }
                    };

                    let futures = self.futures.pin();
                    let fut = match futures.get(&task.future_id()){
                        None => {
                            error!(future_id = task.future_id(), "executor doesn't have a corresponding future");
                            break;
                        }
                        Some(fut) => fut,
                    };

                    let waker = futures::task::waker_ref(&task);
                    let mut cx = Context::from_waker(&waker);
                    if fut.0.borrow_mut().as_mut().poll(&mut cx).is_ready() {
                        // The future has finished executing so we can remove it from
                        // the map. Ignoring the result from remove which will cause the
                        // heap allocation for future to be deallocated.
                        futures.remove(&task.future_id());
                        debug!(fut_id = task.future_id(), remaining_futures = futures.len(), "a future completed execution");

                        if futures.is_empty() {
                            info!("all futures completed, quitting executor loop");
                            break;
                        }
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
