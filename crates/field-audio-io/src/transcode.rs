// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Streaming file-to-file transcode with optional sample-rate conversion.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use anyhow::{bail, Context, Result};
use field_audio_model::BLOCK_FRAMES;
use field_audio_process::StreamingResampler;

use crate::decode::{decode_range, probe_header};
use crate::pcm::select_channels;
use crate::spec::EncodeSpec;
use crate::stream::{begin_stream, finish_with_tags};
use crate::{encoder, TagMap};

/// Default decode/encode block size (matches pager [`BLOCK_FRAMES`]).
pub const DEFAULT_BLOCK_FRAMES: u64 = BLOCK_FRAMES;

/// Parameters for [`transcode`].
pub struct TranscodeRequest<'a> {
    /// Source media path (must be readable).
    pub source: &'a Path,
    /// Destination path (parent must exist and be writable).
    pub dest: &'a Path,
    /// Encoder id (`wav`, `flac`, `ogg`).
    pub encoder_id: &'a str,
    /// Output rate / PCM format / channel count.
    pub spec: EncodeSpec,
    /// Source channel indices to include (0-based).
    pub channel_indices: &'a [usize],
    /// Optional metadata tags applied after encode.
    pub tags: &'a TagMap,
    /// Decode block size in source frames (default [`DEFAULT_BLOCK_FRAMES`]).
    pub block_frames: u64,
    /// Optional progress callback: `(done_source_frames, total_source_frames)`.
    pub on_progress: Option<&'a mut dyn FnMut(u64, u64)>,
}

/// Stream `source` through optional SRC into `dest` using `encoder_id` / `spec`.
///
/// Peak memory stays O(block × channels + resampler delay), independent of
/// media length. Progress reports **source** frames.
pub fn transcode(mut req: TranscodeRequest<'_>) -> Result<()> {
    validate_paths(req.source, req.dest)?;

    let enc =
        encoder(req.encoder_id).with_context(|| format!("unknown encoder `{}`", req.encoder_id))?;
    if !enc.supports(&req.spec) {
        bail!(
            "{} cannot encode the selected format, rate, or channel count",
            enc.label()
        );
    }
    if req.channel_indices.is_empty() {
        bail!("select at least one channel");
    }

    let header =
        probe_header(req.source).with_context(|| format!("probe {}", req.source.display()))?;
    let source_rate = header.sample_rate;
    let source_channels = header.channel_count;
    let source_total = header.frame_count;
    if source_rate == 0 {
        bail!("source has no sample rate: {}", req.source.display());
    }
    for &index in req.channel_indices {
        if index >= source_channels {
            bail!("channel index {index} is out of range for {source_channels} channels");
        }
    }

    let out_channels = req.channel_indices.len().max(1);
    if req.spec.channel_count as usize != out_channels {
        bail!(
            "EncodeSpec channel_count {} does not match selected channels {}",
            req.spec.channel_count,
            out_channels
        );
    }

    let block_frames = if req.block_frames == 0 {
        DEFAULT_BLOCK_FRAMES
    } else {
        req.block_frames
    };

    let mut resampler =
        StreamingResampler::new(source_rate, req.spec.sample_rate, out_channels, 1024)?;
    let expected_out = if resampler.is_passthrough() {
        source_total
    } else {
        resampler.expected_output_frames(source_total)
    };

    // Ensure dest is creatable / writable before starting the heavy work.
    {
        let mut probe = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(req.dest)
            .with_context(|| format!("open {} for write", req.dest.display()))?;
        probe.write_all(&[]).ok();
    }

    let mut stream = begin_stream(req.encoder_id, &req.spec, req.dest, expected_out)?;
    if let Some(cb) = req.on_progress.as_deref_mut() {
        cb(0, source_total);
    }

    let mut start = 0u64;
    while start < source_total {
        let n = block_frames.min(source_total - start);
        let decoded = decode_range(req.source, start, n)
            .with_context(|| format!("decode {} @ {start}", req.source.display()))?;
        let selected = select_channels(&decoded, req.channel_indices);

        if resampler.is_passthrough() {
            stream.write_planar(&selected)?;
        } else {
            let mut converted = vec![Vec::new(); out_channels];
            resampler.process_planar(&selected, &mut converted)?;
            if planar_nonempty(&converted) {
                stream.write_planar(&converted)?;
            }
        }

        let done = (start + n).min(source_total);
        if let Some(cb) = req.on_progress.as_deref_mut() {
            cb(done, source_total);
        }
        start += n;
    }

    if !resampler.is_passthrough() {
        let mut converted = vec![Vec::new(); out_channels];
        resampler.flush_planar(&mut converted)?;
        if planar_nonempty(&converted) {
            stream.write_planar(&converted)?;
        }
    }

    stream.finish()?;
    finish_with_tags(req.dest, req.encoder_id, req.tags)?;
    Ok(())
}

fn planar_nonempty(planar: &[Vec<f32>]) -> bool {
    planar.iter().any(|ch| !ch.is_empty())
}

fn validate_paths(source: &Path, dest: &Path) -> Result<()> {
    if !source.exists() {
        bail!("source not found: {}", source.display());
    }
    // Readable check.
    let _ = fs::File::open(source)
        .with_context(|| format!("source not readable: {}", source.display()))?;

    let parent = dest.parent().filter(|p| !p.as_os_str().is_empty());
    match parent {
        Some(parent) if !parent.exists() => {
            bail!("output directory does not exist: {}", parent.display());
        }
        Some(parent) => {
            // Writable check: try creating a temp file in the parent.
            let probe_name = format!(".fieldassist-write-probe-{}", std::process::id());
            let probe_path = parent.join(probe_name);
            match OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&probe_path)
            {
                Ok(_) => {
                    let _ = fs::remove_file(&probe_path);
                }
                Err(err) => {
                    bail!(
                        "output directory not writable ({}): {err}",
                        parent.display()
                    );
                }
            }
        }
        None => {
            // Dest is a bare filename in the cwd; cwd writability is checked
            // when creating the dest file.
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode, probe_file, PcmFormat};

    fn write_sine_wav(path: &Path, channels: u16, frames: u32, sample_rate: u32) {
        use std::io::Write;
        let bits_per_sample: u16 = 16;
        let block_align = channels * bits_per_sample / 8;
        let byte_rate = sample_rate * u32::from(block_align);
        let data_len = frames * u32::from(block_align);
        let mut out = std::fs::File::create(path).unwrap();
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
            let sample = (0.5 * (t * 440.0 * std::f32::consts::TAU).sin() * i16::MAX as f32) as i16;
            for _ in 0..channels {
                out.write_all(&sample.to_le_bytes()).unwrap();
            }
        }
    }

    #[test]
    fn wav_to_wav_progress_and_probe() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("in.wav");
        let dest = dir.path().join("out.wav");
        write_sine_wav(&src, 2, 4096, 48_000);
        let mut progress = Vec::new();
        let tags = TagMap::new();
        transcode(TranscodeRequest {
            source: &src,
            dest: &dest,
            encoder_id: "wav",
            spec: EncodeSpec {
                sample_rate: 48_000,
                sample_format: Some(PcmFormat::S16),
                channel_count: 2,
            },
            channel_indices: &[0, 1],
            tags: &tags,
            block_frames: 1024,
            on_progress: Some(&mut |done, total| progress.push((done, total))),
        })
        .unwrap();
        assert!(!progress.is_empty());
        assert_eq!(progress.last().unwrap(), &(4096, 4096));
        let probed = probe_file(&dest).unwrap();
        assert_eq!(probed.sample_rate, 48_000);
        assert_eq!(probed.frame_count, 4096);
    }

    #[test]
    fn rate_and_format_convert() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("in.wav");
        let dest = dir.path().join("out.wav");
        write_sine_wav(&src, 1, 4800, 48_000);
        let tags = TagMap::new();
        transcode(TranscodeRequest {
            source: &src,
            dest: &dest,
            encoder_id: "wav",
            spec: EncodeSpec {
                sample_rate: 44_100,
                sample_format: Some(PcmFormat::S24),
                channel_count: 1,
            },
            channel_indices: &[0],
            tags: &tags,
            block_frames: 1024,
            on_progress: None,
        })
        .unwrap();
        let probed = probe_file(&dest).unwrap();
        assert_eq!(probed.sample_rate, 44_100);
        assert_eq!(probed.bits_per_sample, Some(24));
        let expected = (4800.0 * 44_100.0 / 48_000.0) as i64;
        let delta = (probed.frame_count as i64 - expected).unsigned_abs();
        assert!(delta < 256, "frames {} vs ~{expected}", probed.frame_count);
        let _ = decode(&dest).unwrap();
    }

    #[test]
    fn rejects_unknown_encoder() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("in.wav");
        let dest = dir.path().join("out.wav");
        write_sine_wav(&src, 1, 256, 48_000);
        let tags = TagMap::new();
        let err = transcode(TranscodeRequest {
            source: &src,
            dest: &dest,
            encoder_id: "nope",
            spec: EncodeSpec {
                sample_rate: 48_000,
                sample_format: Some(PcmFormat::S16),
                channel_count: 1,
            },
            channel_indices: &[0],
            tags: &tags,
            block_frames: 256,
            on_progress: None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("unknown encoder"), "{err}");
    }

    #[test]
    fn rejects_missing_source() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.wav");
        let tags = TagMap::new();
        let err = transcode(TranscodeRequest {
            source: &dir.path().join("missing.wav"),
            dest: &dest,
            encoder_id: "wav",
            spec: EncodeSpec {
                sample_rate: 48_000,
                sample_format: Some(PcmFormat::S16),
                channel_count: 1,
            },
            channel_indices: &[0],
            tags: &tags,
            block_frames: 256,
            on_progress: None,
        })
        .unwrap_err();
        assert!(
            err.to_string().contains("not found") || err.to_string().contains("not readable"),
            "{err}"
        );
    }

    #[test]
    fn rejects_missing_dest_parent() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("in.wav");
        write_sine_wav(&src, 1, 256, 48_000);
        let dest = dir.path().join("nope").join("out.wav");
        let tags = TagMap::new();
        let err = transcode(TranscodeRequest {
            source: &src,
            dest: &dest,
            encoder_id: "wav",
            spec: EncodeSpec {
                sample_rate: 48_000,
                sample_format: Some(PcmFormat::S16),
                channel_count: 1,
            },
            channel_indices: &[0],
            tags: &tags,
            block_frames: 256,
            on_progress: None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("does not exist"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_unwritable_dest_parent() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("in.wav");
        write_sine_wav(&src, 1, 256, 48_000);
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).unwrap();
        let mut perms = fs::metadata(&locked).unwrap().permissions();
        perms.set_mode(0o555);
        fs::set_permissions(&locked, perms).unwrap();
        let dest = locked.join("out.wav");
        let tags = TagMap::new();
        let err = transcode(TranscodeRequest {
            source: &src,
            dest: &dest,
            encoder_id: "wav",
            spec: EncodeSpec {
                sample_rate: 48_000,
                sample_format: Some(PcmFormat::S16),
                channel_count: 1,
            },
            channel_indices: &[0],
            tags: &tags,
            block_frames: 256,
            on_progress: None,
        });
        // Restore perms so tempdir cleanup works.
        let mut perms = fs::metadata(&locked).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&locked, perms).unwrap();
        let err = err.unwrap_err();
        assert!(err.to_string().contains("not writable"), "{err}");
    }

    #[test]
    fn rejects_unsupported_spec() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("in.wav");
        let dest = dir.path().join("out.flac");
        write_sine_wav(&src, 1, 4096, 48_000);
        let tags = TagMap::new();
        let err = transcode(TranscodeRequest {
            source: &src,
            dest: &dest,
            encoder_id: "flac",
            spec: EncodeSpec {
                sample_rate: 48_000,
                sample_format: Some(PcmFormat::F32), // FLAC does not support float
                channel_count: 1,
            },
            channel_indices: &[0],
            tags: &tags,
            block_frames: 1024,
            on_progress: None,
        })
        .unwrap_err();
        assert!(
            err.to_string().contains("cannot encode") || err.to_string().contains("FLAC"),
            "{err}"
        );
    }
}
