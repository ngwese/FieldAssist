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
}
