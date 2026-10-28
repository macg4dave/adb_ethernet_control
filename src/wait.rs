//! The single `wait_until(condition, deadline)` helper and the injectable clock.

use std::cell::Cell;
use std::time::{Duration, Instant};

use crate::log::{Log, quote};

/// Source of time for flows, so tests can run waits instantly.
pub trait Clock {
    /// Current monotonic time.
    fn now(&self) -> Instant;
    /// Pauses for `duration`.
    fn sleep(&self, duration: Duration);
}

/// The real clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// A clock that only advances when slept on; for tests.
pub struct FakeClock {
    start: Instant,
    offset: Cell<Duration>,
}

impl FakeClock {
    /// A fake clock starting now.
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            offset: Cell::new(Duration::ZERO),
        }
    }

    /// Total time slept so far.
    pub fn elapsed(&self) -> Duration {
        self.offset.get()
    }
}

impl Default for FakeClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.start + self.offset.get()
    }

    fn sleep(&self, duration: Duration) {
        self.offset.set(self.offset.get() + duration);
    }
}

/// How a wait ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Waited<T> {
    /// The condition held; `value` is what it produced.
    Met {
        /// Value returned by the condition.
        value: T,
        /// Time from the start of the wait.
        elapsed: Duration,
    },
    /// The deadline passed; `last` is the last observation.
    TimedOut {
        /// Last reason the condition was not met.
        last: String,
        /// Time from the start of the wait.
        elapsed: Duration,
    },
}

impl<T> Waited<T> {
    /// The value if the condition was met.
    pub fn met(self) -> Option<T> {
        match self {
            Self::Met { value, .. } => Some(value),
            Self::TimedOut { .. } => None,
        }
    }
}

/// Checks `condition` until it returns `Ok` or `timeout` passes, re-checking
/// every `poll`. `condition` returns `Err(observation)` while not met; each
/// change of observation is logged so waits are visible in the journal.
pub fn wait_until<T>(
    clock: &dyn Clock,
    log: &Log,
    what: &str,
    timeout: Duration,
    poll: Duration,
    mut condition: impl FnMut() -> Result<T, String>,
) -> Waited<T> {
    let start = clock.now();
    log.info(format_args!(
        "wait start for={what} timeout_s={}",
        timeout.as_secs()
    ));
    let mut last: Option<String> = None;
    loop {
        let elapsed = clock.now() - start;
        match condition() {
            Ok(value) => {
                log.info(format_args!(
                    "wait end for={what} result=met elapsed_ms={}",
                    elapsed.as_millis()
                ));
                return Waited::Met { value, elapsed };
            }
            Err(observation) => {
                if last.as_deref() != Some(observation.as_str()) {
                    log.info(format_args!(
                        "wait observe for={what} now={} elapsed_ms={}",
                        quote(&observation),
                        elapsed.as_millis()
                    ));
                }
                if elapsed >= timeout {
                    log.warn(format_args!(
                        "wait end for={what} result=timeout elapsed_ms={} last={}",
                        elapsed.as_millis(),
                        quote(&observation)
                    ));
                    return Waited::TimedOut {
                        last: observation,
                        elapsed,
                    };
                }
                last = Some(observation);
                clock.sleep(poll.min(timeout.saturating_sub(elapsed)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    #[test]
    fn met_immediately_does_not_sleep() {
        let clock = FakeClock::new();
        let (log, _) = Log::memory();
        let result = wait_until(&clock, &log, "x", SECOND * 10, SECOND, || {
            Ok::<_, String>(7)
        });
        assert_eq!(result.met(), Some(7));
        assert_eq!(clock.elapsed(), Duration::ZERO);
    }

    #[test]
    fn met_after_some_polls() {
        let clock = FakeClock::new();
        let (log, _) = Log::memory();
        let mut calls = 0;
        let result = wait_until(&clock, &log, "x", SECOND * 10, SECOND * 2, || {
            calls += 1;
            if calls == 3 {
                Ok(())
            } else {
                Err("not yet".into())
            }
        });
        assert!(matches!(result, Waited::Met { elapsed, .. } if elapsed == SECOND * 4));
    }

    #[test]
    fn times_out_with_last_observation_and_logs_changes_once() {
        let clock = FakeClock::new();
        let (log, buffer) = Log::memory();
        let result: Waited<()> = wait_until(&clock, &log, "phone", SECOND * 5, SECOND * 2, || {
            Err("absent".into())
        });
        assert_eq!(
            result,
            Waited::TimedOut {
                last: "absent".into(),
                elapsed: SECOND * 5
            }
        );
        let text = buffer.contents();
        assert_eq!(text.matches("wait observe").count(), 1);
        assert!(text.contains("result=timeout"));
    }
}
