use futures::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::time::{interval, Instant, Interval};

#[derive(Debug)]
pub struct ClockStream {
    get_duration: fn() -> Duration,
    interval: Interval,
}

impl ClockStream {
    pub fn new(get_duration: fn() -> Duration) -> Self {
        Self {
            get_duration,
            interval: interval(get_duration()),
        }
    }
}

impl Stream for ClockStream {
    type Item = Instant;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Instant>> {
        let ret = self.interval.poll_tick(cx).map(Some);
        if ret.is_ready() {
            let duration = (self.get_duration)();
            self.interval.reset_at(Instant::now() + duration);
        }
        ret
    }
}
