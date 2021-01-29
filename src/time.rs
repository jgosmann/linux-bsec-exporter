use super::bsec::Time;
use super::monitor::Sleep;
use std::time::{Duration, Instant};

pub struct TimeAlive {
    start: Instant,
}

impl Default for TimeAlive {
    fn default() -> Self {
        TimeAlive {
            start: Instant::now(),
        }
    }
}

impl Time for TimeAlive {
    fn timestamp_ns(&self) -> i64 {
        Instant::now().duration_since(self.start).as_nanos() as i64
    }
}

impl Sleep for TimeAlive {
    type SleepFuture = tokio::time::Sleep;
    fn sleep(&self, duration: Duration) -> Self::SleepFuture {
        tokio::time::sleep(duration)
    }
}
