// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

#![deny(missing_docs)]

//! # field-audio-process
//!
//! DSP helpers for FieldAssist: analysis ops (minmax peaks, envelope,
//! transients, spectral) and planar resampling.
//!
//! ```
//! use field_audio_process::{build_peaks, PEAK_BLOCK};
//!
//! let samples = vec![0.0f32, 0.5, -0.25, 1.0];
//! let peaks = build_peaks(&samples);
//! assert!(!peaks.is_empty() || samples.len() < PEAK_BLOCK);
//! ```

mod analysis;
mod resample;
mod synth;

pub use analysis::{
    build_peaks, fold_minmax_bins, min_max_in_range, spectral_band_for_freq, AnalysisCtx,
    AnalysisKind, AnalysisSink, EnvelopePeakOp, MinMaxOp, SpectralOp, TransientDetectOp,
    PEAK_BLOCK, SPECTRAL_BAND_COUNT, SPECTRAL_DB_FLOOR, SPECTRAL_FFT_SIZE, SPECTRAL_FMIN_HZ,
};
pub use resample::resample_planar;
pub use synth::{
    chirp, dc, goertzel_power, impulse, left_impulse, max_abs_err, nyquist_square, rms, silence,
    sine, sine_planar, sine_snr_db, tone_to_image_db, two_tone,
};
