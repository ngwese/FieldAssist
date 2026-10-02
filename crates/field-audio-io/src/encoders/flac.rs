// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::io::{Cursor, Write};

use anyhow::{bail, Context, Result};
use flac_codec::encode::{FlacSampleWriter, Options};

use crate::metadata::TagMap;
use crate::pcm::{bits_for_integer, interleave_i32, planar_frames};
use crate::spec::{EncodeSpec, EncoderCaps, PcmFormat};
use crate::FormatEncoder;

/// FLAC encoder using `flac-codec`.
pub struct FlacEncoder;

const FORMATS: &[PcmFormat] = &[PcmFormat::S8, PcmFormat::S16, PcmFormat::S24];

fn flac_options() -> Options {
    // Default compression; no padding block (ingest/staging files are final).
    Options::default().no_padding()
}

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

    fn encode_with_tags(
        &self,
        spec: &EncodeSpec,
        planar: &[Vec<f32>],
        writer: &mut dyn Write,
        tags: &TagMap,
    ) -> Result<()> {
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
        let bits = bits_for_integer(format).context("FLAC requires signed integer PCM")?;
        let channels = planar.len() as u8;
        let samples = interleave_i32(planar, bits);

        let mut cursor = Cursor::new(Vec::new());
        let mut flac = FlacSampleWriter::new(
            &mut cursor,
            flac_options(),
            spec.sample_rate,
            bits,
            channels,
            Some(samples.len() as u64),
        )
        .map_err(|err| anyhow::anyhow!("FLAC create writer: {err}"))?;
        flac.write(&samples)
            .map_err(|err| anyhow::anyhow!("FLAC write: {err}"))?;
        flac.finalize()
            .map_err(|err| anyhow::anyhow!("FLAC finalize: {err}"))?;
        let bytes = cursor.into_inner();
        if bytes.is_empty() {
            bail!("FLAC encoder produced no bytes");
        }
        super::tag_write::encode_with_optional_tags(
            "flac",
            tags,
            |out| {
                out.extend_from_slice(&bytes);
                Ok(())
            },
            writer,
        )
    }
}
