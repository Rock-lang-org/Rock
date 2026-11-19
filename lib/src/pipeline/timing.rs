//! Opt-in per-phase wall-clock timing, enabled with `ROCK_TIME_PHASES=1`.
//!
//! Used to measure compile-time regressions of pipeline passes; it prints to
//! stderr and costs one environment lookup when disabled.

use std::time::Instant;

pub(crate) struct PhaseTimer {
    last: Option<Instant>,
}

impl PhaseTimer {
    pub(crate) fn from_env() -> Self {
        let enabled = std::env::var_os("ROCK_TIME_PHASES").is_some_and(|value| value != "0");
        Self {
            last: enabled.then(Instant::now),
        }
    }

    /// Reports the time spent since the previous lap under `phase`.
    pub(crate) fn lap(&mut self, phase: &str) {
        let Some(last) = self.last else {
            return;
        };
        let now = Instant::now();
        eprintln!(
            "[rock-time] {phase:<16} {:>10.3} ms",
            (now - last).as_secs_f64() * 1000.0
        );
        self.last = Some(now);
    }
}
