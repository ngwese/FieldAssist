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
}
