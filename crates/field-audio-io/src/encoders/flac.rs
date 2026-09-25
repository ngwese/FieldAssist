// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::io::Write;

use anyhow::{bail, Context, Result};
use flacenc::bitsink::ByteSink;
use flacenc::component::BitRepr;
use flacenc::config;
use flacenc::constant::{MAX_BLOCK_SIZE, MIN_BLOCK_SIZE};
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
        if planar.is_empty() || planar.len() as u16 != spec.channel_count {
            bail!(
                "FLAC channel count mismatch: spec {} vs planar {}",
                spec.channel_count,
                planar.len()
            );
        }
        let frames = planar_frames(planar);
        if frames == 0 {
            bail!("FLAC cannot encode an empty buffer");
        }
        // flacenc/Symphonia need at least MIN_BLOCK_SIZE samples in a frame;
        // shorter buffers encode to bytes that common decoders reject.
        if frames < MIN_BLOCK_SIZE {
            bail!("FLAC cannot encode fewer than {MIN_BLOCK_SIZE} frames (got {frames})");
        }
        let bits = bits_for_integer(format).context("FLAC requires signed integer PCM")?;

        let mut encoder_cfg = config::Encoder::default();
        encoder_cfg.multithread = false;
        // Prefer one frame when the whole buffer fits — avoids short final
        // frames that Symphonia cannot read from flacenc output.
        if frames <= MAX_BLOCK_SIZE {
            encoder_cfg.block_size = frames;
        }
        let block_size = encoder_cfg.block_size;

        // flacenc's short final frame is playable in ffmpeg/reference flac but
        // Symphonia (our decoder) fails with "unexpected end of file". Pad to a
        // whole number of blocks, then declare the true length in STREAMINFO.
        let padded_frames = frames.div_ceil(block_size) * block_size;
        let padded_planar: Option<Vec<Vec<f32>>> = if padded_frames == frames {
            None
        } else {
            Some(
                planar
                    .iter()
                    .map(|ch| {
                        let mut padded = ch.clone();
                        padded.resize(padded_frames, 0.0);
                        padded
                    })
                    .collect(),
            )
        };
        let planar_for_encode: &[Vec<f32>] = padded_planar.as_deref().unwrap_or(planar);

        let samples = interleave_i32(planar_for_encode, bits);
        let source = MemSource::from_samples(
            &samples,
            planar.len(),
            bits as usize,
            spec.sample_rate as usize,
        );
        let config = encoder_cfg
            .into_verified()
            .map_err(|(_, err)| anyhow::anyhow!("invalid FLAC encoder config: {err}"))?;
        let mut stream = encode_with_fixed_block_size(&config, source, config.block_size)
            .map_err(|err| anyhow::anyhow!("FLAC encode failed: {err}"))?;
        if padded_frames != frames {
            stream.stream_info_mut().set_total_samples(frames);
            // Padded MD5 would not match the audible samples; clear it.
            stream.stream_info_mut().set_md5_digest(&[0u8; 16]);
            // All encoded frames are full-size after padding.
            stream
                .stream_info_mut()
                .set_block_sizes(block_size, block_size)
                .map_err(|err| anyhow::anyhow!("FLAC block size update failed: {err}"))?;
        }
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
