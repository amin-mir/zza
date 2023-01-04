use std::sync::{Arc, Mutex};
use std::thread;
use std::collections::VecDeque;
use std::fmt::{self, Display, Formatter};
use std::ops::{Deref, DerefMut};
use std::task::Waker;
use std::time::Instant;

use crossbeam::channel::{self, Receiver};

mod spawner;
pub use spawner::Spawner;

mod scheduler;
use scheduler::Scheduler;

mod waiter;
use waiter::Waiter;

/// `run` will make three different components that together will perform
/// sleep reactor functionality:
///
/// 1. `Waiter` thread that receives sleep requests and waits before
///     calling the provided Waker.
/// 2. `Scheduler` thread that schedules new sleeps and signals `Waiter`.
/// 3. `Spawner` that is returned to caller for submitting new sleeps.
// TODO: done channel should be passed from caller.
pub fn run(done_rx: Receiver<()>) -> Spawner {
    // TODO: we don't actually need a done channel here. if we can just drop the Spawner
    // then the sleep_tx will disconnect, thus attempting to receive on sleep_rx will result in
    // RecvError which causes scheduler to quit. Scheduler first tries to process all the queued sleeps (unbounded channel)
    // but that shouldn't take long as it just stores them to Sleeps. Scheduler getting dropped will in
    // turn cause the interrupt_tx to disconnect and when that is detected by Waiter, it will quit the loop
    // and shut down immediately (bounded cap=1), therefore we don't need a separate done channel. All we need is a way
    // to drop Spawner.
    let (interrupt_tx, interrupt_rx) = channel::bounded(1);

    let (sleep_tx, sleep_rx) = channel::unbounded();

    let sleeps = Arc::new(Mutex::new(Sleeps::new()));

    let spawner = Spawner::new(sleep_tx);
    let mut scheduler = Scheduler::new(done_rx.clone(), interrupt_tx, sleep_rx, sleeps.clone());
    let mut waiter = Waiter::new(interrupt_rx, done_rx.clone(), sleeps);

    thread::spawn(move || {
        waiter.run();
    });

    thread::spawn(move || {
        scheduler.run();
    });

    // TODO: should we combine the done channel with thread handles into spawner
    // so that when it's dropped it closes the done channel and then joins the threads.

    spawner
}

// TODO: add random ID to each sleep request and implement
// Display as it can help with debugging.
#[derive(Debug)]
pub struct Sleep {
    pub id: usize,
    pub until: Instant,
    pub waker: Waker,
}

impl Display for Sleep {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let dur = self.until.duration_since(Instant::now());
        write!(
            f,
            "Sleep {{id = {}, remaining_duration = {} }}",
            self.id,
            dur.as_millis()
        )
    }
}

impl Sleep {
    pub fn new(id: usize, until: Instant, waker: Waker) -> Self {
        Self { id, until, waker }
    }
}

#[derive(Debug)]
pub struct Sleeps(VecDeque<Sleep>);

impl Display for Sleeps {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.write_str("Sleeps:\n")?;
        self.0.iter().fold(Ok(()), |result, sleep| {
            result.and_then(|_| writeln!(f, "  {}", sleep))
        })
    }
}

impl Deref for Sleeps {
    type Target = VecDeque<Sleep>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Sleeps {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Sleeps {
    pub fn new() -> Self {
        Self(VecDeque::new())
    }
    // Finds the right index to insert the sleep.
    pub fn add(&mut self, sleep: Sleep) -> usize {
        let i = self.partition_point(|s| s.until < sleep.until);
        self.insert(i, sleep);
        i
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Wake, Waker};
    use std::time::Duration;

    use crossbeam::channel::{self, Receiver, Sender};

    use super::*;

    pub struct TestWaker {
        woken: AtomicBool,
        wake_tx: Sender<()>,
        wake_rx: Receiver<()>,
    }

    impl TestWaker {
        pub fn new() -> Arc<Self> {
            let (tx, rx) = channel::bounded(1);
            let waker = Self {
                woken: AtomicBool::new(false),
                wake_tx: tx,
                wake_rx: rx,
            };
            Arc::new(waker)
        }

        pub fn waker(self: Arc<Self>) -> Waker {
            Waker::from(self)
        }

        pub fn is_woken(self: &Arc<Self>) -> bool {
            self.woken.load(Ordering::Relaxed)
        }

        pub fn wait_woken(self: &Arc<Self>) {
            self.wake_rx.recv().unwrap();
            assert!(self.is_woken())
        }
    }

    impl Wake for TestWaker {
        fn wake(self: Arc<Self>) {
            self.woken.store(true, Ordering::Relaxed);
            self.wake_tx.send(()).unwrap();
        }
    }

    #[test]
    fn should_add_at_end() {
        let waker = TestWaker::new().waker();
        let now = Instant::now();

        let mut sleeps = Sleeps(VecDeque::from([
            Sleep::new(0, now + Duration::from_millis(100), waker.clone()),
            Sleep::new(1, now + Duration::from_millis(200), waker.clone()),
        ]));

        let i = sleeps.add(Sleep::new(
            2,
            now + Duration::from_millis(300),
            waker.clone(),
        ));
        assert_eq!(i, 2);
        assert_eq!(sleeps.0.len(), 3);
    }

    #[test]
    fn should_add_at_middle() {
        let waker = TestWaker::new().waker();
        let now = Instant::now();

        let mut sleeps = Sleeps(VecDeque::from([
            Sleep::new(0, now + Duration::from_millis(100), waker.clone()),
            Sleep::new(1, now + Duration::from_millis(300), waker.clone()),
        ]));

        let i = sleeps.add(Sleep::new(
            2,
            now + Duration::from_millis(200),
            waker.clone(),
        ));
        assert_eq!(i, 1);
        assert_eq!(sleeps.0.len(), 3);
    }

    #[test]
    fn should_add_at_beginning() {
        let waker = TestWaker::new().waker();
        let now = Instant::now();
       
        let mut sleeps = Sleeps(VecDeque::from([
            Sleep::new(0, now + Duration::from_millis(200), waker.clone()),
            Sleep::new(1, now + Duration::from_millis(300), waker.clone()),
        ]));

        let i = sleeps.add(Sleep::new(
            2,
            now + Duration::from_millis(100),
            waker.clone(),
        ));
        assert_eq!(i, 0);
        assert_eq!(sleeps.0.len(), 3);
    }
}
