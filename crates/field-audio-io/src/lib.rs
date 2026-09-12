// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

#![deny(missing_docs)]

//! # field-audio-io
//!
//! Probe, decode, and encode audio files for FieldAssist. Decoding is
//! decode-agnostic at the model boundary via [`BlockSource`](field_audio_model::BlockSource).
//!
//! ```
//! use field_audio_io::{encoder, EncodeSpec, PcmFormat};
//!
//! let wav = encoder("wav").expect("wav encoder");
//! let spec = EncodeSpec {
//!     sample_rate: 48_000,
//!     sample_format: Some(PcmFormat::S16),
//!     channel_count: 2,
//! };
//! assert!(wav.supports(&spec));
//! ```

mod decode;
mod encoders;
mod pcm;
mod spec;

use std::io::Write;

use anyhow::Result;

pub use decode::{
    decode, decode_range, load_buffer, probe_file, probe_header, DecodedAudio, ProbedFile,
    SymphoniaBlockSource,
};
pub use pcm::{
    bits_for_integer, clamp_unit, interleave_f32, interleave_i32, planar_frames, select_channels,
    to_signed, to_u8,
};
pub use spec::{
    format_rate, snap_format, spec_supported, EncodeSpec, EncoderCaps, PcmFormat, RATE_PRESETS,
};

/// Streaming / buffer encoder for a container format.
pub trait FormatEncoder: Send + Sync {
    /// Stable encoder id (e.g. `"wav"`).
    fn id(&self) -> &'static str;
    /// Human-readable label.
    fn label(&self) -> &'static str;
    /// Default file extension without a leading dot.
    fn extension(&self) -> &'static str;
    /// Capability limits for UI and validation.
    fn capabilities(&self) -> EncoderCaps;
    /// Whether `spec` is within [`capabilities`](Self::capabilities).
    fn supports(&self, spec: &EncodeSpec) -> bool {
        spec_supported(self.capabilities(), spec)
    }
    /// Encode planar f32 PCM into `writer`.
    fn encode(&self, spec: &EncodeSpec, planar: &[Vec<f32>], writer: &mut dyn Write) -> Result<()>;
}

/// Built-in encoders (WAV, FLAC, Ogg Vorbis).
pub fn encoders() -> &'static [&'static dyn FormatEncoder] {
    encoders::encoders()
}

/// Look up a built-in encoder by id.
pub fn encoder(id: &str) -> Option<&'static dyn FormatEncoder> {
    encoders::encoder(id)
}
