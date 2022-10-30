use std::task::Waker;
use std::time::Instant;

use crossbeam::channel::Sender;

use super::Sleep;

/// Spawner provides a way to submit sleep tasks to the Reactor
/// from the outside world.
#[derive(Clone)]
pub struct Spawner {
    sleep_tx: Sender<Sleep>,
}

impl Spawner {
    pub fn new(tx: Sender<Sleep>) -> Self {
        Self { sleep_tx: tx }
    }

    pub fn spawn(&self, until: Instant, waker: Waker) {
        let sleep = Sleep { until, waker };
        self.sleep_tx.send(sleep).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crossbeam::channel;

    use super::*;
    use crate::reactor::sleep::tests::TestWaker;

    #[test]
    fn spawn_constructs_correct_sleep() {
        let (tx, rx) = channel::unbounded();

        let waker: Waker = TestWaker.into();
        let until = Instant::now() + Duration::from_millis(200);

        let spawner = Spawner::new(tx);
        spawner.spawn(until, waker.clone());

        let spawned = rx.recv().unwrap();
        assert!(spawned.waker.will_wake(&waker));
        assert_eq!(spawned.until, until);
    }
}