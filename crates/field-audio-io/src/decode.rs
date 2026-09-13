// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;
use std::{fs, fs::File, path::Path, time::SystemTime};

use anyhow::{bail, Context, Result};
use field_audio_model::{BlockSource, Buffer, BufferSource, PcmBuffer};
use field_audio_process::build_peaks;
use symphonia::core::{
    audio::{sample::SampleFormat, AudioSpec, GenericAudioBufferRef},
    codecs::audio::{AudioCodecParameters, AudioDecoderOptions},
    errors::Error as SymphoniaError,
    formats::{probe::Hint, FormatOptions, FormatReader, SeekMode, SeekTo, Track, TrackType},
    io::MediaSourceStream,
    meta::MetadataOptions,
    packet::Packet,
    units::{Time, Timestamp},
};

/// [`BlockSource`] backed by Symphonia decode.
#[derive(Debug, Default, Clone, Copy)]
pub struct SymphoniaBlockSource;

impl BlockSource for SymphoniaBlockSource {
    fn decode_range(&self, path: &Path, start: u64, count: u64) -> anyhow::Result<Vec<Vec<f32>>> {
        decode_range(path, start, count)
    }
}

/// Metadata for a media file without necessarily decoding every sample.
#[derive(Debug, Clone)]
pub struct ProbedFile {
    /// Filesystem path that was probed.
    pub path: PathBuf,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channel_count: usize,
    /// Total frames when known.
    pub frame_count: u64,
    /// Bits per sample when the container reports it.
    pub bits_per_sample: Option<u32>,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Last modified time.
    pub modified: SystemTime,
    /// Container / extension label.
    pub container_format: String,
    /// Codec label.
    pub codec: String,
    /// Fully decoded samples when a header-only probe was impossible.
    pub samples: Option<Arc<Vec<Vec<f32>>>>,
}

/// Historical name; prefer [`PcmBuffer`].
pub type DecodedAudio = PcmBuffer;

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

/// Header-only probe. Never decodes PCM; fails if the container does not
/// report a frame count.
pub fn probe_header(path: &Path) -> Result<ProbedFile> {
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    if let Some(probed) = probe_flac_header(path, metadata.len()) {
        return Ok(probed);
    }
    let format = open_format(path)?;
    let track = first_audio_track(format.as_ref())?;
    let params = audio_codec_params(&track)?;
    let sample_rate = params.sample_rate.unwrap_or(0);
    let channel_count = params.channels.as_ref().map(|ch| ch.count()).unwrap_or(0);
    let bits_per_sample = codec_bits_per_sample(params);
    let frame_count = track.num_frames.filter(|n| *n > 0);
    let container_format = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("unknown")
        .to_string();
    let codec = params.codec.to_string();
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

/// Probe a file for rate, channels, and length. Fully decodes only when the
/// container does not report a frame count.
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
    let mut format = open_format(path)?;
    let track = first_audio_track(format.as_ref())?;
    let params = audio_codec_params(&track)?;
    let sample_rate = params.sample_rate.unwrap_or(0);
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .context("unsupported audio codec")?;

    let mut decoded_start = 0u64;
    if sample_rate > 0 {
        let seek_to = if track.time_base.is_some() {
            SeekTo::Timestamp {
                ts: Timestamp::new(start as i64),
                track_id,
            }
        } else {
            let seconds = start as f64 / f64::from(sample_rate);
            SeekTo::Time {
                time: Time::try_from_secs_f64(seconds).unwrap_or_default(),
                track_id: Some(track_id),
            }
        };
        if let Ok(seeked) = format.seek(SeekMode::Accurate, seek_to) {
            decoder.reset();
            decoded_start = if let Some(tb) = track.time_base {
                let t = tb.calc_time_saturating(seeked.actual_ts);
                (t.as_secs_f64() * f64::from(sample_rate)).round() as u64
            } else {
                seeked.actual_ts.get().max(0) as u64
            };
        }
    }
    if decoded_start > start {
        decoded_start = 0;
    }

    let mut spec: Option<AudioSpec> = None;
    let mut channels: Vec<Vec<f32>> = Vec::new();
    let skip = start.saturating_sub(decoded_start);
    let needed = skip + count;

    loop {
        if !channels.is_empty() && channels[0].len() as u64 >= needed {
            break;
        }
        let packet = match next_audio_packet(format.as_mut())? {
            Some(packet) => packet,
            None => break,
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                append_decoded(decoded, &mut spec, &mut channels)?;
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
    let mut format = open_format(path)?;
    let container_format = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("unknown")
        .to_string();

    let track = first_audio_track(format.as_ref())?;
    let params = audio_codec_params(&track)?;
    let codec = params.codec.to_string();
    let mut bits_per_sample = codec_bits_per_sample(params);

    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .context("unsupported audio codec")?;

    let mut spec: Option<AudioSpec> = None;
    let mut channels: Vec<Vec<f32>> = Vec::new();

    loop {
        let packet = match next_audio_packet(format.as_mut())? {
            Some(packet) => packet,
            None => break,
        };

        if packet.track_id != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                append_decoded(decoded, &mut spec, &mut channels)?;
            }
            Err(SymphoniaError::DecodeError(_)) | Err(SymphoniaError::IoError(_)) => continue,
            Err(err) => bail!("unrecoverable decode error: {err}"),
        }
    }

    if channels.is_empty() || channels.iter().all(|c| c.is_empty()) {
        bail!("file contained no audio samples");
    }

    let sample_rate = spec.as_ref().map(|s| s.rate()).unwrap_or(0);
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

fn open_format(path: &Path) -> Result<Box<dyn FormatReader>> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .with_context(|| format!("unsupported or unreadable audio format: {}", path.display()))
}

fn first_audio_track(format: &dyn FormatReader) -> Result<Track> {
    format
        .default_track(TrackType::Audio)
        .cloned()
        .context("no supported audio track in file")
}

fn audio_codec_params(track: &Track) -> Result<&AudioCodecParameters> {
    track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .context("no audio codec parameters")
}

fn next_audio_packet(format: &mut dyn FormatReader) -> Result<Option<Packet>> {
    match format.next_packet() {
        Ok(packet) => Ok(packet),
        Err(SymphoniaError::ResetRequired) => {
            bail!("media reset required; this file is not supported");
        }
        Err(err) => bail!("error reading audio packet: {err}"),
    }
}

fn codec_bits_per_sample(params: &AudioCodecParameters) -> Option<u32> {
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

fn append_decoded(
    decoded: GenericAudioBufferRef<'_>,
    spec: &mut Option<AudioSpec>,
    channels: &mut Vec<Vec<f32>>,
) -> Result<()> {
    let decoded_spec = decoded.spec().clone();
    let channel_count = decoded_spec.channels().count();
    if channel_count == 0 {
        bail!("audio track has no channels");
    }

    if spec.is_none() {
        *spec = Some(decoded_spec);
        *channels = vec![Vec::new(); channel_count];
    }

    let mut packet: Vec<Vec<f32>> = Vec::new();
    decoded.copy_to_vecs_planar(&mut packet);
    if packet.len() != channels.len() {
        bail!("decoded plane count does not match channel count");
    }
    for (ch, plane) in channels.iter_mut().zip(packet) {
        ch.extend(plane);
    }

    Ok(())
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
