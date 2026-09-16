// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Incremental analysis operations over planar PCM blocks.

mod envelope;
mod minmax;
mod spectral;
mod transient;

pub use envelope::EnvelopePeakOp;
pub use minmax::{build_peaks, fold_minmax_bins, min_max_in_range, MinMaxOp, PEAK_BLOCK};
pub use spectral::{
    spectral_band_for_freq, SpectralOp, SPECTRAL_BAND_COUNT, SPECTRAL_DB_FLOOR, SPECTRAL_FFT_SIZE,
    SPECTRAL_FMIN_HZ,
};
pub use transient::TransientDetectOp;

use field_audio_model::NewMarker;

/// Stable id for a built-in analysis operation / stream kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisKind {
    /// Overview min/max bins (waveform paint).
    MinMax,
    /// Peak envelope follower with a fixed smoothing window.
    EnvelopePeak,
    /// Onset / transient marker detection.
    Transients,
    /// Log-band spectrogram stream (Spectrum representation).
    Spectral,
}

impl AnalysisKind {
    /// Stable string id used in caches and progress labels.
    pub fn id(self) -> &'static str {
        match self {
            Self::MinMax => "minmax",
            Self::EnvelopePeak => "envelope_peak",
            Self::Transients => "transients",
            Self::Spectral => "spectral",
        }
    }

    /// Human-readable background job label.
    pub fn progress_label(self) -> &'static str {
        match self {
            Self::MinMax => "building peaks",
            Self::EnvelopePeak => "building envelope",
            Self::Transients => "detecting transients",
            Self::Spectral => "building spectrum",
        }
    }
}

/// Shared context passed at the start of an analysis pass.
#[derive(Debug, Clone, Copy)]
pub struct AnalysisCtx {
    /// Composition sample rate in Hz.
    pub sample_rate: u32,
    /// Number of planar channels in the read path.
    pub channel_count: usize,
}

/// Sink for discrete analysis outputs produced at pass end (or mid-pass flush).
#[derive(Debug, Default)]
pub struct AnalysisSink {
    /// Markers to insert after replacing the prior set in-range.
    pub markers: Vec<NewMarker>,
}
