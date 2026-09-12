// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::io::Write;

use anyhow::{bail, Context, Result};
use flacenc::bitsink::ByteSink;
use flacenc::component::BitRepr;
use flacenc::config;
use flacenc::encode_with_fixed_block_size;
use flacenc::error::Verify;
use flacenc::source::MemSource;

use crate::pcm::{bits_for_integer, interleave_i32, planar_frames};
use crate::spec::{EncodeSpec, EncoderCaps, PcmFormat};
use crate::FormatEncoder;

/// FLAC encoder using `flacenc`.
pub struct FlacEncoder;

const FORMATS: &[PcmFormat] = &[PcmFormat::S8, PcmFormat::S16, PcmFormat::S24];

impl FormatEncoder for FlacEncoder {
    fn id(&self) -> &'static str {
        "flac"
    }

    fn label(&self) -> &'static str {
        "FLAC"
    }

    fn extension(&self) -> &'static str {
        "flac"
    }

    fn capabilities(&self) -> EncoderCaps {
        EncoderCaps {
            sample_formats: Some(FORMATS),
            min_sample_rate: 1,
            max_sample_rate: 1_048_575,
            min_channels: 1,
            max_channels: 8,
        }
    }

    fn encode(&self, spec: &EncodeSpec, planar: &[Vec<f32>], writer: &mut dyn Write) -> Result<()> {
        let format = spec
            .sample_format
            .context("FLAC requires a sample format")?;
        if !self.supports(spec) {
            bail!(
                "FLAC cannot encode {format:?} at {} Hz {} ch",
                spec.sample_rate,
                spec.channel_count
            );
        }
        let bits = bits_for_integer(format).context("FLAC requires signed integer PCM")?;
        let samples = interleave_i32(planar, bits);
        let frames = planar_frames(planar);
        let source = MemSource::from_samples(
            &samples,
            planar.len(),
            bits as usize,
            spec.sample_rate as usize,
        );
        let mut encoder_cfg = config::Encoder::default();
        encoder_cfg.multithread = false;
        if frames > 0 && frames < encoder_cfg.block_size {
            encoder_cfg.block_size = frames.max(16);
        }
        let config = encoder_cfg
            .into_verified()
            .map_err(|(_, err)| anyhow::anyhow!("invalid FLAC encoder config: {err}"))?;
        let stream = encode_with_fixed_block_size(&config, source, config.block_size)
            .map_err(|err| anyhow::anyhow!("FLAC encode failed: {err}"))?;
        let mut sink = ByteSink::new();
        stream
            .write(&mut sink)
            .map_err(|err| anyhow::anyhow!("FLAC write failed: {err}"))?;
        if sink.as_slice().is_empty() {
            bail!("FLAC encoder produced no bytes");
        }
        writer
            .write_all(sink.as_slice())
            .context("write FLAC bytes")?;
        Ok(())
    }
}
