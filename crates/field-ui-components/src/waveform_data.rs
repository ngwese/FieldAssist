// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Waveform paint/read provider trait (UI-facing, gpui-free).

/// Which body the waveform lanes paint (View menu representation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WaveformRepresentation {
    /// Min/max peak columns (and sample-accurate zoom).
    #[default]
    Peaks,
    /// Time × frequency heatmap from the spectral stream.
    Spectrum,
    /// Peaks on top of each lane, spectrum below, with a shared splitter.
    PeaksSpectrum,
}

/// Default peaks-pane height fraction for [`WaveformRepresentation::PeaksSpectrum`].
pub const DEFAULT_PEAKS_SPECTRUM_SPLIT: f32 = 0.2;
/// Minimum peaks-pane height fraction when dragging the shared splitter.
pub const MIN_PEAKS_SPECTRUM_SPLIT: f32 = 0.15;
/// Maximum peaks-pane height fraction when dragging the shared splitter.
pub const MAX_PEAKS_SPECTRUM_SPLIT: f32 = 0.85;

/// Clamp a peaks/spectrum lane split fraction into the allowed range.
pub fn clamp_peaks_spectrum_split(fraction: f32) -> f32 {
    fraction.clamp(MIN_PEAKS_SPECTRUM_SPLIT, MAX_PEAKS_SPECTRUM_SPLIT)
}

/// Sample access for waveform overview and zoomed paints.
///
/// Leaf UI widgets depend on this trait; applications implement it for their
/// document or buffer types.
///
/// Progressive analysis: `*_has_data` means bins exist to paint now;
/// `ensure_*` / `needs` jobs fill coverage while the UI consumes partial
/// results. Full-timeline readiness is only required to stop requesting work.
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
    /// Whether any overview peak bins are available to paint (may be partial).
    fn peaks_ready(&self) -> bool {
        true
    }
    /// Whether overview peak analysis still needs work.
    fn peaks_complete(&self) -> bool {
        self.peaks_ready()
    }
    /// Request overview min/max analysis when paint needs it (pull-based).
    fn ensure_minmax_peaks(&self) {}
    /// Active waveform body representation (Peaks vs Spectrum).
    fn waveform_representation(&self) -> WaveformRepresentation {
        WaveformRepresentation::Peaks
    }
    /// Shared peaks-pane height fraction for [`WaveformRepresentation::PeaksSpectrum`].
    fn peaks_spectrum_split(&self) -> f32 {
        DEFAULT_PEAKS_SPECTRUM_SPLIT
    }
    /// Whether the peak-envelope overlay should be drawn.
    fn envelope_overlay_enabled(&self) -> bool {
        false
    }
    /// Whether any envelope-peak bins are available to paint (may be partial).
    fn envelope_ready(&self) -> bool {
        false
    }
    /// Whether envelope-peak analysis still needs work.
    fn envelope_complete(&self) -> bool {
        self.envelope_ready()
    }
    /// Request envelope-peak analysis when the overlay is on and data is missing.
    fn ensure_envelope_peak(&self) {}
    /// Fill per-column envelope amplitudes for overlay paint.
    fn fill_envelope_columns(
        &self,
        _channel: usize,
        _start: f64,
        _samples_per_pixel: f64,
        dest: &mut [f32],
    ) {
        dest.fill(0.0);
    }
    /// Whether any spectral hops are available to paint (may be partial).
    fn spectral_ready(&self) -> bool {
        false
    }
    /// Whether spectral analysis still needs work.
    fn spectral_complete(&self) -> bool {
        self.spectral_ready()
    }
    /// Monotonic coverage fingerprint for spectrum cache keys (e.g. covered frames).
    fn spectral_coverage_frames(&self) -> u64 {
        0
    }
    /// Request spectral analysis when Spectrum representation needs data.
    fn ensure_spectral(&self) {}
    /// Log bands per spectral hop (must match analysis stream packing).
    fn spectral_band_count(&self) -> usize {
        64
    }
    /// dB floor used when a spectral cell has no data.
    fn spectral_db_floor(&self) -> f32 {
        -80.0
    }
    /// Fill packed `width * band_count` dB columns for Spectrum paint.
    fn fill_spectral_columns(
        &self,
        _channel: usize,
        _start: f64,
        _samples_per_pixel: f64,
        dest: &mut [f32],
    ) {
        dest.fill(self.spectral_db_floor());
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
    /// Samples folded into each overview peak bin (default `256`).
    ///
    /// Waveform paint uses this instead of importing audio-process constants.
    fn peak_block(&self) -> usize {
        256
    }
}
