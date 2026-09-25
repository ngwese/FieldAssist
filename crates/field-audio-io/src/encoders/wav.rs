// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::io::{Cursor, Write};

use anyhow::{bail, Context, Result};
use hound::{SampleFormat, WavSpec, WavWriter};

use crate::pcm::{clamp_unit, planar_frames, to_signed, to_u8};
use crate::spec::{EncodeSpec, EncoderCaps, PcmFormat};
use crate::FormatEncoder;

/// WAV encoder using `hound`.
pub struct WavEncoder;

const FORMATS: &[PcmFormat] = &[
    PcmFormat::U8,
    PcmFormat::S16,
    PcmFormat::S24,
    PcmFormat::S32,
    PcmFormat::F32,
];

impl FormatEncoder for WavEncoder {
    fn id(&self) -> &'static str {
        "wav"
    }

    fn label(&self) -> &'static str {
        "WAV"
    }

    fn extension(&self) -> &'static str {
        "wav"
    }

    fn capabilities(&self) -> EncoderCaps {
        EncoderCaps {
            sample_formats: Some(FORMATS),
            min_sample_rate: 1,
            max_sample_rate: u32::MAX,
            min_channels: 1,
            max_channels: u16::MAX,
        }
    }

    fn encode(&self, spec: &EncodeSpec, planar: &[Vec<f32>], writer: &mut dyn Write) -> Result<()> {
        let format = spec.sample_format.context("WAV requires a sample format")?;
        if !self.supports(spec) {
            bail!(
                "WAV cannot encode {format:?} at {} Hz {} ch",
                spec.sample_rate,
                spec.channel_count
            );
        }
        let (bits_per_sample, sample_format) = match format {
            PcmFormat::U8 => (8, SampleFormat::Int),
            PcmFormat::S16 => (16, SampleFormat::Int),
            PcmFormat::S24 => (24, SampleFormat::Int),
            PcmFormat::S32 => (32, SampleFormat::Int),
            PcmFormat::F32 => (32, SampleFormat::Float),
            other => bail!("WAV does not support {other:?}"),
        };
        let wav_spec = WavSpec {
            channels: spec.channel_count,
            sample_rate: spec.sample_rate,
            bits_per_sample,
            sample_format,
        };
        let mut cursor = Cursor::new(Vec::new());
        let mut wav = WavWriter::new(&mut cursor, wav_spec).context("create WAV writer")?;
        let frames = planar_frames(planar);
        let channels = planar.len();
        for frame in 0..frames {
            for ch in 0..channels {
                let sample = planar[ch][frame];
                match format {
                    PcmFormat::U8 => {
                        // Hound's i8 Sample API expects mid-biased signed values;
                        // it converts to unsigned WAV bytes via +128.
                        let signed = (i16::from(to_u8(sample)) - 128) as i8;
                        wav.write_sample(signed).context("write WAV sample")?;
                    }
                    PcmFormat::S16 => {
                        wav.write_sample(to_signed(sample, 16) as i16)
                            .context("write WAV sample")?;
                    }
                    PcmFormat::S24 | PcmFormat::S32 => {
                        wav.write_sample(to_signed(sample, u32::from(format.bits())))
                            .context("write WAV sample")?;
                    }
                    PcmFormat::F32 => {
                        wav.write_sample(clamp_unit(sample))
                            .context("write WAV sample")?;
                    }
                    _ => unreachable!(),
                }
            }
        }
        wav.finalize().context("finalize WAV")?;
        writer
            .write_all(&cursor.into_inner())
            .context("write WAV bytes")?;
        Ok(())
    }
}
