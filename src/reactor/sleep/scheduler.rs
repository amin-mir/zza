use std::error::Error;
use std::fmt::Display;
use std::sync::{Arc, Mutex};

use crossbeam::channel::{Receiver, Sender, TrySendError};
use crossbeam::select;
use tracing::{debug, info};

use super::{Sleep, Sleeps};

pub struct Scheduler {
    done_rx: Receiver<()>,
    interrupt_tx: Sender<()>,
    sleep_rx: Receiver<Sleep>,
    sleeps: Arc<Mutex<Sleeps>>,
}

impl Scheduler {
    pub fn new(
        done_rx: Receiver<()>,
        interrupt_tx: Sender<()>,
        sleep_rx: Receiver<Sleep>,
        sleeps: Arc<Mutex<Sleeps>>,
    ) -> Self {
        Self {
            done_rx,
            interrupt_tx,
            sleep_rx,
            sleeps,
        }
    }

    pub fn run(&mut self) {
        // TODO: can we read in batch using a combination of recv and try_recv
        // so we do the locking only once?? Heap allocations can be avoided by
        // pre-allocating a vector and re-using it. Will it bring any performance
        // benefits? This should be measured.
        loop {
            select! {
                recv(self.sleep_rx) -> msg => {
                    let sleep = match msg {
                        Err(_) => {
                            info!("channel for receiving Sleep is closed");
                            break;
                        },
                        Ok(sleep) => sleep,
                    };

                    if let Err(e) = self.handle_sleep(sleep) {
                        info!("failed to handle sleep: {}", e);
                        break;
                    }
                }
                recv(self.done_rx) -> _ => break,
            }
        }

        debug!("scheduler loop finished, quitting the thread")
    }

    fn handle_sleep(&self, sleep: Sleep) -> Result<(), HandleSleepError> {
        debug!("going to acquire lock to send sleep to waiter.");
        let mut sleeps = self.sleeps.lock().unwrap();
        let i = sleeps.add(sleep);

        // Handle the case where there's an earlier time we should wake up from sleep.
        if i == 0 {
            debug!("received an earlier sleep, going to unpark.");
            match self.interrupt_tx.try_send(()) {
                Err(TrySendError::Disconnected(_)) => Err(HandleSleepError),

                // If channel was full or send was successful. interrupt_tx is a zero-capacity channel
                // and the reason we still return Ok is that it indicates the other side is not ready
                // to receive anymore messages because it's busy already processing other sleeps.
                // Take a look at waiter to understand more on this.
                _ => Ok(()),
            }
        } else {
            Ok(())
        }
    }
}

#[derive(Debug)]
struct HandleSleepError;

impl Error for HandleSleepError {}

impl Display for HandleSleepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("failed to try_send on closed interrupt_tx channel")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::thread;
    use std::time::{Duration, Instant};

    use crossbeam::channel::{self, RecvTimeoutError};
    use test_log::test;

    use crate::SimpleWaker;

    fn chan_not_received<T>(rx: Receiver<T>) -> Result<(), String> {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Err(RecvTimeoutError::Timeout) => Ok(()),
            Err(RecvTimeoutError::Disconnected) => {
                Err("Unparker called_rx channel is disconnected.".to_string())
            }
            _ => Err("unpark has been called.".to_string()),
        }
    }

    #[test]
    fn should_return_after_done_signal() {
        let (done_tx, done_rx) = channel::bounded(0);

        // interrupt_rx will be unusable because in the middle of the test, done signal
        // is sent, after which scheduler shuts down and the Sender half is closed.
        let (interrupt_tx, _interrupt_rx) = channel::bounded(1);

        // Setting channel capacity to 0, otherwise the send will be successful.
        // The point of this test is making sure the graceful shutdown works, meaning
        // that after done signal is received, scheduler won't process any more sleeps.
        // If send times out with a 0-capacity channel, it means no one is on the
        // other side listening for our messages.
        let (sleep_tx, sleep_rx) = channel::bounded(0); // TODO: should this be unbounded like how it is when actually used?

        let sleeps = Arc::new(Mutex::new(Sleeps::new()));

        let mut scheduler = Scheduler::new(done_rx, interrupt_tx, sleep_rx, sleeps);
        let scheduler_thread = thread::spawn(move || {
            scheduler.run();
        });

        thread::sleep(Duration::from_millis(10));

        // Signal done to scheduler.
        drop(done_tx);

        // Wait until the done signal is received by scheduler.
        thread::sleep(Duration::from_millis(10));

        let test_waker = SimpleWaker::new();
        let sleep = Sleep::new(
            1,
            Instant::now() + Duration::from_millis(100),
            test_waker.waker(),
        );

        // Scheduler shouldn't accept anymore sleeps. Any attempts to send on
        // the sleep_tx should result in SendError which will only happen when
        // the receiving side has disconnected.
        assert!(sleep_tx.send(sleep).is_err());
        scheduler_thread.join().unwrap();
    }

    #[test]
    fn should_not_interrupt() {
        let (done_tx, done_rx) = channel::bounded(0);
        let (interrupt_tx, interrupt_rx) = channel::bounded(1);
        let (sleep_tx, sleep_rx) = channel::unbounded();
        let sleeps = Arc::new(Mutex::new(Sleeps::new()));

        let test_waker = SimpleWaker::new();
        {
            let mut sleeps = sleeps.lock().unwrap();
            sleeps.add(Sleep::new(
                0,
                Instant::now() + Duration::from_millis(50),
                test_waker.clone().waker(),
            ));
        }

        let mut scheduler = Scheduler::new(done_rx, interrupt_tx, sleep_rx, sleeps);
        let scheduler_thread = thread::spawn(move || {
            scheduler.run();
        });

        sleep_tx
            .send(Sleep::new(
                1,
                Instant::now() + Duration::from_millis(100),
                test_waker.waker(),
            ))
            .unwrap();

        // Should not interrupt because haven't received an earlier sleep.
        chan_not_received(interrupt_rx).unwrap();
        drop(done_tx);
        scheduler_thread.join().unwrap();
    }

    #[test]
    fn should_interrupt() {
        let (done_tx, done_rx) = channel::bounded(0);
        let (interrupt_tx, interrupt_rx) = channel::bounded(1);
        let (sleep_tx, sleep_rx) = channel::unbounded();
        let sleeps = Arc::new(Mutex::new(Sleeps::new()));

        let test_waker = SimpleWaker::new();
        {
            // lock is released at the end of block.
            let mut sleeps = sleeps.lock().unwrap();
            sleeps.add(Sleep::new(
                0,
                Instant::now() + Duration::from_millis(50),
                test_waker.clone().waker(),
            ));
        }

        let mut scheduler = Scheduler::new(done_rx, interrupt_tx, sleep_rx, sleeps);
        let scheduler_thread = thread::spawn(move || {
            scheduler.run();
        });

        sleep_tx
            .send(Sleep::new(
                1,
                Instant::now() + Duration::from_millis(20),
                test_waker.waker(),
            ))
            .unwrap();

        // Should interrupt because receied an earlier sleep.
        interrupt_rx.recv().unwrap();
        drop(done_tx);
        scheduler_thread.join().unwrap();
    }
}
