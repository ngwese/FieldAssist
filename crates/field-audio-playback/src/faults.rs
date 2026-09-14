// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Host-side coalescing of playback underruns and CPAL stream errors.
//!
//! The realtime callback only bumps atomics. [`PlaybackShared::take_faults`]
//! drains those counters on a non-realtime thread; this flusher rate-limits
//! the resulting log lines so a burst of xruns does not flood the UI.

use std::time::{Duration, Instant};

/// Default window for coalescing playback fault messages.
pub const PLAYBACK_FAULT_LOG_INTERVAL: Duration = Duration::from_secs(1);

/// Underrun / stream-error counts since the last
/// [`crate::PlaybackShared::take_faults`] call.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlaybackFaults {
    /// Callbacks that starved on the prefetch ring (not end-of-timeline).
    pub underruns: u64,
    /// CPAL stream error callback invocations.
    pub stream_errors: u64,
    /// Most recent CPAL stream error `Display` text, if any.
    pub last_stream_error: Option<String>,
}

/// Severity of a coalesced playback fault line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackFaultLevel {
    /// Prefetch underrun (audio was starved).
    Warn,
    /// CPAL stream error / device fault.
    Error,
}

/// One host log line produced by [`PlaybackFaultFlusher`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackFaultMessage {
    /// Warn for underruns, error for stream faults.
    pub level: PlaybackFaultLevel,
    /// Human-readable body (no topic); hosts typically use topic `playback`.
    pub text: String,
}

/// Accumulates [`PlaybackFaults`] and emits at most one underrun line and one
/// stream-error line per [`PLAYBACK_FAULT_LOG_INTERVAL`] (after an immediate
/// first report).
#[derive(Debug)]
pub struct PlaybackFaultFlusher {
    pending_underruns: u64,
    pending_stream_errors: u64,
    last_stream_error: Option<String>,
    last_emit: Option<Instant>,
    interval: Duration,
}

impl Default for PlaybackFaultFlusher {
    fn default() -> Self {
        Self::new(PLAYBACK_FAULT_LOG_INTERVAL)
    }
}

impl PlaybackFaultFlusher {
    /// Create a flusher with an explicit coalescing interval.
    pub fn new(interval: Duration) -> Self {
        Self {
            pending_underruns: 0,
            pending_stream_errors: 0,
            last_stream_error: None,
            last_emit: None,
            interval,
        }
    }

    /// Fold a drain from [`crate::PlaybackShared::take_faults`] into the
    /// pending window.
    pub fn push(&mut self, faults: PlaybackFaults) {
        self.pending_underruns = self.pending_underruns.saturating_add(faults.underruns);
        self.pending_stream_errors = self
            .pending_stream_errors
            .saturating_add(faults.stream_errors);
        if let Some(detail) = faults.last_stream_error {
            self.last_stream_error = Some(detail);
        }
    }

    /// If the coalescing window has elapsed (or this is the first batch),
    /// return formatted log lines and clear pending counts.
    pub fn poll(&mut self, now: Instant) -> Vec<PlaybackFaultMessage> {
        if self.pending_underruns == 0 && self.pending_stream_errors == 0 {
            return Vec::new();
        }
        let due = match self.last_emit {
            None => true,
            Some(prev) => now.saturating_duration_since(prev) >= self.interval,
        };
        if !due {
            return Vec::new();
        }
        self.last_emit = Some(now);
        self.drain_messages()
    }

    fn drain_messages(&mut self) -> Vec<PlaybackFaultMessage> {
        let mut out = Vec::new();
        if self.pending_underruns > 0 {
            out.push(PlaybackFaultMessage {
                level: PlaybackFaultLevel::Warn,
                text: format_underrun(self.pending_underruns),
            });
            self.pending_underruns = 0;
        }
        if self.pending_stream_errors > 0 {
            out.push(PlaybackFaultMessage {
                level: PlaybackFaultLevel::Error,
                text: format_stream_error(
                    self.pending_stream_errors,
                    self.last_stream_error.as_deref(),
                ),
            });
            self.pending_stream_errors = 0;
            self.last_stream_error = None;
        }
        out
    }
}

fn format_underrun(count: u64) -> String {
    if count == 1 {
        "prefetch underrun: output callback starved".to_string()
    } else {
        format!("{count} prefetch underruns: output callback starved")
    }
}

fn stream_error_kind(detail: Option<&str>) -> &'static str {
    let Some(detail) = detail else {
        return "xrun / device fault";
    };
    let lower = detail.to_ascii_lowercase();
    if lower.contains("overrun") {
        "output overrun"
    } else if lower.contains("underrun") {
        "output underrun"
    } else if lower.contains("xrun") {
        "xrun"
    } else {
        "xrun / device fault"
    }
}

fn format_stream_error(count: u64, detail: Option<&str>) -> String {
    let kind = stream_error_kind(detail);
    match (count, detail) {
        (1, Some(d)) => format!("stream error ({kind}): {d}"),
        (1, None) => format!("stream error ({kind})"),
        (n, Some(d)) => format!("{n} stream errors ({kind}): last: {d}"),
        (n, None) => format!("{n} stream errors ({kind})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn underruns(n: u64) -> PlaybackFaults {
        PlaybackFaults {
            underruns: n,
            stream_errors: 0,
            last_stream_error: None,
        }
    }

    fn stream_errors(n: u64, detail: &str) -> PlaybackFaults {
        PlaybackFaults {
            underruns: 0,
            stream_errors: n,
            last_stream_error: Some(detail.to_string()),
        }
    }

    #[test]
    fn first_fault_emits_immediately() {
        let mut flusher = PlaybackFaultFlusher::new(Duration::from_secs(1));
        flusher.push(underruns(1));
        let lines = flusher.poll(Instant::now());
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].level, PlaybackFaultLevel::Warn);
        assert_eq!(lines[0].text, "prefetch underrun: output callback starved");
    }

    #[test]
    fn coalesces_underruns_inside_the_window() {
        let t0 = Instant::now();
        let mut flusher = PlaybackFaultFlusher::new(Duration::from_millis(100));
        flusher.push(underruns(1));
        assert_eq!(flusher.poll(t0).len(), 1);

        flusher.push(underruns(2));
        flusher.push(underruns(3));
        assert!(flusher.poll(t0 + Duration::from_millis(50)).is_empty());

        let lines = flusher.poll(t0 + Duration::from_millis(100));
        assert_eq!(lines.len(), 1);
        assert_eq!(
            lines[0].text,
            "5 prefetch underruns: output callback starved"
        );
    }

    #[test]
    fn stream_error_includes_detail_and_overrun_kind() {
        let mut flusher = PlaybackFaultFlusher::default();
        flusher.push(stream_errors(1, "CoreAudio output overrun"));
        let lines = flusher.poll(Instant::now());
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].level, PlaybackFaultLevel::Error);
        assert_eq!(
            lines[0].text,
            "stream error (output overrun): CoreAudio output overrun"
        );
    }

    #[test]
    fn coalesced_stream_errors_keep_last_detail() {
        let t0 = Instant::now();
        let mut flusher = PlaybackFaultFlusher::new(Duration::from_millis(10));
        flusher.push(stream_errors(1, "first"));
        assert_eq!(flusher.poll(t0).len(), 1);
        flusher.push(stream_errors(1, "DeviceNotAvailable"));
        flusher.push(stream_errors(2, "backend xrun"));
        let lines = flusher.poll(t0 + Duration::from_millis(10));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "3 stream errors (xrun): last: backend xrun");
    }

    #[test]
    fn idle_poll_is_empty() {
        let mut flusher = PlaybackFaultFlusher::default();
        assert!(flusher.poll(Instant::now()).is_empty());
    }
}
