use std::time::{Duration, Instant};

use futures::future;

use tracing::info;
use zza::{sleep, Executor};

fn main() {
    // the call to init at the end sets this to be the
    // default, global collector for this application.
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .init();

    let executor = Executor::new();

    executor.spawn(async {
        println!("spawning the sleep future");
        let now = Instant::now();
        let sleep_fut1 = sleep(Duration::from_secs(2));
        let sleep_fut2 = sleep(Duration::from_secs(1));

        // future::join will consume both futures and return a wrapper
        // with a waker that will wake if any of the wakers of those futures
        // are woken.
        future::join(sleep_fut1, sleep_fut2).await;

        sleep(Duration::from_secs(1)).await;
        info!(dur_ms = now.elapsed().as_millis(), "sleep ended");
    });

    executor.run();
}
