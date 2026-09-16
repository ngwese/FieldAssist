// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Synthetic stimuli and spectral helpers for pipeline identity / SRC quality
//! tests. Generators produce planar `f32` channels; no files are required.

use std::f32::consts::TAU;

/// Unit impulse at frame 0 (1.0), silence afterward.
pub fn impulse(frames: usize, channels: usize) -> Vec<Vec<f32>> {
    let mut planes = vec![vec![0.0f32; frames]; channels.max(1)];
    if frames > 0 {
        planes[0][0] = 1.0;
    }
    planes
}

/// Left-channel impulse only (stereo isolation).
pub fn left_impulse(frames: usize) -> Vec<Vec<f32>> {
    impulse(frames, 2)
}

/// Silence.
pub fn silence(frames: usize, channels: usize) -> Vec<Vec<f32>> {
    vec![vec![0.0f32; frames]; channels.max(1)]
}

/// Constant DC.
pub fn dc(frames: usize, channels: usize, level: f32) -> Vec<Vec<f32>> {
    vec![vec![level; frames]; channels.max(1)]
}

/// Mono (or multi-channel identical) sine.
pub fn sine(frames: usize, sample_rate: u32, freq_hz: f32, amplitude: f32) -> Vec<f32> {
    let rate = sample_rate.max(1) as f32;
    (0..frames)
        .map(|i| (i as f32 / rate * freq_hz * TAU).sin() * amplitude)
        .collect()
}

/// Planar sine (same tone on every channel).
pub fn sine_planar(
    frames: usize,
    channels: usize,
    sample_rate: u32,
    freq_hz: f32,
    amplitude: f32,
) -> Vec<Vec<f32>> {
    let mono = sine(frames, sample_rate, freq_hz, amplitude);
    vec![mono; channels.max(1)]
}

/// Sum of two sines (classic IMD / alias probe).
pub fn two_tone(
    frames: usize,
    sample_rate: u32,
    freq_a: f32,
    freq_b: f32,
    amplitude: f32,
) -> Vec<f32> {
    let rate = sample_rate.max(1) as f32;
    let half = amplitude * 0.5;
    (0..frames)
        .map(|i| {
            let t = i as f32 / rate;
            (t * freq_a * TAU).sin() * half + (t * freq_b * TAU).sin() * half
        })
        .collect()
}

/// Alternating `+amplitude` / `-amplitude` (Nyquist at even lengths).
pub fn nyquist_square(frames: usize, amplitude: f32) -> Vec<f32> {
    (0..frames)
        .map(|i| if i % 2 == 0 { amplitude } else { -amplitude })
        .collect()
}

/// Linear frequency chirp from `f0` to `f1` Hz.
pub fn chirp(frames: usize, sample_rate: u32, f0: f32, f1: f32, amplitude: f32) -> Vec<f32> {
    let rate = sample_rate.max(1) as f32;
    let duration = frames.max(1) as f32 / rate;
    (0..frames)
        .map(|i| {
            let t = i as f32 / rate;
            let phase = TAU * (f0 * t + (f1 - f0) * t * t / (2.0 * duration.max(1e-9)));
            phase.sin() * amplitude
        })
        .collect()
}

/// Goertzel magnitude-squared at `freq_hz` over `samples`.
pub fn goertzel_power(samples: &[f32], sample_rate: u32, freq_hz: f32) -> f32 {
    let n = samples.len();
    if n == 0 || sample_rate == 0 {
        return 0.0;
    }
    let k = (0.5 + n as f32 * freq_hz / sample_rate as f32).floor();
    let w = TAU * k / n as f32;
    let coeff = 2.0 * w.cos();
    let mut s1 = 0.0f32;
    let mut s2 = 0.0f32;
    for &x in samples {
        let s0 = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let real = s1 - s2 * w.cos();
    let imag = s2 * w.sin();
    real * real + imag * imag
}

/// RMS of `samples`.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt()
}

/// Rough SNR in dB of `samples` against a least-squares sine at `freq_hz`.
pub fn sine_snr_db(samples: &[f32], sample_rate: u32, freq_hz: f32) -> f32 {
    if samples.is_empty() || sample_rate == 0 {
        return f32::NEG_INFINITY;
    }
    let rate = sample_rate as f32;
    let mut sum_c = 0.0f32;
    let mut sum_s = 0.0f32;
    let mut sum_cc = 0.0f32;
    let mut sum_ss = 0.0f32;
    let mut sum_cs = 0.0f32;
    for (i, &x) in samples.iter().enumerate() {
        let phase = i as f32 / rate * freq_hz * TAU;
        let c = phase.cos();
        let s = phase.sin();
        sum_c += x * c;
        sum_s += x * s;
        sum_cc += c * c;
        sum_ss += s * s;
        sum_cs += c * s;
    }
    let det = sum_cc * sum_ss - sum_cs * sum_cs;
    if det.abs() < 1e-20 {
        return f32::NEG_INFINITY;
    }
    let a = (sum_c * sum_ss - sum_s * sum_cs) / det;
    let b = (sum_s * sum_cc - sum_c * sum_cs) / det;
    let mut err = 0.0f32;
    let mut sig = 0.0f32;
    for (i, &x) in samples.iter().enumerate() {
        let phase = i as f32 / rate * freq_hz * TAU;
        let y = a * phase.cos() + b * phase.sin();
        let d = x - y;
        err += d * d;
        sig += y * y;
    }
    if err <= f32::EPSILON {
        return 120.0;
    }
    10.0 * (sig / err).log10()
}

/// Ratio of tone power at `signal_hz` to power at `image_hz`, in dB.
pub fn tone_to_image_db(samples: &[f32], sample_rate: u32, signal_hz: f32, image_hz: f32) -> f32 {
    let signal = goertzel_power(samples, sample_rate, signal_hz).max(1e-20);
    let image = goertzel_power(samples, sample_rate, image_hz).max(1e-20);
    10.0 * (signal / image).log10()
}

/// Maximum absolute difference between two equal-length mono buffers.
pub fn max_abs_err(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impulse_and_two_tone_shapes() {
        let imp = impulse(8, 1);
        assert_eq!(imp[0][0], 1.0);
        assert_eq!(imp[0][1], 0.0);
        let left = left_impulse(4);
        assert_eq!(left[0][0], 1.0);
        assert_eq!(left[1][0], 0.0);
        let tt = two_tone(100, 44100, 19000.0, 20000.0, 0.5);
        assert_eq!(tt.len(), 100);
    }

    #[test]
    fn goertzel_finds_sine() {
        let s = sine(4096, 48000, 1000.0, 0.5);
        let at = goertzel_power(&s, 48000, 1000.0);
        let off = goertzel_power(&s, 48000, 5000.0);
        assert!(at > off * 100.0, "at={at} off={off}");
        let snr = sine_snr_db(&s, 48000, 1000.0);
        assert!(snr > 80.0, "snr={snr}");
    }
}
