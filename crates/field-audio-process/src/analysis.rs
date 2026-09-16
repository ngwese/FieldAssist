// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Incremental analysis operations over planar PCM blocks.

use std::f32::consts::PI;
use std::sync::Arc;

use dasp::envelope::detect::Peak;
use dasp::envelope::Detector;
use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

use field_audio_model::{NewMarker, MARKER_TYPE_TRANSIENT};

use crate::peaks::PEAK_BLOCK;

/// Stable id for a built-in analysis operation / stream kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisKind {
    /// Overview min/max bins (waveform paint).
    MinMax,
    /// Peak envelope follower with a fixed smoothing window.
    EnvelopePeak,
    /// Onset / transient marker detection.
    Transients,
    /// Log-band spectrogram stream (Spectrum representation).
    Spectral,
}

impl AnalysisKind {
    /// Stable string id used in caches and progress labels.
    pub fn id(self) -> &'static str {
        match self {
            Self::MinMax => "minmax",
            Self::EnvelopePeak => "envelope_peak",
            Self::Transients => "transients",
            Self::Spectral => "spectral",
        }
    }

    /// Human-readable background job label.
    pub fn progress_label(self) -> &'static str {
        match self {
            Self::MinMax => "building peaks",
            Self::EnvelopePeak => "building envelope",
            Self::Transients => "detecting transients",
            Self::Spectral => "building spectrum",
        }
    }
}

/// Shared context passed at the start of an analysis pass.
#[derive(Debug, Clone, Copy)]
pub struct AnalysisCtx {
    /// Composition sample rate in Hz.
    pub sample_rate: u32,
    /// Number of planar channels in the read path.
    pub channel_count: usize,
}

/// Sink for discrete analysis outputs produced at pass end (or mid-pass flush).
#[derive(Debug, Default)]
pub struct AnalysisSink {
    /// Markers to insert after replacing the prior set in-range.
    pub markers: Vec<NewMarker>,
}

/// Fold one planar channel chunk into overview `(min, max)` bins.
pub fn fold_minmax_bins(
    samples: &[f32],
    peaks: &mut Vec<(f32, f32)>,
    global_min: &mut f32,
    global_max: &mut f32,
) {
    for chunk in samples.chunks(PEAK_BLOCK) {
        let mut pmin = f32::MAX;
        let mut pmax = f32::MIN;
        for &s in chunk {
            pmin = pmin.min(s);
            pmax = pmax.max(s);
            *global_min = (*global_min).min(s);
            *global_max = (*global_max).max(s);
        }
        peaks.push(if pmin <= pmax {
            (pmin, pmax)
        } else {
            (0.0, 0.0)
        });
    }
}

/// Peak-envelope follower: 5 ms attack / 300 ms release, hop-downsampled bins.
#[derive(Debug)]
pub struct EnvelopePeakOp {
    detectors: Vec<Detector<[f32; 1], Peak>>,
    hop: usize,
    hop_acc: Vec<f32>,
    hop_count: Vec<usize>,
    sample_rate: u32,
}

impl EnvelopePeakOp {
    /// Attack window in seconds.
    pub const ATTACK_SECS: f32 = 0.005;
    /// Release window in seconds.
    pub const RELEASE_SECS: f32 = 0.3;

    /// Create detectors for `channel_count` channels at `sample_rate`.
    pub fn new(sample_rate: u32, channel_count: usize) -> Self {
        let attack = (Self::ATTACK_SECS * sample_rate as f32).max(1.0);
        let release = (Self::RELEASE_SECS * sample_rate as f32).max(1.0);
        let detectors = (0..channel_count)
            .map(|_| Detector::peak(attack, release))
            .collect();
        Self {
            detectors,
            hop: PEAK_BLOCK,
            hop_acc: vec![0.0; channel_count],
            hop_count: vec![0; channel_count],
            sample_rate,
        }
    }

    /// Frames folded into each output bin.
    pub fn hop_frames(&self) -> usize {
        self.hop
    }

    /// Sample rate used to size the follower.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Consume planar channel samples and append hop bins to `out`.
    pub fn consume_channel(&mut self, channel: usize, samples: &[f32], out: &mut Vec<f32>) {
        let detector = &mut self.detectors[channel];
        for &sample in samples {
            let env = detector.next([sample])[0];
            let acc = &mut self.hop_acc[channel];
            let count = &mut self.hop_count[channel];
            *acc = acc.max(env);
            *count += 1;
            if *count >= self.hop {
                out.push(*acc);
                *acc = 0.0;
                *count = 0;
            }
        }
    }

    /// Flush a partial hop bin (if any) into `out`.
    pub fn flush_channel(&mut self, channel: usize, out: &mut Vec<f32>) {
        if self.hop_count[channel] > 0 {
            out.push(self.hop_acc[channel]);
            self.hop_acc[channel] = 0.0;
            self.hop_count[channel] = 0;
        }
    }
}

/// Fast/slow peak-envelope transient detector.
#[derive(Debug)]
pub struct TransientDetectOp {
    fast: Vec<Detector<[f32; 1], Peak>>,
    slow: Vec<Detector<[f32; 1], Peak>>,
    /// Absolute frame offset at the start of the current pass.
    frame_cursor: u64,
    /// Frames to skip after an onset.
    refractory: u64,
    next_allowed: Vec<u64>,
    threshold: f32,
    detections: Vec<u64>,
}

impl TransientDetectOp {
    const FAST_SECS: f32 = 0.005;
    const SLOW_SECS: f32 = 0.05;
    const REFRACTORY_SECS: f32 = 0.05;
    const THRESHOLD: f32 = 0.05;

    /// Create detectors for `channel_count` at `sample_rate`.
    pub fn new(sample_rate: u32, channel_count: usize) -> Self {
        let fast_f = (Self::FAST_SECS * sample_rate as f32).max(1.0);
        let slow_f = (Self::SLOW_SECS * sample_rate as f32).max(1.0);
        let refractory = ((Self::REFRACTORY_SECS * sample_rate as f32) as u64).max(1);
        Self {
            fast: (0..channel_count)
                .map(|_| Detector::peak(fast_f, fast_f))
                .collect(),
            slow: (0..channel_count)
                .map(|_| Detector::peak(slow_f, slow_f))
                .collect(),
            frame_cursor: 0,
            refractory,
            next_allowed: vec![0; channel_count],
            threshold: Self::THRESHOLD,
            detections: Vec::new(),
        }
    }

    /// Reset pass state before reading from frame 0 (or a target start).
    ///
    /// Envelope followers keep their state so a short pre-roll can warm them
    /// before [`Self::begin`] moves the cursor to the real range start.
    pub fn begin(&mut self, start_frame: u64) {
        self.frame_cursor = start_frame;
        self.detections.clear();
        for allowed in &mut self.next_allowed {
            *allowed = start_frame;
        }
    }

    /// Seconds of audio before a ranged start used to settle followers.
    pub fn warmup_frames(sample_rate: u32) -> u64 {
        ((Self::SLOW_SECS * 2.0 * sample_rate as f32) as u64).max(1)
    }

    /// Consume interleaved timeline progress across all channels in `planar`.
    ///
    /// Channels are scanned per-frame; an onset on any channel emits one marker.
    pub fn consume_planar(&mut self, planar: &[Vec<f32>]) {
        if planar.is_empty() {
            return;
        }
        let frames = planar[0].len();
        for i in 0..frames {
            let frame = self.frame_cursor + i as u64;
            let mut hit = false;
            for ch in 0..planar.len() {
                let sample = planar[ch][i];
                let fast = self.fast[ch].next([sample])[0];
                let slow = self.slow[ch].next([sample])[0];
                if frame >= self.next_allowed[ch] && (fast - slow) > self.threshold {
                    hit = true;
                    self.next_allowed[ch] = frame + self.refractory;
                }
            }
            if hit {
                self.detections.push(frame);
            }
        }
        self.frame_cursor += frames as u64;
    }

    /// Drain detected onset frames into `sink` as Transient markers.
    pub fn finish(&mut self, sink: &mut AnalysisSink) {
        for frame in self.detections.drain(..) {
            sink.markers
                .push(NewMarker::new(frame, MARKER_TYPE_TRANSIENT, None));
        }
    }
}

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
        // Oldest sample is at ring_pos (next write).
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
    // Force strictly non-decreasing edges and at least one bin per band when possible.
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
    use crate::peaks::build_peaks;

    #[test]
    fn envelope_decays_after_impulse() {
        let mut op = EnvelopePeakOp::new(1_000, 1);
        let mut impulse = vec![0.0f32; 400];
        impulse[0] = 1.0;
        let mut bins = Vec::new();
        op.consume_channel(0, &impulse, &mut bins);
        op.flush_channel(0, &mut bins);
        assert!(!bins.is_empty());
        // Later bins should be quieter than the first hop that saw the impulse.
        if bins.len() >= 2 {
            assert!(bins[0] >= bins[bins.len() - 1]);
        }
    }

    #[test]
    fn transient_detects_click_in_silence() {
        let mut op = TransientDetectOp::new(8_000, 1);
        op.begin(0);
        let mut samples = vec![0.0f32; 8_000];
        // Short burst so the fast follower leads the slow one.
        for s in &mut samples[4_000..4_040] {
            *s = 1.0;
        }
        op.consume_planar(&[samples]);
        let mut sink = AnalysisSink::default();
        op.finish(&mut sink);
        assert!(
            !sink.markers.is_empty(),
            "expected at least one transient marker"
        );
        let frame = sink.markers[0].frame;
        assert!((3_900..=4_100).contains(&frame));
    }

    #[test]
    fn fold_minmax_matches_build_peaks() {
        let samples: Vec<f32> = (0..1_000).map(|i| (i as f32) / 1000.0 - 0.5).collect();
        let expected = build_peaks(&samples);
        let mut got = Vec::new();
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        fold_minmax_bins(&samples, &mut got, &mut min, &mut max);
        assert_eq!(got, expected);
    }

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
}
