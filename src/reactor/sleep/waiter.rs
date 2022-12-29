use std::sync::{Arc, Mutex};
use std::time::Instant;

use crossbeam::channel::Receiver;
use crossbeam::select;
use tracing::debug;

use super::{Sleep, Sleeps};

/// Provides a waiting mechanism on sleep request with millisecond granularity.
/// The wait/park functionality is implemented by using timeouts when selecting
/// on channels, so there's no need to use the crossbeam's parker or a dedicated channel.
pub struct Waiter {
    // Channel to be notified when there's a new earlier sleep. Upon receiving
    // a message on this channel, waiter resets the loop and pops a new sleep.
    // It will be a bounded channel with capacity=1. The sender will `try_send`
    // to channel. The reason is that if Waiter is busy and haven't processed the
    // previous interrupt, we don't have to block the sender or keep sending more
    // interrupts. blocking the sender will reduce the sleep requests served by the
    // reactor, and having several unprocessed interrupts in the channel means the 
    // Waiter will unnecessarily wake up (and lock/unlock) several times in a loop.
    interrupt_rx: Receiver<()>,

    // Channel for signaling that Waiter should stop.
    done_rx: Receiver<()>,

    sleeps: Arc<Mutex<Sleeps>>,
}

impl Waiter {
    pub fn new(
        interrupt_rx: Receiver<()>,
        done_rx: Receiver<()>,
        sleeps: Arc<Mutex<Sleeps>>,
    ) -> Self {
        Self {
            interrupt_rx,
            done_rx,
            sleeps,
        }
    }

    pub fn run(&mut self) {
        // Using a variable declared outside of loop to reduce the number
        // of times we acquire the lock. This is useful when there's a new
        // sleep that finishes ealier than the current one being slept on.
        let mut unfinished_sleep: Option<Sleep> = None;

        loop {
            let mut sleeps = self.sleeps.lock().unwrap();

            if unfinished_sleep.is_some() {
                let sleep = unfinished_sleep.take().unwrap();
                let i = sleeps.add(sleep);
                debug!(index = i, "unfinished sleep re-added.");
            }

            debug!(sleeps = %*sleeps, "waiter's current sleeps");

            let sleep = sleeps.pop_front();

            // Release the lock before waiting on channels otherwise there would be a
            // deadlock because the lock is held here while Scheduler is trying to acquire
            // it to add more sleeps to the shared data structure `Sleeps`.
            drop(sleeps);

            match sleep {
                // Park the thread indefinitely when there are no sleeps to handle.
                None => {
                    debug!("going to park indefinitely because empty sleep list.");
                    select! {
                        recv(self.interrupt_rx) -> _msg => (),
                        recv(self.done_rx) -> _msg => break,
                    }
                }
                Some(sleep) => {
                    debug!(until = ?sleep.until, "waiter loop is going to park until sleep due time");

                    let now = Instant::now();
                    if now >= sleep.until {
                        debug!(
                            until = ?sleep.until,
                            "expired sleep, calling wake immediately",
                        );
                        sleep.waker.wake();
                        continue;
                    }

                    select! {
                        recv(self.interrupt_rx) -> _msg => {
                            if Instant::now() < sleep.until {
                                unfinished_sleep = Some(sleep);
                            }
                        }
                        recv(self.done_rx) -> _msg => break, 
                        default(sleep.until.duration_since(now)) => {
                            debug!(
                                sleep_duration_ms = now.elapsed().as_millis(),
                                "waking up the waker"
                            );
                            sleep.waker.wake();
                        }
                    }
                }
            }
        }

        debug!("waiter loop finished")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::thread;
    use std::time::Duration;

    use crossbeam::channel;
    use test_log::test;

    use crate::reactor::sleep::tests::TestWaker;

    #[test]
    fn should_wake_after_deadline() {
        let (interrupt_tx, interrupt_rx) = channel::bounded(1); 
        let (done_tx, done_rx) = channel::bounded(0);
        let sleeps = Arc::new(Mutex::new(Sleeps::new()));

        let mut waiter = Waiter::new(interrupt_rx, done_rx, sleeps.clone());
        let waiter_thread = thread::spawn(move || {
            waiter.run();
        });

        // Add a sleep request to the shared Sleeps and interrupt.
        let test_waker = TestWaker::new();
        {
            // The lock is released at the end of the block.
            let mut sleeps = sleeps.lock().unwrap();
            sleeps.add(Sleep::new(
                0,
                Instant::now() + Duration::from_millis(20),
                test_waker.clone().waker(),
            ));
            interrupt_tx.try_send(()).unwrap();
        }

        // Wait until waker is called.
        test_waker.wait_woken();

        drop(done_tx);
        waiter_thread.join().unwrap();
    }

    #[test]
    fn should_handle_expired_sleeps() {
        // Expired sleeps are the following:
        // Instant::now() > sleep.until
        let (interrupt_tx, interrupt_rx) = channel::bounded(1); 
        let (done_tx, done_rx) = channel::bounded(0);
        let sleeps = Arc::new(Mutex::new(Sleeps::new()));

        let mut waiter = Waiter::new(interrupt_rx, done_rx, sleeps.clone());
        let waiter_thread = thread::spawn(move || {
            waiter.run();
        });

        // Add a sleep request to the shared Sleeps and interrupt.
        let test_waker = TestWaker::new();
        {
            // The lock is released at the end of the block.
            let mut sleeps = sleeps.lock().unwrap();
            sleeps.add(Sleep::new(
                0,
                Instant::now() - Duration::from_millis(20),
                test_waker.clone().waker(),
            ));
            interrupt_tx.try_send(()).unwrap();
        }

        // Wait until waker is called.
        test_waker.wait_woken();

        drop(done_tx);
        waiter_thread.join().unwrap();
    }

    #[test]
    fn earlier_sleep_should_be_handled_first() {
        let (interrupt_tx, interrupt_rx) = channel::bounded(1);
        let (done_tx, done_rx) = channel::bounded(0);
        let sleeps = Arc::new(Mutex::new(Sleeps::new()));

        let mut waiter = Waiter::new(interrupt_rx, done_rx, sleeps.clone());
        let waiter_thread = thread::spawn(move || {
            waiter.run();
        });

        // Add a sleep request to the shared Sleeps and interrupt.
        let test_waker1 = TestWaker::new();
        {
            // The lock is released at the end of the block.
            let mut sleeps = sleeps.lock().unwrap();
            sleeps.add(Sleep::new(
                0,
                Instant::now() + Duration::from_millis(60),
                test_waker1.clone().waker(),
            ));
            interrupt_tx.try_send(()).unwrap();
        }

        // Waiting here to make sure the previous sleep request
        // gets picked up by Waiter.
        thread::sleep(Duration::from_millis(10));

        let test_waker2 = TestWaker::new();
        {
            // The lock is released at the end of the block.
            let mut sleeps = sleeps.lock().unwrap();
            sleeps.add(Sleep::new(
                1,
                Instant::now() + Duration::from_millis(20),
                test_waker2.clone().waker(),
            ));
            interrupt_tx.try_send(()).unwrap();
        }

        // Wait until waker2 is called.
        // By this time waker1 should not be woken yet.
        test_waker2.wait_woken();
        assert!(!test_waker1.is_woken());

        test_waker1.wait_woken();
        assert!(test_waker1.is_woken());

        drop(done_tx);
        waiter_thread.join().unwrap();
    }
}
