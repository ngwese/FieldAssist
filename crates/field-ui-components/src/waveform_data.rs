// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Waveform paint/read provider trait (UI-facing, gpui-free).

/// Sample access for waveform overview and zoomed paints.
///
/// Leaf UI widgets depend on this trait; applications implement it for their
/// document or buffer types.
pub trait WaveformDataProvider: Send + Sync {
    /// Sample rate in Hz.
    fn sample_rate(&self) -> u32;
    /// Number of planar channels.
    fn channel_count(&self) -> usize;
    /// Total frame count.
    fn frames(&self) -> usize;
    /// Duration in seconds.
    fn duration_secs(&self) -> f64;
    /// Display label for `channel`.
    fn channel_label(&self, channel: usize) -> String;
    /// Copy samples starting at `start` into `dest`.
    fn read_channel(&self, channel: usize, start: usize, dest: &mut [f32]);
    /// Min/max amplitude in `[start, end)`.
    fn min_max_in_range(&self, channel: usize, start: f64, end: f64) -> (f32, f32);
    /// Overview paint needs peak bins; sample-accurate zoom still reads PCM.
    fn peaks_ready(&self) -> bool {
        true
    }
    /// Fill column min/max pairs for overview painting.
    fn fill_minmax_columns(
        &self,
        channel: usize,
        start: f64,
        samples_per_pixel: f64,
        dest: &mut [(f32, f32)],
    ) {
        for (i, slot) in dest.iter_mut().enumerate() {
            let a = start + i as f64 * samples_per_pixel;
            *slot = self.min_max_in_range(channel, a, a + samples_per_pixel);
        }
    }
}
