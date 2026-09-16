// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Log-band STFT spectrogram analysis op.

use std::f32::consts::PI;
use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

use super::minmax::PEAK_BLOCK;

/// FFT size for the shipping spectral stream.
pub const SPECTRAL_FFT_SIZE: usize = 1024;
/// Number of log-spaced magnitude bands stored per hop.
pub const SPECTRAL_BAND_COUNT: usize = 64;
/// Lowest frequency edge for log bands (Hz).
pub const SPECTRAL_FMIN_HZ: f32 = 20.0;
/// dB floor stored in the spectral stream.
pub const SPECTRAL_DB_FLOOR: f32 = -80.0;

struct ChannelStft {
    ring: Vec<f32>,
    ring_pos: usize,
    hop_fill: usize,
}

/// Overlapping STFT → log-band dB spectrogram (hop = [`PEAK_BLOCK`]).
pub struct SpectralOp {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    /// Inclusive start / exclusive end FFT bin index per log band.
    band_ranges: Vec<(usize, usize)>,
    channels: Vec<ChannelStft>,
    time_scratch: Vec<f32>,
    freq_scratch: Vec<Complex<f32>>,
    hop: usize,
    fft_size: usize,
    band_count: usize,
    sample_rate: u32,
}

impl std::fmt::Debug for SpectralOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpectralOp")
            .field("hop", &self.hop)
            .field("fft_size", &self.fft_size)
            .field("band_count", &self.band_count)
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels.len())
            .finish()
    }
}

impl SpectralOp {
    /// Create STFT state for `channel_count` at `sample_rate`.
    pub fn new(sample_rate: u32, channel_count: usize) -> Self {
        let fft_size = SPECTRAL_FFT_SIZE;
        let band_count = SPECTRAL_BAND_COUNT;
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(fft_size);
        let window = hann_window(fft_size);
        let band_ranges = log_band_ranges(sample_rate, fft_size, band_count);
        let time_scratch = fft.make_input_vec();
        let freq_scratch = fft.make_output_vec();
        let channels = (0..channel_count)
            .map(|_| ChannelStft {
                ring: vec![0.0; fft_size],
                ring_pos: 0,
                hop_fill: 0,
            })
            .collect();
        Self {
            fft,
            window,
            band_ranges,
            channels,
            time_scratch,
            freq_scratch,
            hop: PEAK_BLOCK,
            fft_size,
            band_count,
            sample_rate,
        }
    }

    /// Frames between successive spectral frames.
    pub fn hop_frames(&self) -> usize {
        self.hop
    }

    /// Number of log bands per spectral frame.
    pub fn band_count(&self) -> usize {
        self.band_count
    }

    /// FFT size used by this op.
    pub fn fft_size(&self) -> usize {
        self.fft_size
    }

    /// Sample rate used to build log band edges.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Frames of lookback needed before a hop-aligned mid-timeline consume.
    ///
    /// Equals `fft_size - hop` so the first emitted hop after priming matches a
    /// full left-to-right pass at that timeline position.
    pub fn warmup_frames(&self) -> usize {
        self.fft_size.saturating_sub(self.hop)
    }

    /// Fill the STFT ring without emitting frames (regional job lookback).
    ///
    /// Prefer a hop-aligned prime length so `hop_fill == 0` when consume starts.
    pub fn prime_channel(&mut self, channel: usize, samples: &[f32]) {
        for &sample in samples {
            let ch = &mut self.channels[channel];
            ch.ring[ch.ring_pos] = sample;
            ch.ring_pos = (ch.ring_pos + 1) % self.fft_size;
            ch.hop_fill += 1;
            if ch.hop_fill >= self.hop {
                ch.hop_fill = 0;
            }
        }
    }

    /// Consume channel samples and append packed hop×band dB values to `out`.
    pub fn consume_channel(&mut self, channel: usize, samples: &[f32], out: &mut Vec<f32>) {
        for &sample in samples {
            let ch = &mut self.channels[channel];
            ch.ring[ch.ring_pos] = sample;
            ch.ring_pos = (ch.ring_pos + 1) % self.fft_size;
            ch.hop_fill += 1;
            if ch.hop_fill >= self.hop {
                ch.hop_fill = 0;
                self.emit_frame(channel, out);
            }
        }
    }

    /// Flush a partial hop (zero-padded window) into `out` when any samples remain.
    pub fn flush_channel(&mut self, channel: usize, out: &mut Vec<f32>) {
        if self.channels[channel].hop_fill > 0 {
            self.channels[channel].hop_fill = 0;
            self.emit_frame(channel, out);
        }
    }

    fn emit_frame(&mut self, channel: usize, out: &mut Vec<f32>) {
        let ring_pos = self.channels[channel].ring_pos;
        for i in 0..self.fft_size {
            let idx = (ring_pos + i) % self.fft_size;
            self.time_scratch[i] = self.channels[channel].ring[idx] * self.window[i];
        }
        self.fft
            .process(&mut self.time_scratch, &mut self.freq_scratch)
            .expect("spectral FFT size matches planner");
        let inv_n = 1.0 / self.fft_size as f32;
        for &(start, end) in &self.band_ranges {
            let mut sum = 0.0f32;
            let mut count = 0usize;
            for bin in start..end {
                let c = self.freq_scratch[bin];
                let mag = (c.re * c.re + c.im * c.im).sqrt() * inv_n;
                sum += mag;
                count += 1;
            }
            let mag = if count == 0 { 0.0 } else { sum / count as f32 };
            let db = if mag > 1e-12 {
                (20.0 * mag.log10()).clamp(SPECTRAL_DB_FLOOR, 0.0)
            } else {
                SPECTRAL_DB_FLOOR
            };
            out.push(db);
        }
    }
}

fn hann_window(n: usize) -> Vec<f32> {
    if n <= 1 {
        return vec![1.0; n];
    }
    (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / (n - 1) as f32).cos())
        .collect()
}

fn log_band_ranges(sample_rate: u32, fft_size: usize, band_count: usize) -> Vec<(usize, usize)> {
    let n_bins = fft_size / 2 + 1;
    let nyquist = sample_rate as f32 * 0.5;
    let f_min = SPECTRAL_FMIN_HZ.min(nyquist * 0.5).max(1.0);
    let f_max = nyquist.max(f_min * 1.01);
    let log_min = f_min.ln();
    let log_max = f_max.ln();
    let mut edges = Vec::with_capacity(band_count + 1);
    for b in 0..=band_count {
        let t = b as f32 / band_count as f32;
        let freq = (log_min + (log_max - log_min) * t).exp();
        let bin = ((freq * fft_size as f32) / sample_rate as f32).round() as usize;
        edges.push(bin.clamp(1, n_bins));
    }
    for i in 1..edges.len() {
        if edges[i] <= edges[i - 1] {
            edges[i] = (edges[i - 1] + 1).min(n_bins);
        }
    }
    edges[band_count] = n_bins;
    let mut ranges = Vec::with_capacity(band_count);
    for b in 0..band_count {
        let start = edges[b].min(n_bins.saturating_sub(1));
        let end = edges[b + 1].max(start + 1).min(n_bins);
        ranges.push((start, end));
    }
    ranges
}

/// Expected log band index for a pure tone near `freq_hz` (best-effort).
pub fn spectral_band_for_freq(sample_rate: u32, freq_hz: f32) -> usize {
    let nyquist = sample_rate as f32 * 0.5;
    let f_min = SPECTRAL_FMIN_HZ.min(nyquist * 0.5).max(1.0);
    let f_max = nyquist.max(f_min * 1.01);
    let freq = freq_hz.clamp(f_min, f_max);
    let t = (freq.ln() - f_min.ln()) / (f_max.ln() - f_min.ln());
    ((t * SPECTRAL_BAND_COUNT as f32).floor() as usize).min(SPECTRAL_BAND_COUNT - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spectral_tone_peaks_near_expected_band() {
        let sr = 8_000u32;
        let freq = 1_000.0f32;
        let mut op = SpectralOp::new(sr, 1);
        let mut samples = vec![0.0f32; SPECTRAL_FFT_SIZE * 4];
        for (i, s) in samples.iter_mut().enumerate() {
            *s = (2.0 * PI * freq * i as f32 / sr as f32).sin();
        }
        let mut out = Vec::new();
        op.consume_channel(0, &samples, &mut out);
        op.flush_channel(0, &mut out);
        assert!(!out.is_empty());
        assert_eq!(out.len() % SPECTRAL_BAND_COUNT, 0);
        let expected = spectral_band_for_freq(sr, freq);
        let hops = out.len() / SPECTRAL_BAND_COUNT;
        let hop = hops.saturating_sub(1);
        let frame = &out[hop * SPECTRAL_BAND_COUNT..(hop + 1) * SPECTRAL_BAND_COUNT];
        let (peak_band, _) = frame
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap();
        assert!(
            (peak_band as i32 - expected as i32).abs() <= 2,
            "peak band {peak_band} expected near {expected}; frame={frame:?}"
        );
    }

    #[test]
    fn prime_then_consume_matches_full_pass_at_midpoint() {
        let sr = 8_000u32;
        let freq = 1_000.0f32;
        let frames = SPECTRAL_FFT_SIZE * 8;
        let samples: Vec<f32> = (0..frames)
            .map(|i| (2.0 * PI * freq * i as f32 / sr as f32).sin())
            .collect();

        let mut full = SpectralOp::new(sr, 1);
        let mut full_out = Vec::new();
        full.consume_channel(0, &samples, &mut full_out);
        full.flush_channel(0, &mut full_out);

        let hop = PEAK_BLOCK;
        let warmup = SPECTRAL_FFT_SIZE - hop;
        // Hop-aligned mid-timeline start after enough lookback.
        let dirty_start = ((warmup + hop - 1) / hop) * hop + hop * 4;
        let dirty_end = dirty_start + hop * 8;
        assert!(dirty_end <= frames);

        let mut regional = SpectralOp::new(sr, 1);
        regional.prime_channel(0, &samples[dirty_start - warmup..dirty_start]);
        let mut regional_out = Vec::new();
        regional.consume_channel(0, &samples[dirty_start..dirty_end], &mut regional_out);

        let start_hop = dirty_start / hop;
        let end_hop = dirty_end / hop;
        let expected_hops = end_hop - start_hop;
        assert_eq!(regional_out.len() / SPECTRAL_BAND_COUNT, expected_hops);

        for i in 0..expected_hops {
            let full_base = (start_hop + i) * SPECTRAL_BAND_COUNT;
            let reg_base = i * SPECTRAL_BAND_COUNT;
            let full_frame = &full_out[full_base..full_base + SPECTRAL_BAND_COUNT];
            let reg_frame = &regional_out[reg_base..reg_base + SPECTRAL_BAND_COUNT];
            assert_eq!(
                full_frame,
                reg_frame,
                "hop {} (timeline hop {}) mismatch",
                i,
                start_hop + i
            );
        }
    }
}
