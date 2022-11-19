use std::collections::VecDeque;
use std::fmt::{self, Display, Formatter};
use std::ops::{Deref, DerefMut};
use std::task::Waker;
use std::time::Instant;

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
        Self {
            id,
            until,
            waker,
        }
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
    use std::time::Duration;

    use lazy_static::lazy_static;

    use super::*;
    use crate::reactor::sleep::tests::TestWaker;

    lazy_static! {
        static ref WAKER: Waker = TestWaker::new().waker();
    }

    #[test]
    fn should_add_at_end() {
        let now = Instant::now();

        let mut sleeps = Sleeps(VecDeque::from([
            Sleep::new(0, now + Duration::from_millis(100), WAKER.clone()),
            Sleep::new(1, now + Duration::from_millis(200), WAKER.clone()),
        ]));

        let i = sleeps.add(Sleep::new(2, now + Duration::from_millis(300), WAKER.clone()));
        assert_eq!(i, 2);
        assert_eq!(sleeps.0.len(), 3);
    }

    #[test]
    fn should_add_at_middle() {
        let now = Instant::now();

        let mut sleeps = Sleeps(VecDeque::from([
            Sleep::new(0, now + Duration::from_millis(100), WAKER.clone()),
            Sleep::new(1, now + Duration::from_millis(300), WAKER.clone()),
        ]));

        let i = sleeps.add(Sleep::new(2, now + Duration::from_millis(200), WAKER.clone()));
        assert_eq!(i, 1);
        assert_eq!(sleeps.0.len(), 3);
    }

    #[test]
    fn should_add_at_beginning() {
        let now = Instant::now();

        let mut sleeps = Sleeps(VecDeque::from([
            Sleep::new(0, now + Duration::from_millis(200), WAKER.clone()),
            Sleep::new(1, now + Duration::from_millis(300), WAKER.clone()),
        ]));

        let i = sleeps.add(Sleep::new(2, now + Duration::from_millis(100), WAKER.clone()));
        assert_eq!(i, 0);
        assert_eq!(sleeps.0.len(), 3);
    }
}
