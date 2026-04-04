use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A simple countdown timer
#[derive(Debug, Clone)]
pub struct Timer {
    /// When the timer was started (None if stopped)
    started_at: Option<Instant>,
    /// Total duration in seconds
    duration_secs: u32,
    /// Whether the timer is paused
    paused: bool,
    /// Elapsed when paused
    paused_elapsed: Duration,
}

impl Timer {
    pub fn new() -> Self {
        Self {
            started_at: None,
            duration_secs: 25 * 60, // default 25 min pomodoro
            paused: false,
            paused_elapsed: Duration::ZERO,
        }
    }

    /// Start or restart the timer with the given duration in minutes
    pub fn start(&mut self, minutes: u32) {
        self.duration_secs = minutes * 60;
        self.started_at = Some(Instant::now());
        self.paused = false;
        self.paused_elapsed = Duration::ZERO;
    }

    /// Toggle pause/resume
    pub fn toggle_pause(&mut self) {
        if let Some(start) = self.started_at {
            if self.paused {
                // Resume: shift started_at forward by the pause duration
                let pause_duration = start.elapsed() - self.paused_elapsed;
                self.started_at = Some(Instant::now() - self.paused_elapsed);
                self.paused = false;
            } else {
                // Pause: record elapsed so far
                self.paused_elapsed = start.elapsed();
                self.paused = true;
            }
        }
    }

    /// Stop the timer
    pub fn stop(&mut self) {
        self.started_at = None;
        self.paused = false;
    }

    /// Check if timer is running
    pub fn is_running(&self) -> bool {
        self.started_at.is_some()
    }

    /// Get remaining time as (minutes, seconds), or None if not running
    pub fn remaining(&self) -> Option<(u32, u32)> {
        let start = self.started_at?;
        let elapsed = if self.paused {
            self.paused_elapsed
        } else {
            start.elapsed()
        };
        let elapsed_secs = elapsed.as_secs() as u32;
        if elapsed_secs >= self.duration_secs {
            Some((0, 0)) // Timer expired
        } else {
            let remaining = self.duration_secs - elapsed_secs;
            Some((remaining / 60, remaining % 60))
        }
    }

    /// Check if timer has expired
    pub fn is_expired(&self) -> bool {
        self.remaining().map(|(m, s)| m == 0 && s == 0).unwrap_or(false)
    }

    /// Format remaining time as "MM:SS" or status string
    pub fn display(&self) -> String {
        match self.remaining() {
            Some((0, 0)) => "Done!".into(),
            Some((m, s)) => {
                let pause_indicator = if self.paused { " ||" } else { "" };
                format!("{:02}:{:02}{}", m, s, pause_indicator)
            }
            None => "—".into(),
        }
    }
}

pub type SharedTimer = Arc<Mutex<Timer>>;

pub fn new_shared() -> SharedTimer {
    Arc::new(Mutex::new(Timer::new()))
}
