// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Streaming encode sessions for bounded-memory transcode.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::thread::{self, JoinHandle};

use anyhow::{bail, Context, Result};
use flacenc::bitsink::ByteSink;
use flacenc::component::BitRepr;
use flacenc::config;
use flacenc::constant::{MAX_BLOCK_SIZE, MIN_BLOCK_SIZE};
use flacenc::encode_with_fixed_block_size;
use flacenc::error::Verify;
use flacenc::source::Fill;
use flacenc::source::Source;
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

// ── FLAC (pull Source + bounded block queue) ─────────────────────────────────

struct FlacEncodeStream {
    tx: Option<SyncSender<Option<Vec<i32>>>>,
    join: Option<JoinHandle<Result<()>>>,
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
        let sample_rate = spec.sample_rate as usize;
        let total_frames = total_output_frames as usize;
        if total_frames > 0 && total_frames < MIN_BLOCK_SIZE {
            bail!("FLAC cannot encode fewer than {MIN_BLOCK_SIZE} frames (got {total_frames})");
        }
        let dest_buf = dest.to_path_buf();
        let (tx, rx) = sync_channel::<Option<Vec<i32>>>(2);
        let join = thread::Builder::new()
            .name("fa-flac-encode".into())
            .spawn(move || {
                encode_flac_from_channel(
                    rx,
                    channels,
                    bits as usize,
                    sample_rate,
                    total_frames,
                    &dest_buf,
                )
            })
            .context("spawn FLAC encode thread")?;
        Ok(Self {
            tx: Some(tx),
            join: Some(join),
            bits,
            channels,
            frames_written: 0,
            total_output_frames: total_output_frames,
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
        let tx = self.tx.as_ref().context("FLAC stream already finished")?;
        tx.send(Some(samples))
            .map_err(|_| anyhow::anyhow!("FLAC encode thread closed"))?;
        self.frames_written += frames as u64;
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        // Pad to whole flacenc blocks with silence so Symphonia can decode.
        if let Some(tx) = self.tx.as_ref() {
            let block = MAX_BLOCK_SIZE.min(4096).max(MIN_BLOCK_SIZE);
            let target = if self.total_output_frames > 0 {
                self.total_output_frames
                    .div_ceil(block as u64)
                    .saturating_mul(block as u64)
            } else {
                self.frames_written
                    .div_ceil(block as u64)
                    .saturating_mul(block as u64)
            };
            while self.frames_written < target {
                let n = ((target - self.frames_written) as usize).min(block);
                let silence = vec![0i32; n * self.channels];
                tx.send(Some(silence))
                    .map_err(|_| anyhow::anyhow!("FLAC encode thread closed"))?;
                self.frames_written += n as u64;
            }
        }
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(None);
            drop(tx);
        }
        if let Some(join) = self.join.take() {
            join.join()
                .map_err(|_| anyhow::anyhow!("FLAC encode thread panicked"))??;
        }
        Ok(())
    }
}

struct ChannelFlacSource {
    rx: Receiver<Option<Vec<i32>>>,
    leftover: Vec<i32>,
    channels: usize,
    bits: usize,
    sample_rate: usize,
    total_frames: usize,
    frames_delivered: usize,
    eof: bool,
}

impl Source for ChannelFlacSource {
    fn channels(&self) -> usize {
        self.channels
    }
    fn bits_per_sample(&self) -> usize {
        self.bits
    }
    fn sample_rate(&self) -> usize {
        self.sample_rate
    }
    fn len_hint(&self) -> Option<usize> {
        if self.total_frames > 0 {
            Some(self.total_frames)
        } else {
            None
        }
    }
    fn read_samples<F: Fill>(
        &mut self,
        block_size: usize,
        dest: &mut F,
    ) -> Result<usize, flacenc::error::SourceError> {
        let want = block_size.saturating_mul(self.channels);
        while self.leftover.len() < want && !self.eof {
            match self.rx.recv() {
                Ok(Some(chunk)) => self.leftover.extend_from_slice(&chunk),
                Ok(None) => self.eof = true,
                Err(_) => self.eof = true,
            }
        }
        if self.leftover.is_empty() {
            return Ok(0);
        }
        let take_samples = want.min(self.leftover.len());
        // Align to whole frames.
        let take_samples = take_samples - (take_samples % self.channels.max(1));
        if take_samples == 0 {
            return Ok(0);
        }
        let chunk: Vec<i32> = self.leftover.drain(..take_samples).collect();
        let frames = take_samples / self.channels;
        dest.fill_interleaved(&chunk)?;
        self.frames_delivered += frames;
        Ok(frames)
    }
}

fn encode_flac_from_channel(
    rx: Receiver<Option<Vec<i32>>>,
    channels: usize,
    bits: usize,
    sample_rate: usize,
    total_frames: usize,
    dest: &Path,
) -> Result<()> {
    let mut source = ChannelFlacSource {
        rx,
        leftover: Vec::new(),
        channels,
        bits,
        sample_rate,
        total_frames,
        frames_delivered: 0,
        eof: false,
    };
    let mut encoder_cfg = config::Encoder::default();
    encoder_cfg.multithread = false;
    if total_frames > 0 && total_frames <= MAX_BLOCK_SIZE {
        encoder_cfg.block_size = total_frames.max(MIN_BLOCK_SIZE);
    }
    let block_size = encoder_cfg.block_size;
    let config = encoder_cfg
        .into_verified()
        .map_err(|(_, err)| anyhow::anyhow!("invalid FLAC encoder config: {err}"))?;

    // Pad declared length to whole blocks so Symphonia can read the stream;
    // adjust STREAMINFO after encode (same approach as full-buffer path).
    let padded_hint = if total_frames > 0 {
        total_frames.div_ceil(block_size) * block_size
    } else {
        0
    };
    if padded_hint > total_frames && total_frames > 0 {
        source.total_frames = padded_hint;
    }

    let mut stream = encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|err| anyhow::anyhow!("FLAC encode failed: {err}"))?;
    if padded_hint != total_frames && total_frames > 0 {
        stream.stream_info_mut().set_total_samples(total_frames);
        stream.stream_info_mut().set_md5_digest(&[0u8; 16]);
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
    std::fs::write(dest, sink.as_slice()).with_context(|| format!("write {}", dest.display()))?;
    Ok(())
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
