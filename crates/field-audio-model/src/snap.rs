// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

const DEFAULT_SEARCH_RADIUS: usize = 4096;

fn is_zero_crossing(samples: &[f32], index: usize) -> bool {
    if index >= samples.len() {
        return false;
    }
    if samples[index] == 0.0 {
        return true;
    }
    if index + 1 < samples.len() {
        return samples[index].signum() != samples[index + 1].signum();
    }
    false
}

/// Find the nearest zero crossing to `sample`, searching outward up to `search_radius`.
pub fn nearest_zero_crossing(samples: &[f32], sample: usize, search_radius: usize) -> usize {
    if samples.is_empty() {
        return 0;
    }
    let sample = sample.min(samples.len() - 1);
    if is_zero_crossing(samples, sample) {
        return sample;
    }

    for offset in 1..=search_radius {
        if sample >= offset {
            let left = sample - offset;
            if is_zero_crossing(samples, left) {
                return left;
            }
        }
        let right = sample + offset;
        if right < samples.len() && is_zero_crossing(samples, right) {
            return right;
        }
    }

    sample
}

/// [`nearest_zero_crossing`] with the default search radius.
pub fn nearest_zero_crossing_default(samples: &[f32], sample: usize) -> usize {
    nearest_zero_crossing(samples, sample, DEFAULT_SEARCH_RADIUS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_to_nearest_crossing() {
        let samples = vec![-1.0, -0.5, -0.1, 0.1, 0.5, 1.0];
        assert_eq!(nearest_zero_crossing(&samples, 1, 4), 2);
        assert_eq!(nearest_zero_crossing(&samples, 4, 4), 2);
    }

    #[test]
    fn exact_zero_is_crossing() {
        let samples = vec![1.0, 0.0, -1.0];
        assert_eq!(nearest_zero_crossing(&samples, 1, 4), 1);
    }

    #[test]
    fn clamps_when_no_crossing_found() {
        let samples = vec![0.5, 0.6, 0.7];
        assert_eq!(nearest_zero_crossing(&samples, 1, 2), 1);
    }
}
