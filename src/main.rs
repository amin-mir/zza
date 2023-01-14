use std::time::{Duration, Instant};

use futures::future;
use tracing::info;
use tracing_subscriber::EnvFilter;

use zza::{shutdown, sleep, Executor};

fn main() {
    // the call to init at the end sets this to be the
    // default, global collector for this application.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let done_rx = shutdown::notify();

    let executor = Executor::new(done_rx);

    zza::spawn(async {
        let now = Instant::now();
        let sleep_fut1 = sleep(Duration::from_secs(2));
        let sleep_fut2 = sleep(Duration::from_secs(1));

        // future::join will consume both futures and return a wrapper
        // with a waker that will wake if any of the wakers of those futures
        // are woken.
        future::join(sleep_fut1, sleep_fut2).await;
        info!("future::join of two sleeps finished");

        let handle = zza::spawn(async {
            sleep(Duration::from_millis(500)).await;
            return 5;
        });
        info!("nested spawn result: {}", handle.await);

        sleep(Duration::from_secs(1)).await;
        info!(dur_ms = now.elapsed().as_millis(), "main future completed");
    });

    executor.run();
}
