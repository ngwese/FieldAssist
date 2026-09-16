// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! In-memory analysis stream caches (not persisted in `.facomp`).

use std::collections::HashMap;

use field_audio_process::{
    AnalysisKind, PEAK_BLOCK, SPECTRAL_BAND_COUNT, SPECTRAL_DB_FLOOR, SPECTRAL_FFT_SIZE,
};

/// Per-channel hop-binned float stream (envelope, etc.).
#[derive(Debug, Clone, Default)]
pub struct FloatStreamSeries {
    /// Hop size in frames used when the bins were produced.
    pub hop: usize,
    /// Frames covered (may exceed `bins.len() * hop` by a partial last hop).
    pub covered_frames: u64,
    /// Per-channel bins.
    pub channels: Vec<Vec<f32>>,
}

impl FloatStreamSeries {
    /// Empty series for `channel_count` channels at `hop`.
    pub fn empty(channel_count: usize, hop: usize) -> Self {
        Self {
            hop,
            covered_frames: 0,
            channels: vec![Vec::new(); channel_count],
        }
    }

    /// Whether the stream covers `frames` timeline samples.
    pub fn covers(&self, frames: u64) -> bool {
        self.covered_frames >= frames && !self.channels.is_empty()
    }

    /// Whether any hop bins are available to paint.
    pub fn has_data(&self) -> bool {
        self.covered_frames > 0 && self.channels.iter().any(|ch| !ch.is_empty())
    }

    /// Hops that may be read for paint (clamped to coverage).
    pub fn covered_hops(&self) -> usize {
        if self.hop == 0 {
            return 0;
        }
        ((self.covered_frames as usize) + self.hop - 1) / self.hop
    }
}

/// Per-channel hop × band spectrogram (dB), packed row-major.
#[derive(Debug, Clone, Default)]
pub struct SpectralStreamSeries {
    /// Hop size in frames.
    pub hop: usize,
    /// FFT size used when the series was produced.
    pub fft_size: usize,
    /// Log bands per hop.
    pub band_count: usize,
    /// Frames covered by the series.
    pub covered_frames: u64,
    /// Per channel: packed `hop_count * band_count` dB values.
    pub channels: Vec<Vec<f32>>,
}

impl SpectralStreamSeries {
    /// Empty series for `channel_count` channels.
    pub fn empty(channel_count: usize) -> Self {
        Self {
            hop: PEAK_BLOCK,
            fft_size: SPECTRAL_FFT_SIZE,
            band_count: SPECTRAL_BAND_COUNT,
            covered_frames: 0,
            channels: vec![Vec::new(); channel_count],
        }
    }

    /// Whether the stream covers `frames` timeline samples with the expected shape.
    pub fn covers(&self, frames: u64, channel_count: usize) -> bool {
        self.covered_frames >= frames
            && self.channels.len() == channel_count
            && self.band_count == SPECTRAL_BAND_COUNT
            && self.hop > 0
            && !self.channels.is_empty()
    }

    /// Whether any spectral hops are available to paint.
    pub fn has_data(&self) -> bool {
        self.covered_frames > 0 && self.channels.iter().any(|ch| !ch.is_empty())
    }

    /// Hops that may be read for paint (clamped to coverage).
    pub fn covered_hops(&self) -> usize {
        if self.hop == 0 {
            return 0;
        }
        ((self.covered_frames as usize) + self.hop - 1) / self.hop
    }

    /// Number of spectral hops stored for `channel`.
    pub fn hop_count(&self, channel: usize) -> usize {
        let Some(data) = self.channels.get(channel) else {
            return 0;
        };
        if self.band_count == 0 {
            return 0;
        }
        data.len() / self.band_count
    }
}

/// Composition-level analysis streams (envelope peak, spectral, …). Min/max
/// overview bins remain on [`crate::ClipCache`].
#[derive(Debug, Clone, Default)]
pub struct AnalysisStreams {
    float_streams: HashMap<AnalysisKind, FloatStreamSeries>,
    spectral: Option<SpectralStreamSeries>,
}

impl AnalysisStreams {
    /// Clear all derived streams (call after sample-changing edits).
    pub fn clear(&mut self) {
        self.float_streams.clear();
        self.spectral = None;
    }

    /// Look up a float stream by kind.
    pub fn float(&self, kind: AnalysisKind) -> Option<&FloatStreamSeries> {
        self.float_streams.get(&kind)
    }

    /// Mutable look up a float stream by kind.
    pub fn float_mut(&mut self, kind: AnalysisKind) -> Option<&mut FloatStreamSeries> {
        self.float_streams.get_mut(&kind)
    }

    /// Ensure a float stream slot exists for `kind`.
    pub fn ensure_float(
        &mut self,
        kind: AnalysisKind,
        channel_count: usize,
        hop: usize,
    ) -> &mut FloatStreamSeries {
        self.float_streams
            .entry(kind)
            .or_insert_with(|| FloatStreamSeries::empty(channel_count, hop))
    }

    /// Whether envelope-peak data covers the composition length.
    pub fn envelope_peak_ready(&self, frames: u64, channel_count: usize) -> bool {
        self.float(AnalysisKind::EnvelopePeak)
            .is_some_and(|s| s.channels.len() == channel_count && s.covers(frames))
    }

    /// Whether any envelope-peak bins are available to paint.
    pub fn envelope_peak_has_data(&self, channel_count: usize) -> bool {
        self.float(AnalysisKind::EnvelopePeak)
            .is_some_and(|s| s.channels.len() == channel_count && s.has_data())
    }

    /// Default hop for envelope peak bins.
    pub fn envelope_hop() -> usize {
        PEAK_BLOCK
    }

    /// Look up the spectral stream.
    pub fn spectral(&self) -> Option<&SpectralStreamSeries> {
        self.spectral.as_ref()
    }

    /// Mutable look up the spectral stream.
    pub fn spectral_mut(&mut self) -> Option<&mut SpectralStreamSeries> {
        self.spectral.as_mut()
    }

    /// Ensure a spectral stream slot exists.
    pub fn ensure_spectral(&mut self, channel_count: usize) -> &mut SpectralStreamSeries {
        if self
            .spectral
            .as_ref()
            .is_none_or(|s| s.channels.len() != channel_count)
        {
            self.spectral = Some(SpectralStreamSeries::empty(channel_count));
        }
        self.spectral.as_mut().expect("spectral just ensured")
    }

    /// Whether spectral data covers the composition length.
    pub fn spectral_ready(&self, frames: u64, channel_count: usize) -> bool {
        self.spectral()
            .is_some_and(|s| s.covers(frames, channel_count))
    }

    /// Whether any spectral hops are available to paint.
    pub fn spectral_has_data(&self, channel_count: usize) -> bool {
        self.spectral()
            .is_some_and(|s| s.channels.len() == channel_count && s.has_data())
    }

    /// Default hop for spectral frames.
    pub fn spectral_hop() -> usize {
        PEAK_BLOCK
    }

    /// Floor used when a spectral column has no data.
    pub fn spectral_db_floor() -> f32 {
        SPECTRAL_DB_FLOOR
    }
}
