// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

mod flac;
mod vorbis;
mod wav;

use crate::FormatEncoder;
use flac::FlacEncoder;
use vorbis::VorbisEncoderImpl;
use wav::WavEncoder;

static WAV: WavEncoder = WavEncoder;
static FLAC: FlacEncoder = FlacEncoder;
static VORBIS: VorbisEncoderImpl = VorbisEncoderImpl;
static ENCODERS: [&dyn FormatEncoder; 3] = [&WAV, &FLAC, &VORBIS];

pub(crate) fn encoders() -> &'static [&'static dyn FormatEncoder] {
    &ENCODERS
}

pub(crate) fn encoder(id: &str) -> Option<&'static dyn FormatEncoder> {
    encoders()
        .iter()
        .copied()
        .find(|encoder| encoder.id() == id)
}
