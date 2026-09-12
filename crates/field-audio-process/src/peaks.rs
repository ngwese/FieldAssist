// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

/// Number of samples folded into each overview peak bin.
pub const PEAK_BLOCK: usize = 256;

/// Build `(min, max)` peak bins over `samples` using [`PEAK_BLOCK`].
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

    #[test]
    fn peaks_cover_all_samples() {
        let samples: Vec<f32> = (0..1000).map(|i| (i as f32) / 1000.0 - 0.5).collect();
        let peaks = build_peaks(&samples);
        assert_eq!(peaks.len(), (1000 + PEAK_BLOCK - 1) / PEAK_BLOCK);
        let (min, max) = min_max_in_range(&samples, &peaks, 0.0, 1000.0);
        assert!(min < 0.0);
        assert!(max > 0.0);
    }
}
