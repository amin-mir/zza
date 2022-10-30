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
    use std::task::{Wake, Waker};
    use std::time::Duration;
    use std::time::Instant;

    use super::*;

    pub struct TestWaker;

    impl TestWaker {
        pub fn to_waker() -> Waker {
            Waker::from(Arc::new(TestWaker))
        }
    }

    impl From<TestWaker> for Waker {
        fn from(test_waker: TestWaker) -> Self {
            Waker::from(Arc::new(test_waker))
        }
    }

    impl Wake for TestWaker {
        fn wake(self: Arc<Self>) {}
    }

    #[test]
    fn test_spawn() {
        let spawner = run();

        let waker: Waker = TestWaker.into();
        let until = Instant::now() + Duration::from_millis(100);
        spawner.spawn(until, waker.clone());

        let until = Instant::now() + Duration::from_millis(50);
        spawner.spawn(until, waker);

        thread::sleep(Duration::from_secs(1));
    }
}
