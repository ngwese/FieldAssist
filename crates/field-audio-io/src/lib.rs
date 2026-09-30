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
mod metadata;
mod pcm;
#[cfg(test)]
mod roundtrip;
mod spec;
mod stream;
mod transcode;

use std::io::Write;

use anyhow::Result;

pub use decode::{
    decode, decode_range, load_buffer, probe_file, probe_header, DecodedAudio, ProbedFile,
    SymphoniaBlockSource,
};
pub use metadata::{
    build_tag_map, probe_source_variables, TagMap, TechnicalSourceFields, CANONICAL_KEYS,
};
pub use pcm::{
    bits_for_integer, clamp_unit, interleave_f32, interleave_i32, planar_frames, select_channels,
    to_signed, to_u8,
};
pub use spec::{
    format_rate, snap_format, spec_supported, EncodeSpec, EncoderCaps, PcmFormat, RATE_PRESETS,
};
pub use stream::{begin_stream, finish_with_tags, EncodeStream};
pub use transcode::{transcode, TranscodeRequest, DEFAULT_BLOCK_FRAMES};

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
    /// Encode planar f32 PCM into `writer` (no tags).
    fn encode(&self, spec: &EncodeSpec, planar: &[Vec<f32>], writer: &mut dyn Write) -> Result<()> {
        self.encode_with_tags(spec, planar, writer, &TagMap::new())
    }
    /// Encode planar f32 PCM with optional metadata tags.
    fn encode_with_tags(
        &self,
        spec: &EncodeSpec,
        planar: &[Vec<f32>],
        writer: &mut dyn Write,
        tags: &TagMap,
    ) -> Result<()>;
}

/// Built-in encoders (WAV, FLAC, Ogg Vorbis).
pub fn encoders() -> &'static [&'static dyn FormatEncoder] {
    encoders::encoders()
}

/// Look up a built-in encoder by id.
pub fn encoder(id: &str) -> Option<&'static dyn FormatEncoder> {
    encoders::encoder(id)
}
