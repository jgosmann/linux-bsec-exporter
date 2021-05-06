use super::monitor::Sleep;
use bsec::clock::TimePassed;
use std::time::Duration;

impl Sleep for TimePassed {
    type SleepFuture = tokio::time::Sleep;
    fn sleep(&self, duration: Duration) -> Self::SleepFuture {
        tokio::time::sleep(duration)
    }
}
