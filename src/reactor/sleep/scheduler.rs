use std::sync::{Arc, Mutex};

use crossbeam::channel::Receiver;
use crossbeam::sync::Unparker;

use super::{Sleep, Sleeps};

pub struct Scheduler {
    unparker: Unparker,
    sleep_rx: Receiver<Sleep>,
    sleeps: Arc<Mutex<Sleeps>>,
}

impl Scheduler {
    pub fn new(sleep_rx: Receiver<Sleep>, unparker: Unparker, sleeps: Arc<Mutex<Sleeps>>) -> Self {
        Self {
            unparker,
            sleep_rx,
            sleeps,
        }
    }

    pub fn run(&mut self) {
        // IDEA: can we read in batch so we do the locking one time only???
        while let Ok(sleep) = self.sleep_rx.recv() {
            println!("[Scheduler] going to acquire lock to send sleep to waiter.");
            let mut sleeps = self.sleeps.lock().unwrap();
            let i = sleeps.add(sleep);
            // Handle the case where there's an earlier time we should wake up
            // from sleep.
            if i == 0 {
                println!("[Scheduler] received an earlier sleep, going to unpark.");
                self.unparker.unpark();
            }
        }
    }
}
