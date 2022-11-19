use std::sync::{Arc, Mutex};

use crossbeam::channel::Receiver;
use tracing::debug;

use super::{Sleep, Sleeps};

pub trait Unparker {
    fn unpark(&self);
}

impl Unparker for crossbeam::sync::Unparker {
    fn unpark(&self) {
        self.unpark();
    }
}

pub struct Scheduler<U: Unparker> {
    unparker: U,
    sleep_rx: Receiver<Sleep>,
    sleeps: Arc<Mutex<Sleeps>>,
}

impl<U: Unparker> Scheduler<U> {
    pub fn new(sleep_rx: Receiver<Sleep>, unparker: U, sleeps: Arc<Mutex<Sleeps>>) -> Self {
        Self {
            unparker,
            sleep_rx,
            sleeps,
        }
    }

    pub fn run(&mut self) {
        // IDEA: can we read in batch so we do the locking one time only???
        while let Ok(sleep) = self.sleep_rx.recv() {
            debug!("going to acquire lock to send sleep to waiter.");
            let mut sleeps = self.sleeps.lock().unwrap();
            let i = sleeps.add(sleep);
            // Handle the case where there's an earlier time we should wake up
            // from sleep.
            if i == 0 {
                debug!("received an earlier sleep, going to unpark.");
                self.unparker.unpark();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crossbeam::channel::RecvTimeoutError;
    use tracing::{trace_span, trace};
    use std::thread;
    use std::time::{Duration, Instant};

    use crossbeam::channel::{self, Sender};
    use test_log::test;

    use crate::reactor::sleep::tests::TestWaker;

    struct MockUnparkerInner {
        called: u32,
    }

    #[derive(Clone)]
    struct MockUnparker {
        state: Arc<Mutex<MockUnparkerInner>>,
        called_tx: Sender<()>,
        called_rx: Receiver<()>,
    }

    impl MockUnparker {
        fn new() -> Self {
            let (called_tx, called_rx) = channel::unbounded();
            Self {
                state: Arc::new(Mutex::new(MockUnparkerInner { called: 0 })),
                called_tx,
                called_rx,
            }
        }

        fn not_called(&self) -> Result<(), String> {
            match self.called_rx.recv_timeout(Duration::from_millis(50)) {
                Err(RecvTimeoutError::Timeout) => Ok(()),
                Err(RecvTimeoutError::Disconnected) => {
                    Err("Unparker called_rx channel is disconnected.".to_string())
                }
                _ => Err("unpark has been called.".to_string()),
            }
        }

        fn called(&self) -> u32 {
            self.wait_called();
            let state = self.state.lock().unwrap();
            state.called
        }

        fn wait_called(&self) {
            let _span = trace_span!("MockUnparker.wait_called").entered();

            trace!("waiting for unpark to be called.");
            let now = Instant::now();
            self.called_rx.recv().unwrap();
            trace!("unpark called after {}.", now.elapsed().as_millis());
        }
    }

    impl Unparker for MockUnparker {
        fn unpark(&self) {
            let mut state = self.state.lock().unwrap();
            state.called += 1;
            self.called_tx.send(()).unwrap();
        }
    }

    #[test]
    fn should_not_unpark() -> Result<(), String> {
        let (sleep_tx, sleep_rx) = channel::bounded(1);

        let unparker = MockUnparker::new();

        let test_waker = TestWaker::new();

        let sleeps = Arc::new(Mutex::new(Sleeps::new()));
        {
            let mut sleeps = sleeps.lock().unwrap();
            sleeps.add(Sleep::new(
                0,
                Instant::now() + Duration::from_millis(50),
                test_waker.clone().waker(),
            ));
        }

        let mut scheduler = Scheduler::new(sleep_rx, unparker.clone(), sleeps);
        thread::spawn(move || {
            scheduler.run();
        });

        sleep_tx
            .send(Sleep::new(
                1,
                Instant::now() + Duration::from_millis(100),
                test_waker.clone().waker(),
            ))
            .unwrap();

        // unpark should not have been called.
        unparker.not_called()
    }

    #[test]
    fn should_unpark() {
        let (sleep_tx, sleep_rx) = channel::bounded(1);

        let unparker = MockUnparker::new();

        let test_waker = TestWaker::new();

        let sleeps = Arc::new(Mutex::new(Sleeps::new()));
        {
            // lock is released at the end of block.
            let mut sleeps = sleeps.lock().unwrap();
            sleeps.add(Sleep::new(
                0,
                Instant::now() + Duration::from_millis(50),
                test_waker.clone().waker(),
            ));
        }

        let mut scheduler = Scheduler::new(sleep_rx, unparker.clone(), sleeps);
        thread::spawn(move || {
            scheduler.run();
        });

        sleep_tx
            .send(Sleep::new(
                1,
                Instant::now() + Duration::from_millis(20),
                test_waker.clone().waker(),
            ))
            .unwrap();

        // unpark should have been called.
        assert_eq!(unparker.called(), 1);
    }
}
