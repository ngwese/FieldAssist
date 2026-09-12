// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Playhead position and in/out loop bounds.

use std::sync::Arc;

use super::provider::PlaybackDataProvider;

/// Result of advancing the playhead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayheadEvent {
    /// Advanced normally.
    Ok,
    /// Wrapped to in-point while looping.
    Looped,
    /// Hit the end without looping.
    ReachedEnd,
}

/// Sample-accurate playhead with optional in/out region.
pub struct Playhead {
    provider: Arc<dyn PlaybackDataProvider>,
    position: usize,
    in_point: Option<usize>,
    out_point: Option<usize>,
    looping: bool,
}

impl Playhead {
    /// Create a playhead bound to `provider`.
    pub fn new(provider: Arc<dyn PlaybackDataProvider>) -> Self {
        Self {
            provider,
            position: 0,
            in_point: None,
            out_point: None,
            looping: false,
        }
    }

    /// Underlying provider.
    pub fn provider(&self) -> &Arc<dyn PlaybackDataProvider> {
        &self.provider
    }

    /// Current sample position.
    pub fn position(&self) -> usize {
        self.position
    }

    /// Region in-point, if set.
    pub fn in_point(&self) -> Option<usize> {
        self.in_point
    }

    /// Region out-point, if set.
    pub fn out_point(&self) -> Option<usize> {
        self.out_point
    }

    /// Whether looping is enabled.
    pub fn looping(&self) -> bool {
        self.looping
    }

    /// Set position (clamped).
    pub fn set_position(&mut self, sample: usize) {
        self.position = self.clamp_sample(sample);
    }

    /// Set in-point.
    pub fn set_in(&mut self, sample: usize) {
        self.in_point = Some(self.clamp_sample(sample));
    }

    /// Set out-point.
    pub fn set_out(&mut self, sample: usize) {
        self.out_point = Some(self.clamp_sample(sample));
    }

    /// Clear in/out region.
    pub fn clear_in_out(&mut self) {
        self.in_point = None;
        self.out_point = None;
    }

    /// Set both in and out (ordered).
    pub fn set_in_out(&mut self, start: usize, end: usize) {
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        self.in_point = Some(self.clamp_sample(start));
        self.out_point = Some(self.clamp_sample(end));
    }

    /// Enable or disable looping.
    pub fn set_looping(&mut self, looping: bool) {
        self.looping = looping;
    }

    /// Toggle looping.
    pub fn toggle_looping(&mut self) {
        self.looping = !self.looping;
    }

    /// Start sample for playback (in-point or 0).
    pub fn playback_start(&self) -> usize {
        self.in_point.unwrap_or(0)
    }

    /// Whether the playhead is at the playback end.
    pub fn is_at_end(&self) -> bool {
        self.position >= self.playback_end()
    }

    /// End sample for transport **End** command (region out or buffer end).
    pub fn transport_end(&self) -> usize {
        self.out_point.unwrap_or_else(|| self.max_sample())
    }

    /// End sample for playback stopping / looping (region out only when looping).
    pub fn playback_end(&self) -> usize {
        if self.looping {
            self.transport_end()
        } else {
            self.max_sample()
        }
    }

    /// Clamp to valid sample range.
    pub fn clamp_sample(&self, sample: usize) -> usize {
        sample.min(self.max_sample())
    }

    fn max_sample(&self) -> usize {
        self.provider.frames().saturating_sub(1)
    }

    /// Advance by `frames_played` samples.
    pub fn advance(&mut self, frames_played: usize) -> PlayheadEvent {
        if frames_played == 0 {
            return PlayheadEvent::Ok;
        }
        let end = self.playback_end();
        let start = self.playback_start();
        let next = self.position.saturating_add(frames_played);
        if next >= end {
            if self.looping && end > start {
                self.position = start;
                return PlayheadEvent::Looped;
            }
            self.position = end;
            return PlayheadEvent::ReachedEnd;
        }
        self.position = next;
        PlayheadEvent::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Silence {
        frames: usize,
    }

    impl PlaybackDataProvider for Silence {
        fn sample_rate(&self) -> u32 {
            44100
        }
        fn channel_count(&self) -> usize {
            1
        }
        fn frames(&self) -> usize {
            self.frames
        }
        fn read_interleaved(&self, _start: usize, _count: usize, dest: &mut [f32]) {
            dest.fill(0.0);
        }
    }

    fn playhead(frames: usize) -> Playhead {
        Playhead::new(Arc::new(Silence { frames }))
    }

    #[test]
    fn loops_inside_in_out() {
        let mut ph = playhead(1000);
        ph.set_in_out(100, 200);
        ph.set_looping(true);
        ph.set_position(195);
        assert_eq!(ph.advance(10), PlayheadEvent::Looped);
        assert_eq!(ph.position(), 100);
    }

    #[test]
    fn stops_at_buffer_end_without_loop_even_with_in_out() {
        let mut ph = playhead(1000);
        ph.set_in_out(100, 200);
        ph.set_position(995);
        assert_eq!(ph.advance(10), PlayheadEvent::ReachedEnd);
        assert_eq!(ph.position(), 999);
        assert!(ph.is_at_end());
    }

    #[test]
    fn is_at_end_is_false_before_last_sample() {
        let mut ph = playhead(1000);
        ph.set_position(998);
        assert!(!ph.is_at_end());
        ph.set_position(999);
        assert!(ph.is_at_end());
    }
}
