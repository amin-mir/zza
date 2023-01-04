use crossbeam::channel;
// use std::thread;
// use std::time::{Duration, Instant};
// use std::cell::RefCell;

fn main() {
    let (done_tx, done_rx) = channel::bounded::<()>(0);
    let (ctrlc_tx, ctrlc_rx) = channel::bounded(0);

    ctrlc::set_handler(move || {
        // Move the done_tx to the closure, so when it returns the channel
        // will get dropped as well signalling done to all the receivers.
        // take_ownership(done_tx);
        ctrlc_tx.send(()).unwrap();
        println!("ctrlc handler is run")
    })
    .expect("Error setting Ctrl-C handler");

    ctrlc_rx.recv().unwrap();
    drop(done_tx);

    match done_rx.recv() {
        Ok(_) => println!("received ok, shouldn't have come here!!!"),
        Err(_) => println!("received err meaning that channel is disconnected"),
    }
}
