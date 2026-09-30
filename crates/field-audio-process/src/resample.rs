// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use anyhow::{bail, Context, Result};
use rubato::audioadapter::Adapter;
use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
use rubato::{Fft, FixedSync, Resampler};

/// Resample planar f32 channels from `from_rate` to `to_rate`.
pub fn resample_planar(planar: &[Vec<f32>], from_rate: u32, to_rate: u32) -> Result<Vec<Vec<f32>>> {
    if from_rate == 0 || to_rate == 0 {
        bail!("sample rate must be greater than 0");
    }
    if from_rate == to_rate {
        return Ok(planar.to_vec());
    }
    let channels = planar.len();
    if channels == 0 {
        return Ok(Vec::new());
    }
    let frames = planar.iter().map(|ch| ch.len()).min().unwrap_or(0);
    if frames == 0 {
        return Ok(vec![Vec::new(); channels]);
    }
    let input = SequentialSliceOfVecs::new(planar, channels, frames)
        .map_err(|err| anyhow::anyhow!("wrap input for resampler: {err}"))?;
    let mut resampler = Fft::<f32>::new(
        from_rate as usize,
        to_rate as usize,
        1024,
        channels,
        FixedSync::Both,
    )
    .context("create resampler")?;
    let output = resampler
        .process_all(&input, frames, None)
        .context("resample")?;
    let out_frames = output.frames();
    let data = output.take_data();
    let mut planar_out = vec![vec![0.0f32; out_frames]; channels];
    for frame in 0..out_frames {
        for ch in 0..channels {
            planar_out[ch][frame] = data[frame * channels + ch];
        }
    }
    Ok(planar_out)
}

/// Streaming bandlimited resampler with bounded memory (block + flush).
///
/// Matched rates pass blocks through without rubato. Differing rates use the
/// same FFT quality as [`resample_planar`].
pub struct StreamingResampler {
    from_rate: u32,
    to_rate: u32,
    channels: usize,
    input_frames_seen: u64,
    output_frames_emitted: u64,
    inner: Option<ActiveResampler>,
}

struct ActiveResampler {
    resampler: Fft<f32>,
    leftover: Vec<Vec<f32>>,
    input_scratch: Vec<Vec<f32>>,
    output_interleaved: Vec<f32>,
}

impl StreamingResampler {
    /// Build a resampler for `from_rate` → `to_rate` with `channels` planes.
    ///
    /// `chunk_frames` is the rubato FFT chunk size (typical: 1024).
    pub fn new(from_rate: u32, to_rate: u32, channels: usize, chunk_frames: usize) -> Result<Self> {
        if from_rate == 0 || to_rate == 0 {
            bail!("sample rate must be greater than 0");
        }
        let channels = channels.max(1);
        let inner = if from_rate == to_rate {
            None
        } else {
            let resampler = Fft::<f32>::new(
                from_rate as usize,
                to_rate as usize,
                chunk_frames.max(1),
                channels,
                FixedSync::Both,
            )
            .context("create streaming resampler")?;
            let max_in = resampler.input_frames_max();
            let max_out = resampler.output_frames_max();
            Some(ActiveResampler {
                resampler,
                leftover: vec![Vec::new(); channels],
                input_scratch: vec![vec![0.0f32; max_in]; channels],
                output_interleaved: vec![0.0f32; max_out.saturating_mul(channels)],
            })
        };
        Ok(Self {
            from_rate,
            to_rate,
            channels,
            input_frames_seen: 0,
            output_frames_emitted: 0,
            inner,
        })
    }

    /// Whether rates match (passthrough).
    pub fn is_passthrough(&self) -> bool {
        self.inner.is_none()
    }

    /// Approximate output length for `input_frames` source frames.
    pub fn expected_output_frames(&self, input_frames: u64) -> u64 {
        if self.from_rate == self.to_rate || self.from_rate == 0 {
            return input_frames;
        }
        ((input_frames as u128).saturating_mul(u128::from(self.to_rate))
            / u128::from(self.from_rate)) as u64
    }

    /// Push a planar block; append produced frames onto each channel of `out`.
    ///
    /// `out` must already have `channels` vectors (may be empty). Cleared
    /// output is the caller's responsibility when starting a fresh block.
    pub fn process_planar(&mut self, input: &[Vec<f32>], out: &mut Vec<Vec<f32>>) -> Result<()> {
        ensure_out_channels(out, self.channels);
        let frames = input.iter().map(|ch| ch.len()).min().unwrap_or(0);
        if frames == 0 {
            return Ok(());
        }
        if input.len() < self.channels {
            bail!(
                "SRC channel count mismatch: need {} got {}",
                self.channels,
                input.len()
            );
        }
        self.input_frames_seen = self.input_frames_seen.saturating_add(frames as u64);
        let before = out.first().map(|ch| ch.len()).unwrap_or(0);
        if self.inner.is_none() {
            for ch in 0..self.channels {
                out[ch].extend_from_slice(&input[ch][..frames]);
            }
            self.output_frames_emitted = self.output_frames_emitted.saturating_add(frames as u64);
            return Ok(());
        }
        let active = self.inner.as_mut().unwrap();
        for ch in 0..self.channels {
            active.leftover[ch].extend_from_slice(&input[ch][..frames]);
        }
        drain_ready(active, self.channels, out, None)?;
        let after = out.first().map(|ch| ch.len()).unwrap_or(0);
        self.output_frames_emitted = self
            .output_frames_emitted
            .saturating_add((after.saturating_sub(before)) as u64);
        Ok(())
    }

    /// Drain a partial leftover chunk at EOF and trim trailing pad frames.
    pub fn flush_planar(&mut self, out: &mut Vec<Vec<f32>>) -> Result<()> {
        ensure_out_channels(out, self.channels);
        let expected = self.expected_output_frames(self.input_frames_seen);
        let remaining = expected.saturating_sub(self.output_frames_emitted);
        let Some(active) = self.inner.as_mut() else {
            return Ok(());
        };
        let before = out.first().map(|ch| ch.len()).unwrap_or(0);
        drain_ready(active, self.channels, out, None)?;

        let have = active.leftover[0].len();
        if have > 0 {
            let need = active.resampler.input_frames_next();
            let indexing = rubato::Indexing::new().partial_len(have);
            for ch in 0..self.channels {
                if active.input_scratch[ch].len() < need {
                    active.input_scratch[ch].resize(need, 0.0);
                }
                active.input_scratch[ch].fill(0.0);
                let copy = have.min(need);
                active.input_scratch[ch][..copy].copy_from_slice(&active.leftover[ch][..copy]);
                active.leftover[ch].clear();
            }
            process_chunk(active, self.channels, need, out, Some(&indexing))?;
        }

        // Keep only as many new frames as still needed to reach `expected`.
        let after = out.first().map(|ch| ch.len()).unwrap_or(0);
        let produced = after.saturating_sub(before);
        let keep = (remaining as usize).min(produced);
        for ch in out.iter_mut() {
            ch.truncate(before + keep);
        }
        self.output_frames_emitted = self.output_frames_emitted.saturating_add(keep as u64);
        Ok(())
    }
}

fn ensure_out_channels(out: &mut Vec<Vec<f32>>, channels: usize) {
    while out.len() < channels {
        out.push(Vec::new());
    }
}

fn drain_ready(
    active: &mut ActiveResampler,
    channels: usize,
    out: &mut Vec<Vec<f32>>,
    indexing: Option<&rubato::Indexing>,
) -> Result<()> {
    loop {
        let need = active.resampler.input_frames_next();
        if need == 0 || active.leftover[0].len() < need {
            break;
        }
        for ch in 0..channels {
            if active.input_scratch[ch].len() < need {
                active.input_scratch[ch].resize(need, 0.0);
            }
            active.input_scratch[ch][..need].copy_from_slice(&active.leftover[ch][..need]);
            active.leftover[ch].drain(..need);
        }
        process_chunk(active, channels, need, out, indexing)?;
    }
    Ok(())
}

fn process_chunk(
    active: &mut ActiveResampler,
    channels: usize,
    need: usize,
    out: &mut Vec<Vec<f32>>,
    indexing: Option<&rubato::Indexing>,
) -> Result<()> {
    let out_frames = active.resampler.output_frames_next().max(1);
    let out_samples = out_frames.saturating_mul(channels);
    if active.output_interleaved.len() < out_samples {
        active.output_interleaved.resize(out_samples, 0.0);
    }
    let input = SequentialSliceOfVecs::new(&active.input_scratch, channels, need)
        .map_err(|err| anyhow::anyhow!("wrap SRC input: {err}"))?;
    use rubato::audioadapter_buffers::direct::InterleavedSlice;
    let mut output_adapter = InterleavedSlice::new_mut(
        &mut active.output_interleaved[..out_samples],
        channels,
        out_frames,
    )
    .map_err(|err| anyhow::anyhow!("wrap SRC output: {err}"))?;
    let (_consumed, written) = active
        .resampler
        .process_into_buffer(&input, &mut output_adapter, indexing)
        .context("streaming SRC")?;
    for frame in 0..written {
        for ch in 0..channels {
            out[ch].push(active.output_interleaved[frame * channels + ch]);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{sine, sine_snr_db, tone_to_image_db, two_tone};

    #[test]
    fn resample_48000_to_44100_changes_length() {
        let frames = 4800;
        let input = vec![(0..frames)
            .map(|i| (i as f32 / 48000.0 * 440.0 * std::f32::consts::TAU).sin())
            .collect::<Vec<_>>()];
        let out = resample_planar(&input, 48000, 44100).unwrap();
        let expected = (frames as f64 * 44100.0 / 48000.0).round() as usize;
        let got = out[0].len();
        let delta = (got as i64 - expected as i64).unsigned_abs() as usize;
        assert!(
            delta <= expected / 10 + 64,
            "expected about {expected} frames, got {got}"
        );
    }

    #[test]
    fn matching_rates_are_unchanged() {
        let input = vec![vec![0.1, 0.2, 0.3]];
        let out = resample_planar(&input, 44100, 44100).unwrap();
        assert_eq!(out, input);
    }

    #[test]
    fn resample_44100_to_48000_preserves_midband_snr() {
        let frames = 44_100;
        let input = vec![sine(frames, 44_100, 1_000.0, 0.5)];
        let out = resample_planar(&input, 44_100, 48_000).unwrap();
        let expected = (frames as f64 * 48_000.0 / 44_100.0).round() as usize;
        let delta = (out[0].len() as i64 - expected as i64).unsigned_abs() as usize;
        assert!(
            delta <= expected / 50 + 64,
            "len {} vs ~{expected}",
            out[0].len()
        );
        let start = 8192.min(out[0].len() / 4);
        let end = (start + 4096).min(out[0].len());
        let body = &out[0][start..end];
        let snr = sine_snr_db(body, 48_000, 1_000.0);
        assert!(snr > 80.0, "midband SNR {snr:.1} dB");
    }

    #[test]
    fn resample_44100_to_48000_rejects_two_tone_images() {
        let frames = 44_100;
        let input = vec![two_tone(frames, 44_100, 19_000.0, 20_000.0, 0.8)];
        let out = resample_planar(&input, 44_100, 48_000).unwrap();
        let skip = 1024.min(out[0].len() / 8);
        let body = &out[0][skip..out[0].len() - skip];
        let at_19 = crate::goertzel_power(body, 48_000, 19_000.0);
        let at_diff = crate::goertzel_power(body, 48_000, 1_000.0);
        let rejection = 10.0 * (at_19.max(1e-20) / at_diff.max(1e-20)).log10();
        assert!(
            rejection > 40.0,
            "difference product too strong ({rejection:.1} dB)"
        );
        let tone18 = vec![sine(frames, 44_100, 18_000.0, 0.5)];
        let out = resample_planar(&tone18, 44_100, 48_000).unwrap();
        let body = &out[0][skip..out[0].len() - skip];
        let ratio = tone_to_image_db(body, 48_000, 18_000.0, 12_000.0);
        assert!(ratio > 40.0, "18 kHz image rejection {ratio:.1} dB");
    }

    #[test]
    fn streaming_passthrough_copies_blocks() {
        let mut src = StreamingResampler::new(48_000, 48_000, 1, 1024).unwrap();
        assert!(src.is_passthrough());
        let mut out = vec![Vec::new()];
        src.process_planar(&[vec![0.1, 0.2, 0.3]], &mut out)
            .unwrap();
        assert_eq!(out[0], vec![0.1, 0.2, 0.3]);
        src.flush_planar(&mut out).unwrap();
        assert_eq!(out[0].len(), 3);
    }

    #[test]
    fn streaming_48k_to_44k_matches_oneshot_length() {
        let frames = 8_000;
        let input = vec![sine(frames, 48_000, 440.0, 0.5)];
        let oneshot = resample_planar(&input, 48_000, 44_100).unwrap();
        let mut stream = StreamingResampler::new(48_000, 44_100, 1, 1024).unwrap();
        let mut out = vec![Vec::new()];
        let block = 1024;
        let mut start = 0;
        while start < frames {
            let end = (start + block).min(frames);
            let chunk = vec![input[0][start..end].to_vec()];
            stream.process_planar(&chunk, &mut out).unwrap();
            start = end;
        }
        stream.flush_planar(&mut out).unwrap();
        let delta = (out[0].len() as i64 - oneshot[0].len() as i64).unsigned_abs() as usize;
        assert!(
            delta <= 128,
            "streaming {} vs oneshot {}",
            out[0].len(),
            oneshot[0].len()
        );
        let start = 2048.min(out[0].len() / 4);
        let end = (start + 2048).min(out[0].len());
        if end > start {
            let snr = sine_snr_db(&out[0][start..end], 44_100, 440.0);
            assert!(snr > 60.0, "streaming midband SNR {snr:.1} dB");
        }
    }
}
