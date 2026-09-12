// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Source audio trait for the playback engine.

/// Supplies interleaved PCM frames to [`crate::PlaybackEngine`].
pub trait PlaybackDataProvider: Send + Sync {
    /// Source sample rate in Hz.
    fn sample_rate(&self) -> u32;
    /// Number of interleaved channels.
    fn channel_count(&self) -> usize;
    /// Total frame count.
    fn frames(&self) -> usize;
    /// Read `count` frames starting at `start` into `dest` (interleaved).
    fn read_interleaved(&self, start: usize, count: usize, dest: &mut [f32]);
}
