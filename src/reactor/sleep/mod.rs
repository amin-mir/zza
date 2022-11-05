use std::sync::{Arc, Mutex};
use std::thread;

use crossbeam::channel;
use crossbeam::sync::Parker;

mod sleep;
use sleep::{Sleep, Sleeps};

mod spawner;
use spawner::Spawner;

mod scheduler;
use scheduler::Scheduler;

mod waiter;
use waiter::Waiter;

// TODO List:
// add more tests for scheduler.
// integrate tracing.
// update sleep future to use the new reactor.

pub fn run() -> Spawner {
    // `Sleeper` thread that sleeps
    // `Scheduler` thread that schedules new sleeps and signals `Sleeper`
    // `Spawner` that is returned so submit new sleeps
    let parker = Parker::new();
    let unparker = parker.unparker().clone();

    let (tx, rx) = channel::unbounded();

    let sleeps = Arc::new(Mutex::new(Sleeps::new()));

    let spawner = Spawner::new(tx);
    let mut scheduler = Scheduler::new(rx, unparker, sleeps.clone());
    let mut waiter = Waiter::new(parker, sleeps);

    thread::spawn(move || {
        waiter.run();
    });

    thread::spawn(move || {
        scheduler.run();
    });

    spawner
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Wake, Waker};
    use std::time::Duration;
    use std::time::Instant;

    use crossbeam::channel::{self, Sender, Receiver};

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
        }
    }

    impl Wake for TestWaker {
        fn wake(self: Arc<Self>) {
            self.woken.store(true, Ordering::Relaxed);
            self.wake_tx.send(()).unwrap();
        }
    }

    #[test]
    fn test_spawn() {
        let spawner = run();

        let test_waker = TestWaker::new();
        let waker: Waker = test_waker.waker();
        let until = Instant::now() + Duration::from_millis(100);
        spawner.spawn(until, waker.clone());

        let until = Instant::now() + Duration::from_millis(50);
        spawner.spawn(until, waker);

        thread::sleep(Duration::from_secs(1));
    }
}
