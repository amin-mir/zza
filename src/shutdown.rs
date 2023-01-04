use std::thread;

use crossbeam::channel::{self, Receiver};
use tracing::info;

pub fn notify() -> Receiver<()> {
    // Because ctrlc doesn't take ownership of the channel, it won't get
    // dropped after the provided closure finishes, therefore we need to
    // have another channel to close manually and broadcast the done signal.
    let (ctrlc_tx, ctrlc_rx) = channel::bounded(0);
    let (notify_tx, notify_rx) = channel::bounded(0);

    ctrlc::set_handler(move || {
        ctrlc_tx.send(()).unwrap();
    })
    .expect("Error setting Ctrl-C handler");

    thread::spawn(move || {
        info!("received shutdown signal, going to notify");
        ctrlc_rx.recv().unwrap();
        drop(notify_tx);
    });

    notify_rx
}
