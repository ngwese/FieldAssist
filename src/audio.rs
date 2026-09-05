// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;
use std::{fs, fs::File, path::Path, time::SystemTime};

use anyhow::{bail, Context, Result};
use symphonia::core::{
    audio::{AudioBufferRef, SampleBuffer, SignalSpec},
    codecs::{CodecParameters, DecoderOptions, CODEC_TYPE_NULL},
    errors::Error as SymphoniaError,
    formats::{FormatOptions, SeekMode, SeekTo},
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
    sample::SampleFormat,
    units::Time,
};

use crate::components::waveform::WaveformDataProvider;
use crate::model::{Buffer, BufferSource};

/// Number of samples folded into each overview peak bin.
pub const PEAK_BLOCK: usize = 256;

/// Metadata for a media file without necessarily decoding every sample.
#[derive(Debug, Clone)]
pub struct ProbedFile {
    pub path: PathBuf,
    pub sample_rate: u32,
    pub channel_count: usize,
    pub frame_count: u64,
    pub bits_per_sample: Option<u32>,
    pub size_bytes: u64,
    pub modified: SystemTime,
    pub container_format: String,
    pub codec: String,
    pub samples: Option<Arc<Vec<Vec<f32>>>>,
}

/// Decoded, planar audio ready for waveform display.
#[derive(Debug)]
pub struct DecodedAudio {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
    pub peaks: Vec<Vec<(f32, f32)>>,
}

impl DecodedAudio {
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    pub fn frames(&self) -> usize {
        self.channels.first().map(|c| c.len()).unwrap_or(0)
    }

    pub fn duration_secs(&self) -> f64 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.frames() as f64 / f64::from(self.sample_rate)
        }
    }
}

impl WaveformDataProvider for DecodedAudio {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channel_count(&self) -> usize {
        self.channels.len()
    }

    fn frames(&self) -> usize {
        self.channels.first().map(|c| c.len()).unwrap_or(0)
    }

    fn duration_secs(&self) -> f64 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.frames() as f64 / f64::from(self.sample_rate)
        }
    }

    fn channel_label(&self, channel: usize) -> String {
        match (self.channels.len(), channel) {
            (1, 0) => "Mono".into(),
            (2, 0) => "L".into(),
            (2, 1) => "R".into(),
            _ => format!("Ch {}", channel + 1),
        }
    }

    fn read_channel(&self, channel: usize, start: usize, dest: &mut [f32]) {
        dest.fill(0.0);
        let Some(samples) = self.channels.get(channel) else {
            return;
        };
        if start >= samples.len() {
            return;
        }
        let n = dest.len().min(samples.len() - start);
        dest[..n].copy_from_slice(&samples[start..start + n]);
    }

    fn min_max_in_range(&self, channel: usize, start: f64, end: f64) -> (f32, f32) {
        let samples = self
            .channels
            .get(channel)
            .map(|s| s.as_slice())
            .unwrap_or(&[]);
        let peaks = self.peaks.get(channel).map(|p| p.as_slice()).unwrap_or(&[]);
        min_max_in_range(samples, peaks, start, end)
    }
}

struct DecodeMeta {
    container_format: String,
    codec: String,
    bits_per_sample: Option<u32>,
}

/// Decode `path` into planar f32 samples, one vector per channel.
pub fn decode(path: &Path) -> Result<DecodedAudio> {
    decode_with_meta(path).map(|(audio, _)| audio)
}

/// Load a buffer with file metadata from `path`.
pub fn load_buffer(path: &Path) -> Result<Buffer> {
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    let (audio, meta) = decode_with_meta(path)?;
    Ok(Buffer {
        audio,
        source: Some(BufferSource {
            path: path.to_path_buf(),
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            size_bytes: metadata.len(),
            bits_per_sample: meta.bits_per_sample,
            container_format: meta.container_format,
            codec: meta.codec,
        }),
    })
}

/// Probe a file for rate, channels, and length. Fully decodes only when the
/// container does not report a frame count.
/// Header-only probe. Never decodes PCM; fails if the container does not
/// report a frame count.
pub fn probe_header(path: &Path) -> Result<ProbedFile> {
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    if let Some(probed) = probe_flac_header(path, metadata.len()) {
        return Ok(probed);
    }
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .with_context(|| format!("unsupported or unreadable audio format: {}", path.display()))?;
    let format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .cloned()
        .context("no supported audio track in file")?;
    let sample_rate = track.codec_params.sample_rate.unwrap_or(0);
    let channel_count = track
        .codec_params
        .channels
        .map(|ch| ch.count())
        .unwrap_or(0);
    let bits_per_sample = codec_bits_per_sample(&track.codec_params);
    let frame_count = track.codec_params.n_frames.filter(|n| *n > 0);
    let container_format = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("unknown")
        .to_string();
    let codec = track.codec_params.codec.to_string();
    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let Some(frame_count) = frame_count else {
        bail!(
            "audio header does not report frame count: {}",
            path.display()
        );
    };
    if sample_rate == 0 || channel_count == 0 {
        bail!(
            "audio header is missing sample rate or channels: {}",
            path.display()
        );
    }
    Ok(ProbedFile {
        path: path.to_path_buf(),
        sample_rate,
        channel_count,
        frame_count,
        bits_per_sample,
        size_bytes: metadata.len(),
        modified,
        container_format,
        codec,
        samples: None,
    })
}

pub fn probe_file(path: &Path) -> Result<ProbedFile> {
    match probe_header(path) {
        Ok(probed) => Ok(probed),
        Err(_) => {
            let metadata =
                fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
            let audio = decode(path)?;
            Ok(ProbedFile {
                path: path.to_path_buf(),
                sample_rate: audio.sample_rate,
                channel_count: audio.channel_count(),
                frame_count: audio.frames() as u64,
                bits_per_sample: None,
                size_bytes: metadata.len(),
                modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                container_format: path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("unknown")
                    .to_string(),
                codec: "pcm".into(),
                samples: Some(Arc::new(audio.channels)),
            })
        }
    }
}

/// Decode `count` frames starting at `start`, without loading the whole file.
pub fn decode_range(path: &Path, start: u64, count: u64) -> Result<Vec<Vec<f32>>> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .with_context(|| format!("unsupported or unreadable audio format: {}", path.display()))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .cloned()
        .context("no supported audio track in file")?;
    let sample_rate = track.codec_params.sample_rate.unwrap_or(0);
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("unsupported audio codec")?;

    let mut decoded_start = 0u64;
    if sample_rate > 0 {
        let seek_to = if track.codec_params.time_base.is_some() {
            SeekTo::TimeStamp {
                ts: start,
                track_id,
            }
        } else {
            let seconds = start as f64 / f64::from(sample_rate);
            SeekTo::Time {
                time: Time::new(seconds.trunc() as u64, seconds.fract()),
                track_id: Some(track_id),
            }
        };
        if let Ok(seeked) = format.seek(SeekMode::Accurate, seek_to) {
            decoder.reset();
            decoded_start = if let Some(tb) = track.codec_params.time_base {
                let t = tb.calc_time(seeked.actual_ts);
                ((t.seconds as f64 + t.frac) * f64::from(sample_rate)).round() as u64
            } else {
                seeked.actual_ts
            };
        }
    }
    if decoded_start > start {
        decoded_start = 0;
    }

    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut spec: Option<SignalSpec> = None;
    let mut channels: Vec<Vec<f32>> = Vec::new();
    let skip = start.saturating_sub(decoded_start);
    let needed = skip + count;

    loop {
        if !channels.is_empty() && channels[0].len() as u64 >= needed {
            break;
        }
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::ResetRequired) => {
                bail!("media reset required; this file is not supported");
            }
            Err(SymphoniaError::IoError(err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(err) => {
                bail!("error reading audio packet: {err}");
            }
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                append_decoded(decoded, &mut sample_buf, &mut spec, &mut channels)?;
            }
            Err(SymphoniaError::DecodeError(_)) | Err(SymphoniaError::IoError(_)) => continue,
            Err(err) => bail!("unrecoverable decode error: {err}"),
        }
    }

    if channels.is_empty() {
        bail!("file contained no audio samples");
    }
    let start_i = skip as usize;
    let end_i = start_i + count as usize;
    let mut out = Vec::with_capacity(channels.len());
    for ch in channels {
        if start_i >= ch.len() {
            out.push(vec![0.0; count as usize]);
        } else {
            let e = end_i.min(ch.len());
            let mut slice = ch[start_i..e].to_vec();
            slice.resize(count as usize, 0.0);
            out.push(slice);
        }
    }
    Ok(out)
}

fn decode_with_meta(path: &Path) -> Result<(DecodedAudio, DecodeMeta)> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .with_context(|| format!("unsupported or unreadable audio format: {}", path.display()))?;

    let mut format = probed.format;
    let container_format = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("unknown")
        .to_string();

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .cloned()
        .context("no supported audio track in file")?;

    let codec = track.codec_params.codec.to_string();
    let mut bits_per_sample = codec_bits_per_sample(&track.codec_params);

    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("unsupported audio codec")?;

    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut spec: Option<SignalSpec> = None;
    let mut channels: Vec<Vec<f32>> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::ResetRequired) => {
                bail!("media reset required; this file is not supported");
            }
            Err(SymphoniaError::IoError(err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(err) => {
                bail!("error reading audio packet: {err}");
            }
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                append_decoded(decoded, &mut sample_buf, &mut spec, &mut channels)?;
            }
            Err(SymphoniaError::DecodeError(_)) | Err(SymphoniaError::IoError(_)) => continue,
            Err(err) => bail!("unrecoverable decode error: {err}"),
        }
    }

    if channels.is_empty() || channels.iter().all(|c| c.is_empty()) {
        bail!("file contained no audio samples");
    }

    let sample_rate = spec.map(|s| s.rate).unwrap_or(0);
    if sample_rate == 0 {
        bail!("audio track has no sample rate");
    }

    let peaks = channels.iter().map(|ch| build_peaks(ch)).collect();
    if bits_per_sample.is_none() {
        bits_per_sample = codec_bits_per_sample(decoder.codec_params());
    }

    Ok((
        DecodedAudio {
            sample_rate,
            channels,
            peaks,
        },
        DecodeMeta {
            container_format,
            codec,
            bits_per_sample,
        },
    ))
}

/// Native FLAC stores rate, channels, and length in STREAMINFO. Reading that
/// header avoids a container probe that can hitch on large files.
fn probe_flac_header(path: &Path, size_bytes: u64) -> Option<ProbedFile> {
    let ext = path.extension().and_then(|e| e.to_str())?;
    if !ext.eq_ignore_ascii_case("flac") {
        return None;
    }
    let info = parse_flac_streaminfo(path)?;
    Some(ProbedFile {
        path: path.to_path_buf(),
        sample_rate: info.sample_rate,
        channel_count: info.channel_count,
        frame_count: info.frame_count,
        bits_per_sample: info.bits_per_sample,
        size_bytes,
        modified: fs::metadata(path)
            .ok()
            .and_then(|meta| meta.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH),
        container_format: "flac".into(),
        codec: "flac".into(),
        samples: None,
    })
}

struct FlacStreaminfo {
    sample_rate: u32,
    channel_count: usize,
    bits_per_sample: Option<u32>,
    frame_count: u64,
}

fn skip_id3v2(file: &mut File) -> Option<()> {
    let mut hdr = [0u8; 10];
    file.read_exact(&mut hdr).ok()?;
    if &hdr[0..3] != b"ID3" {
        file.seek(SeekFrom::Start(0)).ok()?;
        return Some(());
    }
    let size = ((u64::from(hdr[6]) & 0x7f) << 21)
        | ((u64::from(hdr[7]) & 0x7f) << 14)
        | ((u64::from(hdr[8]) & 0x7f) << 7)
        | (u64::from(hdr[9]) & 0x7f);
    file.seek(SeekFrom::Start(10 + size)).ok()?;
    Some(())
}

fn parse_flac_streaminfo(path: &Path) -> Option<FlacStreaminfo> {
    let mut file = File::open(path).ok()?;
    skip_id3v2(&mut file)?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).ok()?;
    if &magic != b"fLaC" {
        return None;
    }
    loop {
        let mut block_hdr = [0u8; 4];
        file.read_exact(&mut block_hdr).ok()?;
        let last = block_hdr[0] & 0x80 != 0;
        let block_type = block_hdr[0] & 0x7f;
        let len = u32::from_be_bytes([0, block_hdr[1], block_hdr[2], block_hdr[3]]) as usize;
        if block_type == 0 {
            if len < 18 {
                return None;
            }
            let mut payload = vec![0u8; len];
            file.read_exact(&mut payload).ok()?;
            let packed = u64::from_be_bytes(payload[10..18].try_into().ok()?);
            let sample_rate = (packed >> 44) as u32;
            let channel_count = ((packed >> 41) & 7) as usize + 1;
            let bits = ((packed >> 36) & 0x1f) as u32 + 1;
            let frame_count = packed & ((1u64 << 36) - 1);
            if sample_rate == 0 || channel_count == 0 || frame_count == 0 {
                return None;
            }
            return Some(FlacStreaminfo {
                sample_rate,
                channel_count,
                bits_per_sample: Some(bits),
                frame_count,
            });
        }
        file.seek(SeekFrom::Current(len as i64)).ok()?;
        if last {
            break;
        }
    }
    None
}

fn codec_bits_per_sample(params: &CodecParameters) -> Option<u32> {
    params
        .bits_per_sample
        .or(params.bits_per_coded_sample)
        .or_else(|| {
            params.sample_format.map(|format| match format {
                SampleFormat::U8 | SampleFormat::S8 => 8,
                SampleFormat::U16 | SampleFormat::S16 => 16,
                SampleFormat::U24 | SampleFormat::S24 => 24,
                SampleFormat::U32 | SampleFormat::S32 | SampleFormat::F32 => 32,
                SampleFormat::F64 => 64,
            })
        })
}

impl WaveformDataProvider for Buffer {
    fn sample_rate(&self) -> u32 {
        self.audio.sample_rate
    }

    fn channel_count(&self) -> usize {
        self.audio.channel_count()
    }

    fn frames(&self) -> usize {
        self.audio.frames()
    }

    fn duration_secs(&self) -> f64 {
        self.audio.duration_secs()
    }

    fn channel_label(&self, channel: usize) -> String {
        WaveformDataProvider::channel_label(&self.audio, channel)
    }

    fn read_channel(&self, channel: usize, start: usize, dest: &mut [f32]) {
        WaveformDataProvider::read_channel(&self.audio, channel, start, dest)
    }

    fn min_max_in_range(&self, channel: usize, start: f64, end: f64) -> (f32, f32) {
        WaveformDataProvider::min_max_in_range(&self.audio, channel, start, end)
    }
}

fn append_decoded(
    decoded: AudioBufferRef<'_>,
    sample_buf: &mut Option<SampleBuffer<f32>>,
    spec: &mut Option<SignalSpec>,
    channels: &mut Vec<Vec<f32>>,
) -> Result<()> {
    let decoded_spec = *decoded.spec();
    let channel_count = decoded_spec.channels.count();
    if channel_count == 0 {
        bail!("audio track has no channels");
    }

    if spec.is_none() {
        *spec = Some(decoded_spec);
        *channels = vec![Vec::new(); channel_count];
        *sample_buf = Some(SampleBuffer::<f32>::new(
            decoded.capacity() as u64,
            decoded_spec,
        ));
    }

    let buf = sample_buf
        .as_mut()
        .expect("sample buffer is created with the first packet");
    buf.copy_interleaved_ref(decoded);

    let samples = buf.samples();
    if samples.len() % channel_count != 0 {
        bail!("decoded sample count is not a multiple of the channel count");
    }

    for frame in samples.chunks_exact(channel_count) {
        for (ch, sample) in frame.iter().enumerate() {
            channels[ch].push(*sample);
        }
    }

    Ok(())
}

pub fn build_peaks(samples: &[f32]) -> Vec<(f32, f32)> {
    samples
        .chunks(PEAK_BLOCK)
        .map(|chunk| {
            let mut min = f32::MAX;
            let mut max = f32::MIN;
            for &s in chunk {
                min = min.min(s);
                max = max.max(s);
            }
            if min > max {
                (0.0, 0.0)
            } else {
                (min, max)
            }
        })
        .collect()
}

/// Min/max of `samples` in `[start, end)`, using peak bins when the range is large.
pub fn min_max_in_range(samples: &[f32], peaks: &[(f32, f32)], start: f64, end: f64) -> (f32, f32) {
    if samples.is_empty() {
        return (0.0, 0.0);
    }

    let start_i = start.max(0.0).floor() as usize;
    let end_i = (end.ceil() as usize).clamp(start_i, samples.len());
    if start_i >= end_i {
        return (0.0, 0.0);
    }

    let mut min = f32::MAX;
    let mut max = f32::MIN;

    if end_i - start_i >= PEAK_BLOCK * 2 && !peaks.is_empty() {
        let peak_start = start_i / PEAK_BLOCK;
        let peak_end = ((end_i + PEAK_BLOCK - 1) / PEAK_BLOCK).min(peaks.len());
        for &(pmin, pmax) in &peaks[peak_start..peak_end] {
            min = min.min(pmin);
            max = max.max(pmax);
        }
    } else {
        for &s in &samples[start_i..end_i] {
            min = min.min(s);
            max = max.max(s);
        }
    }

    if min > max {
        (0.0, 0.0)
    } else {
        (min, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::File, io::Write, path::Path};

    fn write_sine_wav(path: &Path, channels: u16, frames: u32, sample_rate: u32) {
        let bits_per_sample: u16 = 16;
        let block_align = channels * bits_per_sample / 8;
        let byte_rate = sample_rate * u32::from(block_align);
        let data_len = frames * u32::from(block_align);
        let mut out = File::create(path).unwrap();
        out.write_all(b"RIFF").unwrap();
        out.write_all(&(36 + data_len).to_le_bytes()).unwrap();
        out.write_all(b"WAVE").unwrap();
        out.write_all(b"fmt ").unwrap();
        out.write_all(&16u32.to_le_bytes()).unwrap();
        out.write_all(&1u16.to_le_bytes()).unwrap();
        out.write_all(&channels.to_le_bytes()).unwrap();
        out.write_all(&sample_rate.to_le_bytes()).unwrap();
        out.write_all(&byte_rate.to_le_bytes()).unwrap();
        out.write_all(&block_align.to_le_bytes()).unwrap();
        out.write_all(&bits_per_sample.to_le_bytes()).unwrap();
        out.write_all(b"data").unwrap();
        out.write_all(&data_len.to_le_bytes()).unwrap();
        for i in 0..frames {
            let t = i as f32 / sample_rate as f32;
            for ch in 0..channels {
                let freq = 440.0 * (ch as f32 + 1.0);
                let sample = (t * freq * std::f32::consts::TAU).sin();
                let pcm = (sample * 0.6 * i16::MAX as f32) as i16;
                out.write_all(&pcm.to_le_bytes()).unwrap();
            }
        }
    }

    #[test]
    fn peaks_cover_all_samples() {
        let samples: Vec<f32> = (0..1000).map(|i| (i as f32) / 1000.0 - 0.5).collect();
        let peaks = build_peaks(&samples);
        assert_eq!(peaks.len(), (1000 + PEAK_BLOCK - 1) / PEAK_BLOCK);
        let (min, max) = min_max_in_range(&samples, &peaks, 0.0, 1000.0);
        assert!(min < 0.0);
        assert!(max > 0.0);
    }

    #[test]
    fn decodes_stereo_wav() {
        let dir = std::env::temp_dir();
        let path = dir.join("snd-display-stereo-test.wav");
        write_sine_wav(&path, 2, 4410, 44100);
        let audio = decode(&path).expect("decode stereo wav");
        assert_eq!(audio.channel_count(), 2);
        assert_eq!(audio.sample_rate, 44100);
        assert_eq!(audio.frames(), 4410);
        assert_eq!(audio.channels[0].len(), audio.channels[1].len());
        assert!(audio.channels[0].iter().any(|s| s.abs() > 0.1));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn load_buffer_captures_wav_source_meta() {
        let dir = std::env::temp_dir();
        let path = dir.join("snd-display-source-meta-test.wav");
        write_sine_wav(&path, 2, 4410, 44100);
        let buffer = load_buffer(&path).expect("load wav");
        let source = buffer.source.expect("source metadata");
        assert_eq!(source.bits_per_sample, Some(16));
        assert_eq!(source.size_bytes, std::fs::metadata(&path).unwrap().len());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn decode_range_reads_a_slice() {
        let dir = std::env::temp_dir();
        let path = dir.join("snd-display-range-test.wav");
        write_sine_wav(&path, 1, 2000, 44100);
        let full = decode(&path).unwrap();
        let slice = decode_range(&path, 100, 50).unwrap();
        assert_eq!(slice.len(), 1);
        assert_eq!(slice[0].len(), 50);
        for (a, b) in slice[0].iter().zip(full.channels[0][100..150].iter()) {
            assert!((a - b).abs() < 1e-4);
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn flac_header_probe_skips_decode() {
        let sample_rate: u64 = 44100;
        let channels_m1: u64 = 1;
        let bps_m1: u64 = 15;
        let total: u64 = 12_345;
        let packed = (sample_rate << 44) | (channels_m1 << 41) | (bps_m1 << 36) | total;
        let mut payload = [0u8; 34];
        payload[10..18].copy_from_slice(&packed.to_be_bytes());
        let mut bytes = Vec::from(b"fLaC".as_slice());
        bytes.push(0x80);
        bytes.extend_from_slice(&34u32.to_be_bytes()[1..]);
        bytes.extend_from_slice(&payload);
        let path = std::env::temp_dir().join("snd-flac-streaminfo-header.flac");
        std::fs::write(&path, &bytes).unwrap();
        let probed = probe_file(&path).unwrap();
        assert_eq!(probed.sample_rate, 44100);
        assert_eq!(probed.channel_count, 2);
        assert_eq!(probed.bits_per_sample, Some(16));
        assert_eq!(probed.frame_count, total);
        assert!(probed.samples.is_none());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn probe_file_reports_wav_frames() {
        let dir = std::env::temp_dir();
        let path = dir.join("snd-display-probe-test.wav");
        write_sine_wav(&path, 2, 4410, 44100);
        let probed = probe_file(&path).unwrap();
        assert_eq!(probed.sample_rate, 44100);
        assert_eq!(probed.channel_count, 2);
        assert_eq!(probed.frame_count, 4410);
        assert_eq!(probed.bits_per_sample, Some(16));
        let _ = std::fs::remove_file(path);
    }
}
