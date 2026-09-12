// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Snapshot of a long-running, non-realtime job.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressState {
    /// Human-readable label for the active job.
    pub label: String,
    /// Completion in `[0.0, 1.0]`.
    pub fraction: f32,
    epoch: u64,
}

impl ProgressState {
    /// Rounded percent in `0..=100`.
    pub fn percent(&self) -> u32 {
        (self.fraction * 100.0).round().clamp(0.0, 100.0) as u32
    }

    /// Label plus percent, e.g. `"building peaks 42%"`.
    pub fn message(&self) -> String {
        format!("{} {}%", self.label, self.percent())
    }
}

/// Shareable handle for reporting and reading job progress.
///
/// `begin` starts a new epoch so a superseded background job cannot
/// overwrite or finish the current one.
#[derive(Clone, Debug)]
pub struct ProgressHandle {
    state: Arc<Mutex<Option<ProgressState>>>,
    epoch: Arc<AtomicU64>,
}

impl Default for ProgressHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressHandle {
    /// Create an idle progress handle with no active job.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(None)),
            epoch: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Start a new job epoch and return its id.
    pub fn begin(&self, label: impl Into<String>) -> u64 {
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        *self.state.lock().unwrap() = Some(ProgressState {
            label: label.into(),
            fraction: 0.0,
            epoch,
        });
        epoch
    }

    /// Whether `epoch` is still the active job id.
    pub fn is_epoch(&self, epoch: u64) -> bool {
        self.epoch.load(Ordering::SeqCst) == epoch
    }

    /// Update progress for `epoch` when it is still current.
    pub fn set_fraction(&self, epoch: u64, fraction: f32) {
        let mut guard = self.state.lock().unwrap();
        if let Some(state) = guard.as_mut() {
            if state.epoch == epoch {
                state.fraction = fraction.clamp(0.0, 1.0);
            }
        }
    }

    /// Set progress from a `done` / `total` ratio for `epoch`.
    pub fn set_ratio(&self, epoch: u64, done: u64, total: u64) {
        let fraction = if total == 0 {
            1.0
        } else {
            done as f32 / total as f32
        };
        self.set_fraction(epoch, fraction);
    }

    /// Clear the job when `epoch` matches the active one.
    pub fn finish(&self, epoch: u64) {
        let mut guard = self.state.lock().unwrap();
        if guard.as_ref().is_some_and(|state| state.epoch == epoch) {
            *guard = None;
        }
    }

    /// Invalidate the current job so a superseded worker cannot finish it.
    pub fn cancel(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        *self.state.lock().unwrap() = None;
    }

    /// Clone the current progress snapshot, if any.
    pub fn snapshot(&self) -> Option<ProgressState> {
        self.state.lock().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_update_finish_and_message() {
        let progress = ProgressHandle::new();
        assert!(progress.snapshot().is_none());
        let epoch = progress.begin("building peaks");
        progress.set_fraction(epoch, 0.42);
        let snap = progress.snapshot().unwrap();
        assert_eq!(snap.message(), "building peaks 42%");
        progress.finish(epoch);
        assert!(progress.snapshot().is_none());
    }

    #[test]
    fn stale_epoch_cannot_finish_new_job() {
        let progress = ProgressHandle::new();
        let first = progress.begin("decode");
        let second = progress.begin("building peaks");
        progress.set_fraction(first, 1.0);
        progress.finish(first);
        let snap = progress.snapshot().unwrap();
        assert_eq!(snap.label, "building peaks");
        assert_eq!(snap.percent(), 0);
        progress.finish(second);
        assert!(progress.snapshot().is_none());
    }

    #[test]
    fn cancel_invalidates_in_flight_job() {
        let progress = ProgressHandle::new();
        let epoch = progress.begin("building peaks");
        progress.cancel();
        assert!(progress.snapshot().is_none());
        progress.set_fraction(epoch, 0.5);
        progress.finish(epoch);
        assert!(progress.snapshot().is_none());
        assert!(!progress.is_epoch(epoch));
    }
}
