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

/// How far beyond an edited range an op must rewrite for continuity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionalRecompute {
    /// Output frames on each side of the edited range that must be rewritten.
    pub radius_frames: u64,
    /// Extra input frames consumed before the first rewritten hop (not emitted).
    pub warmup_frames: u64,
}

/// Whether an analysis op needs a full-timeline pass or only dirty neighborhoods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecomputeScope {
    /// Drop the stream and rebuild the whole timeline (or leave markers alone).
    FullTimeline,
    /// Splice hop streams and rebuild radius-padded dirty ranges only.
    Regional(RegionalRecompute),
}

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

    /// Progress label for a multi-kind analysis pass (stable order).
    pub fn progress_label_for_kinds(kinds: &[Self]) -> String {
        let mut ordered = kinds.to_vec();
        ordered.sort_by_key(|k| k.id());
        ordered.dedup();
        match ordered.as_slice() {
            [] => "analyzing".into(),
            [one] => one.progress_label().into(),
            [Self::MinMax, Self::Spectral] => "building peaks + spectrum".into(),
            many => many
                .iter()
                .map(|k| k.progress_label())
                .collect::<Vec<_>>()
                .join(" + "),
        }
    }

    /// How this op invalidates and rebuilds after sample-changing edits.
    pub fn recompute_scope(self, sample_rate: u32) -> RecomputeScope {
        match self {
            Self::MinMax => RecomputeScope::Regional(RegionalRecompute {
                radius_frames: 0,
                warmup_frames: 0,
            }),
            Self::EnvelopePeak => {
                let warmup = ((EnvelopePeakOp::RELEASE_SECS * sample_rate as f32) as u64).max(1);
                RecomputeScope::Regional(RegionalRecompute {
                    radius_frames: warmup,
                    warmup_frames: warmup,
                })
            }
            Self::Transients => RecomputeScope::FullTimeline,
            Self::Spectral => {
                let lookback = (SPECTRAL_FFT_SIZE.saturating_sub(PEAK_BLOCK)) as u64;
                RecomputeScope::Regional(RegionalRecompute {
                    radius_frames: lookback,
                    warmup_frames: lookback,
                })
            }
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
