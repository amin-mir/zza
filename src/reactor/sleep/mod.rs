use std::sync::{Arc, Mutex};
use std::thread;

use crossbeam::channel::{self, Receiver};

mod sleep;
use sleep::{Sleep, Sleeps};

mod spawner;
pub use spawner::Spawner;

mod scheduler;
use scheduler::Scheduler;

mod waiter;
use waiter::Waiter;

/// `run` will make three different components that together will perform
/// sleep reactor functionality:
///
/// 1. `Waiter` thread that receives sleep requests and waits before
///     calling the provided Waker.
/// 2. `Scheduler` thread that schedules new sleeps and signals `Waiter`.
/// 3. `Spawner` that is returned to caller for submitting new sleeps.
// TODO: done channel should be passed from caller.
pub fn run() -> Spawner {
    let (_done_tx, done_rx) = channel::bounded(0);

    let (interrupt_tx, interrupt_rx) = channel::bounded(1);

    let (sleep_tx, sleep_rx) = channel::unbounded();

    let sleeps = Arc::new(Mutex::new(Sleeps::new()));

    let spawner = Spawner::new(sleep_tx);
    let mut scheduler = Scheduler::new(done_rx.clone(), interrupt_tx, sleep_rx, sleeps.clone());
    let mut waiter = Waiter::new(interrupt_rx, done_rx.clone(), sleeps);

    thread::spawn(move || {
        waiter.run();
    });

    thread::spawn(move || {
        scheduler.run();
    });

    // TODO: should we combine the done channel with thread handles into spawner
    // so that when it's dropped it closes the cone channel and then joins the threads.

    spawner
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Wake, Waker};

    use crossbeam::channel::{self, Receiver, Sender};

    use super::*;

    pub struct TestWaker {
        woken: AtomicBool,
        wake_tx: Sender<()>,
        wake_rx: Receiver<()>,
    }

    impl TestWaker {
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

    impl Wake for TestWaker {
        fn wake(self: Arc<Self>) {
            self.woken.store(true, Ordering::Relaxed);
            self.wake_tx.send(()).unwrap();
        }
    }
}
