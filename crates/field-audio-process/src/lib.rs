// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

#![deny(missing_docs)]

//! # field-audio-process
//!
//! DSP helpers for FieldAssist: overview peak folding, analysis ops, and
//! planar resampling.
//!
//! ```
//! use field_audio_process::{build_peaks, PEAK_BLOCK};
//!
//! let samples = vec![0.0f32, 0.5, -0.25, 1.0];
//! let peaks = build_peaks(&samples);
//! assert!(!peaks.is_empty() || samples.len() < PEAK_BLOCK);
//! ```

mod analysis;
mod peaks;
mod resample;

pub use analysis::{
    fold_minmax_bins, AnalysisCtx, AnalysisKind, AnalysisMarker, AnalysisSink, EnvelopePeakOp,
    TransientDetectOp,
};
pub use peaks::{build_peaks, min_max_in_range, PEAK_BLOCK};
pub use resample::resample_planar;
