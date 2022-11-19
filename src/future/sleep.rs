use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tracing::debug;

use crate::SLEEP_SPAWNER;

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

fn sleep_id() -> usize {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

pub struct SleepFuture {
    id: usize,

    // We use when instead of dur because it is resilient against the
    // cases where the executor is overloaded and the Future is
    // stuck in the queue to be processed. So if the deadline is passed
    // it will be immediately resolved.
    until: Instant,

    registered_with_reactor: bool,
}

impl SleepFuture {
    pub fn new(dur: Duration) -> Self {
        let until = Instant::now() + dur;
        SleepFuture {
            id: sleep_id(),
            until,
            registered_with_reactor: false,
        }
    }
}

impl Future for SleepFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let now = Instant::now();
        let until = self.until;

        debug!(id = self.id, ?until, ?now, "future is being polled");
        if until <= now {
            debug!(id = self.id, ?until, "future is resolved");
            return Poll::Ready(());
        }

        if !self.registered_with_reactor {
            let waker = cx.waker().clone();
            self.registered_with_reactor = true;
            SLEEP_SPAWNER.spawn(self.id, until, waker);
        } else {
            // TODO: how to update the waker previously registered
            // with the reactor. This will be necessary when futures
            // are passed between different threads.
            //
            // let mut waker = waker.lock().unwrap();
            // if !waker.will_wake(cx.waker()) {
            //     *waker = cx.waker().clone();
            // }
        }

        Poll::Pending
    }
}
