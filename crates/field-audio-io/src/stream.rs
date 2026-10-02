// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Streaming encode sessions for bounded-memory transcode.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{bail, Context, Result};
use flac_codec::encode::{FlacSampleWriter, Options};
use hound::{SampleFormat, WavSpec, WavWriter};
use ogg::writing::{PacketWriteEndInfo, PacketWriter};
use rusty_vorbis::VorbisEncoder;

use crate::encoders::tag_write::apply_tags_to_path;
use crate::pcm::{
    bits_for_integer, clamp_unit, interleave_f32, interleave_i32, planar_frames, to_signed, to_u8,
};
use crate::spec::{EncodeSpec, PcmFormat};
use crate::{encoder, TagMap};

/// Streaming encoder session: write planar blocks then finalize.
pub trait EncodeStream: Send {
    /// Append a planar f32 block (already at the output sample rate).
    fn write_planar(&mut self, planar: &[Vec<f32>]) -> Result<()>;
    /// Finalize the container (and any pending encoder state).
    fn finish(&mut self) -> Result<()>;
}

/// Open a streaming encode session writing to `dest`.
///
/// `total_output_frames` is required for FLAC STREAMINFO; other encoders
/// may ignore it. Parent of `dest` must already exist.
pub fn begin_stream(
    encoder_id: &str,
    spec: &EncodeSpec,
    dest: &Path,
    total_output_frames: u64,
) -> Result<Box<dyn EncodeStream>> {
    let enc = encoder(encoder_id).with_context(|| format!("unknown encoder `{encoder_id}`"))?;
    if !enc.supports(spec) {
        bail!(
            "{} cannot encode the selected format, rate, or channel count",
            enc.label()
        );
    }
    match encoder_id {
        "wav" => Ok(Box::new(WavEncodeStream::open(spec, dest)?)),
        "flac" => Ok(Box::new(FlacEncodeStream::open(
            spec,
            dest,
            total_output_frames,
        )?)),
        "ogg" => Ok(Box::new(OggEncodeStream::open(spec, dest)?)),
        other => bail!("streaming encode is not implemented for `{other}`"),
    }
}

/// Apply metadata tags to a finished file on disk (no-op when `tags` empty).
pub fn finish_with_tags(dest: &Path, encoder_id: &str, tags: &TagMap) -> Result<()> {
    if tags.is_empty() {
        return Ok(());
    }
    let extension = encoder(encoder_id).map(|e| e.extension()).unwrap_or("bin");
    apply_tags_to_path(dest, extension, tags)
}

// ── WAV ──────────────────────────────────────────────────────────────────────

struct WavEncodeStream {
    writer: Option<WavWriter<BufWriter<File>>>,
    format: PcmFormat,
    channels: usize,
}

impl WavEncodeStream {
    fn open(spec: &EncodeSpec, dest: &Path) -> Result<Self> {
        let format = spec.sample_format.context("WAV requires a sample format")?;
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
        let file = File::create(dest).with_context(|| format!("create {}", dest.display()))?;
        let writer = WavWriter::new(BufWriter::new(file), wav_spec).context("create WAV writer")?;
        Ok(Self {
            writer: Some(writer),
            format,
            channels: usize::from(spec.channel_count),
        })
    }
}

impl EncodeStream for WavEncodeStream {
    fn write_planar(&mut self, planar: &[Vec<f32>]) -> Result<()> {
        let writer = self
            .writer
            .as_mut()
            .context("WAV stream already finished")?;
        let frames = planar_frames(planar);
        if frames == 0 {
            return Ok(());
        }
        if planar.len() != self.channels {
            bail!(
                "WAV channel count mismatch: expected {} got {}",
                self.channels,
                planar.len()
            );
        }
        for frame in 0..frames {
            for ch in 0..self.channels {
                let sample = planar[ch][frame];
                match self.format {
                    PcmFormat::U8 => {
                        let signed = (i16::from(to_u8(sample)) - 128) as i8;
                        writer.write_sample(signed).context("write WAV sample")?;
                    }
                    PcmFormat::S16 => {
                        writer
                            .write_sample(to_signed(sample, 16) as i16)
                            .context("write WAV sample")?;
                    }
                    PcmFormat::S24 | PcmFormat::S32 => {
                        writer
                            .write_sample(to_signed(sample, u32::from(self.format.bits())))
                            .context("write WAV sample")?;
                    }
                    PcmFormat::F32 => {
                        writer
                            .write_sample(clamp_unit(sample))
                            .context("write WAV sample")?;
                    }
                    _ => unreachable!(),
                }
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        if let Some(writer) = self.writer.take() {
            writer.finalize().context("finalize WAV")?;
        }
        Ok(())
    }
}

// ── FLAC (`flac-codec` sample writer) ────────────────────────────────────────

fn flac_options() -> Options {
    Options::default().no_padding().overwrite()
}

struct FlacEncodeStream {
    writer: Option<FlacSampleWriter<BufWriter<File>>>,
    bits: u32,
    channels: usize,
    frames_written: u64,
    total_output_frames: u64,
}

impl FlacEncodeStream {
    fn open(spec: &EncodeSpec, dest: &Path, total_output_frames: u64) -> Result<Self> {
        let format = spec
            .sample_format
            .context("FLAC requires a sample format")?;
        let bits = bits_for_integer(format).context("FLAC requires signed integer PCM")?;
        let channels = usize::from(spec.channel_count);
        if channels == 0 || channels > 8 {
            bail!("FLAC channel count must be 1..=8 (got {channels})");
        }
        let total_interleaved = if total_output_frames > 0 {
            Some(
                total_output_frames
                    .checked_mul(channels as u64)
                    .context("FLAC total sample count overflow")?,
            )
        } else {
            None
        };
        let writer = FlacSampleWriter::create(
            dest,
            flac_options(),
            spec.sample_rate,
            bits,
            channels as u8,
            total_interleaved,
        )
        .map_err(|err| anyhow::anyhow!("FLAC create {}: {err}", dest.display()))?;
        Ok(Self {
            writer: Some(writer),
            bits,
            channels,
            frames_written: 0,
            total_output_frames,
        })
    }
}

impl EncodeStream for FlacEncodeStream {
    fn write_planar(&mut self, planar: &[Vec<f32>]) -> Result<()> {
        let frames = planar_frames(planar);
        if frames == 0 {
            return Ok(());
        }
        if planar.len() != self.channels {
            bail!(
                "FLAC channel count mismatch: expected {} got {}",
                self.channels,
                planar.len()
            );
        }
        let samples = interleave_i32(planar, self.bits);
        let writer = self
            .writer
            .as_mut()
            .context("FLAC stream already finished")?;
        writer
            .write(&samples)
            .map_err(|err| anyhow::anyhow!("FLAC write: {err}"))?;
        self.frames_written += frames as u64;
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        let Some(mut writer) = self.writer.take() else {
            return Ok(());
        };
        // Match declared STREAMINFO length when the caller committed to a total.
        if self.total_output_frames > 0 && self.frames_written < self.total_output_frames {
            let missing = (self.total_output_frames - self.frames_written) as usize;
            let silence = vec![0i32; missing * self.channels];
            writer
                .write(&silence)
                .map_err(|err| anyhow::anyhow!("FLAC pad silence: {err}"))?;
            self.frames_written += missing as u64;
        }
        writer
            .finalize()
            .map_err(|err| anyhow::anyhow!("FLAC finalize: {err}"))?;
        Ok(())
    }
}

// ── Ogg Vorbis ───────────────────────────────────────────────────────────────

struct OggEncodeStream {
    encoder: VorbisEncoder,
    channels: u16,
    sample_rate: u32,
    file: BufWriter<File>,
    headers_written: bool,
    packet_index: usize,
    serial: u32,
    finished: bool,
}

impl OggEncodeStream {
    fn open(spec: &EncodeSpec, dest: &Path) -> Result<Self> {
        let channels = if spec.channel_count == 1 {
            2 // dual-mono (rusty_vorbis stereo setup header)
        } else {
            spec.channel_count
        };
        let file = File::create(dest).with_context(|| format!("create {}", dest.display()))?;
        Ok(Self {
            encoder: VorbisEncoder::default(),
            channels,
            sample_rate: spec.sample_rate,
            file: BufWriter::new(file),
            headers_written: false,
            packet_index: 0,
            serial: 1,
            finished: false,
        })
    }

    fn drain_packets(&mut self, end_stream: bool) -> Result<()> {
        let mut packets = Vec::new();
        loop {
            match self.encoder.next_packet() {
                Ok(packet) => packets.push(packet),
                Err(rusty_vorbis::Error::Eof) => break,
                Err(rusty_vorbis::Error::Again) => {
                    if packets.is_empty() && !end_stream {
                        break;
                    }
                    continue;
                }
                Err(err) => bail!("Vorbis packet: {err}"),
            }
        }
        if packets.is_empty() {
            return Ok(());
        }
        let mut ogg = PacketWriter::new(&mut self.file);
        let last = packets.len() - 1;
        for (i, packet) in packets.into_iter().enumerate() {
            let is_last = end_stream && i == last;
            let end = if !self.headers_written && self.packet_index < 3 && !is_last {
                PacketWriteEndInfo::EndPage
            } else if is_last {
                PacketWriteEndInfo::EndStream
            } else {
                PacketWriteEndInfo::NormalPacket
            };
            ogg.write_packet(packet.data, self.serial, end, packet.pts.max(0) as u64)
                .context("write Ogg packet")?;
            self.packet_index += 1;
        }
        if self.packet_index >= 3 {
            self.headers_written = true;
        }
        Ok(())
    }
}

impl EncodeStream for OggEncodeStream {
    fn write_planar(&mut self, planar: &[Vec<f32>]) -> Result<()> {
        if self.finished {
            bail!("Ogg stream already finished");
        }
        let frames = planar_frames(planar);
        if frames == 0 {
            return Ok(());
        }
        let mut planes = planar.to_vec();
        if planes.len() == 1 {
            planes.push(planes[0].clone());
        }
        if planes.len() as u16 != self.channels {
            bail!(
                "Ogg channel count mismatch: expected {} got {}",
                self.channels,
                planes.len()
            );
        }
        let pcm = interleave_f32(&planes);
        self.encoder
            .push_pcm_f32(&pcm, self.channels, self.sample_rate)
            .map_err(|err| anyhow::anyhow!("Vorbis encode: {err}"))?;
        self.drain_packets(false)
    }

    fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.encoder.finish();
        self.drain_packets(true)?;
        self.file.flush().context("flush Ogg output")?;
        self.finished = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe_file;

    #[test]
    fn wav_stream_roundtrip_probe() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.wav");
        let spec = EncodeSpec {
            sample_rate: 48_000,
            sample_format: Some(PcmFormat::S16),
            channel_count: 1,
        };
        let mut stream = begin_stream("wav", &spec, &dest, 256).unwrap();
        let block = vec![vec![0.25f32; 128]];
        stream.write_planar(&block).unwrap();
        stream.write_planar(&block).unwrap();
        stream.finish().unwrap();
        let probed = probe_file(&dest).unwrap();
        assert_eq!(probed.sample_rate, 48_000);
        assert_eq!(probed.channel_count, 1);
        assert_eq!(probed.frame_count, 256);
    }
}
