use std::ops::{Deref, DerefMut};
use std::task::Waker;
use std::time::Instant;

pub struct Sleep {
    pub until: Instant,
    pub waker: Waker,
}

pub struct Sleeps(Vec<Sleep>);

impl Deref for Sleeps {
    type Target = Vec<Sleep>;

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
        Self(vec![])
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
        static ref waker: Waker = TestWaker.into();
    }

    #[test]
    fn should_add_at_end() {
        let now = Instant::now();

        let mut sleeps = Sleeps(vec![
            Sleep { until: now + Duration::from_millis(100), waker: waker.clone() },
            Sleep { until: now + Duration::from_millis(200), waker: waker.clone() },
        ]);

        let i = sleeps.add(Sleep { until: now + Duration::from_millis(300), waker: waker.clone() });
        assert_eq!(i, 2);
        assert_eq!(sleeps.0.len(), 3);
    }

    #[test]
    fn should_add_at_middle() {
        let now = Instant::now();

        let mut sleeps = Sleeps(vec![
            Sleep { until: now + Duration::from_millis(100), waker: waker.clone() },
            Sleep { until: now + Duration::from_millis(300), waker: waker.clone() },
        ]);

        let i = sleeps.add(Sleep { until: now + Duration::from_millis(200), waker: waker.clone() });
        assert_eq!(i, 1);
        assert_eq!(sleeps.0.len(), 3);
    }

    #[test]
    fn should_add_at_beginning() {
        let now = Instant::now();

        let mut sleeps = Sleeps(vec![
            Sleep { until: now + Duration::from_millis(200), waker: waker.clone() },
            Sleep { until: now + Duration::from_millis(300), waker: waker.clone() },
        ]);

        let i = sleeps.add(Sleep { until: now + Duration::from_millis(100), waker: waker.clone() });
        assert_eq!(i, 0);
        assert_eq!(sleeps.0.len(), 3);
    }
}
