// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Audio I/O and peak helpers re-exported from library crates.
//!
//! [`WaveformDataProvider`] lives in [`field_ui_components`]. Implementations for
//! app-local types (e.g. [`crate::model::BufferDocument`]) stay here; orphan
//! rules prevent implementing that trait for foreign model buffers in this crate.

#[allow(unused_imports)] // compatibility re-exports for app call sites
pub use field_audio_io::{
    decode, decode_range, load_buffer, probe_file, probe_header, DecodedAudio, ProbedFile,
    SymphoniaBlockSource,
};
#[allow(unused_imports)] // compatibility re-exports for app call sites
pub use field_audio_process::{
    build_peaks, min_max_in_range, PEAK_BLOCK, SPECTRAL_BAND_COUNT, SPECTRAL_DB_FLOOR,
};
