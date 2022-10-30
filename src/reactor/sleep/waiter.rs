use std::sync::{Arc, Mutex};
use std::time::Instant;

use crossbeam::sync::Parker;

use super::{Sleep, Sleeps};

pub struct Waiter {
    parker: Parker,
    sleeps: Arc<Mutex<Sleeps>>,
}

impl Waiter {
    pub fn new(parker: Parker, sleeps: Arc<Mutex<Sleeps>>) -> Self {
        Self { parker, sleeps }
    }

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
                // Ensure that the reason we woke up is because there's a new
                // sleep at index 0. So when we re-insert the previous sleep
                // it shouldn't go to the first index.
                assert_ne!(i, 0);
            }

            match sleeps.pop() {
                // Park the thread indefinitely when there are no sleeps to handle.
                None => {
                    // Release the lock otherwise there would be a deadlock.
                    drop(sleeps);

                    println!("going to park indefinitely because empty sleep list.");
                    self.parker.park();
                }
                Some(sleep) => {
                    // Release the lock otherwise there would be a deadlock.
                    drop(sleeps);

                    // `park_deadline` handles the case that `Instant::now() > sleep.until`
                    // so we don't need to manually check for that.
                    println!("going to park until {:?}.", sleep.until);
                    self.parker.park_deadline(sleep.until);

                    // But it's possible that we wake up while this deadline hasn't passed.
                    // So need to remember to put it back to the list in the next loop iteration.
                    if Instant::now() < sleep.until {
                        unfinished_sleep = Some(sleep);
                    }
                }
            }
        }
    }
}
