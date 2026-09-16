// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Overview min/max peak folding ([`MinMaxOp`]).

/// Number of samples folded into each overview peak bin.
pub const PEAK_BLOCK: usize = 256;

/// Stateful hop-folding min/max overview peak op.
#[derive(Debug)]
pub struct MinMaxOp {
    hop: usize,
    hop_min: Vec<f32>,
    hop_max: Vec<f32>,
    hop_count: Vec<usize>,
    global_min: Vec<f32>,
    global_max: Vec<f32>,
}

impl MinMaxOp {
    /// Create fold state for `channel_count` channels.
    pub fn new(channel_count: usize) -> Self {
        Self {
            hop: PEAK_BLOCK,
            hop_min: vec![f32::MAX; channel_count],
            hop_max: vec![f32::MIN; channel_count],
            hop_count: vec![0; channel_count],
            global_min: vec![f32::MAX; channel_count],
            global_max: vec![f32::MIN; channel_count],
        }
    }

    /// Frames folded into each output bin.
    pub fn hop_frames(&self) -> usize {
        self.hop
    }

    /// Running global min/max observed on `channel` (or `(0, 0)` if empty).
    pub fn global_min_max(&self, channel: usize) -> (f32, f32) {
        let min = self.global_min[channel];
        let max = self.global_max[channel];
        if min <= max {
            (min, max)
        } else {
            (0.0, 0.0)
        }
    }

    /// Combined global min/max across all channels.
    pub fn combined_global_min_max(&self) -> (f32, f32) {
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        for ch in 0..self.global_min.len() {
            let (cmin, cmax) = self.global_min_max(ch);
            if self.global_min[ch] <= self.global_max[ch] {
                min = min.min(cmin);
                max = max.max(cmax);
            }
        }
        if min <= max {
            (min, max)
        } else {
            (0.0, 0.0)
        }
    }

    /// Consume channel samples and append completed hop bins to `out`.
    pub fn consume_channel(&mut self, channel: usize, samples: &[f32], out: &mut Vec<(f32, f32)>) {
        for &sample in samples {
            let gmin = &mut self.global_min[channel];
            let gmax = &mut self.global_max[channel];
            *gmin = (*gmin).min(sample);
            *gmax = (*gmax).max(sample);
            let hmin = &mut self.hop_min[channel];
            let hmax = &mut self.hop_max[channel];
            let count = &mut self.hop_count[channel];
            *hmin = (*hmin).min(sample);
            *hmax = (*hmax).max(sample);
            *count += 1;
            if *count >= self.hop {
                out.push((*hmin, *hmax));
                *hmin = f32::MAX;
                *hmax = f32::MIN;
                *count = 0;
            }
        }
    }

    /// Flush a partial hop bin (if any) into `out`.
    pub fn flush_channel(&mut self, channel: usize, out: &mut Vec<(f32, f32)>) {
        if self.hop_count[channel] > 0 {
            out.push((self.hop_min[channel], self.hop_max[channel]));
            self.hop_min[channel] = f32::MAX;
            self.hop_max[channel] = f32::MIN;
            self.hop_count[channel] = 0;
        }
    }
}

/// Fold one planar channel chunk into overview `(min, max)` bins.
///
/// Convenience wrapper around a one-shot [`MinMaxOp`] for a single channel
/// buffer (updates `global_min` / `global_max` in place).
pub fn fold_minmax_bins(
    samples: &[f32],
    peaks: &mut Vec<(f32, f32)>,
    global_min: &mut f32,
    global_max: &mut f32,
) {
    let mut op = MinMaxOp::new(1);
    op.global_min[0] = *global_min;
    op.global_max[0] = *global_max;
    op.consume_channel(0, samples, peaks);
    op.flush_channel(0, peaks);
    let (min, max) = op.global_min_max(0);
    if op.global_min[0] <= op.global_max[0] {
        *global_min = min;
        *global_max = max;
    }
}

/// Build `(min, max)` peak bins over `samples` using [`PEAK_BLOCK`].
pub fn build_peaks(samples: &[f32]) -> Vec<(f32, f32)> {
    let mut op = MinMaxOp::new(1);
    let mut peaks = Vec::new();
    op.consume_channel(0, samples, &mut peaks);
    op.flush_channel(0, &mut peaks);
    peaks
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
    fn minmax_op_consume_flush_matches_build_peaks() {
        let samples: Vec<f32> = (0..1_000).map(|i| (i as f32) / 1000.0 - 0.5).collect();
        let expected = build_peaks(&samples);
        let mut op = MinMaxOp::new(1);
        let mut got = Vec::new();
        // Consume in uneven chunks to exercise hop carry.
        for chunk in samples.chunks(100) {
            op.consume_channel(0, chunk, &mut got);
        }
        op.flush_channel(0, &mut got);
        assert_eq!(got, expected);
    }
}
