use std::sync::{Arc, Mutex};
use std::time::Instant;

use crossbeam::sync::Parker;
use tracing::{debug};

use super::{Sleep, Sleeps};

/// Provides a waiting mechanism on sleep request with
/// millisecond granularity.
pub struct Waiter {
    parker: Parker,
    sleeps: Arc<Mutex<Sleeps>>,
}

impl Waiter {
    pub fn new(parker: Parker, sleeps: Arc<Mutex<Sleeps>>) -> Self {
        Self { parker, sleeps }
    }

    // TODO: in order to support graceful shutdown, park/unpark should
    // be changed to channels:
    // ```
    // select! {
    //     interrupted(_) -> msg => assert_eq!(msg, Ok(10)),
    //     done(_) -> msg => assert_eq!(msg, Ok(20)),
    //     default(Duration::from_secs(sleep.until - Instant::now())) => println!("timed out"),
    // }
    // ```
    pub fn run(&mut self) {
        // Using a variable declared outside of loop to reduce the number
        // of times we acquire the lock. This is useful when there's a new
        // sleep that finishes ealier than the current one which caused the
        // current sleep to be interrupted.
        let mut unfinished_sleep: Option<Sleep> = None;

        loop {
            let mut sleeps = self.sleeps.lock().unwrap();

            if unfinished_sleep.is_some() {
                let sleep = unfinished_sleep.take().unwrap();
                let i = sleeps.add(sleep);
                debug!(index = i, "unfinished sleep re-added.");
            }

            let now = Instant::now();
            debug!(sleeps = %*sleeps, "waiter's current sleeps");

            match sleeps.pop_front() {
                // Park the thread indefinitely when there are no sleeps to handle.
                None => {
                    // Release the lock otherwise there would be a deadlock.
                    drop(sleeps);

                    debug!("going to park indefinitely because empty sleep list.");
                    self.parker.park();
                }
                Some(sleep) => {
                    // Release the lock otherwise there would be a deadlock.
                    drop(sleeps);

                    // `park_deadline` handles the case that `Instant::now() > sleep.until`
                    // so we don't need to manually check for that.
                    debug!(until = ?sleep.until, "waiter loop is going to park until sleep due time");
                    self.parker.park_deadline(sleep.until);

                    // But it's possible that we wake up while this deadline hasn't passed.
                    // So need to remember to put it back to the list in the next loop iteration.
                    if Instant::now() < sleep.until {
                        unfinished_sleep = Some(sleep);
                    } else {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::thread;
    use std::time::Duration;

    use test_log::test;

    use crate::reactor::sleep::tests::TestWaker;

    #[test]
    fn should_wake_after_deadline() {
        let parker = Parker::new();
        let unparker = parker.unparker().clone();

        let sleeps = Arc::new(Mutex::new(Sleeps::new()));

        let mut waiter = Waiter::new(parker, sleeps.clone());
        thread::spawn(move || {
            waiter.run();
        });

        // Add a sleep request to the shared Sleeps and unpark.
        let test_waker = TestWaker::new();
        {
            // The lock is released at the end of the block.
            let mut sleeps = sleeps.lock().unwrap();
            sleeps.add(Sleep::new(
                0, 
                Instant::now() + Duration::from_millis(20),
                test_waker.clone().waker(),
            ));
            unparker.unpark();
        }

        // Wait until waker is called.
        test_waker.wait_woken();
        assert!(test_waker.is_woken());
    }

    #[test]
    fn earlier_sleep_should_be_handled_first() {
        let parker = Parker::new();
        let unparker = parker.unparker().clone();

        let sleeps = Arc::new(Mutex::new(Sleeps::new()));

        let mut waiter = Waiter::new(parker, sleeps.clone());
        thread::spawn(move || {
            waiter.run();
        });

        // Add a sleep request to the shared Sleeps and unpark.
        let test_waker1 = TestWaker::new();
        {
            // The lock is released at the end of the block.
            let mut sleeps = sleeps.lock().unwrap();
            sleeps.add(Sleep::new(
                0,
                Instant::now() + Duration::from_millis(60),
                test_waker1.clone().waker(),
            ));
            unparker.unpark();
        }

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
            unparker.unpark();
        }

        // Wait until waker2 is called.
        // By this time waker1 should not be woken yet.
        test_waker2.wait_woken();
        assert!(test_waker2.is_woken());
        assert!(!test_waker1.is_woken());

        test_waker1.wait_woken();
        assert!(test_waker1.is_woken());
    }
}
