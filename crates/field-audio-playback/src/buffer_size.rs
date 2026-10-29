// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Output buffer-size catalog and negotiation helpers.
//!
//! Sizes are listed in frames. The catalog is common AoIP / AES67 packet sizes
//! plus powers of two through 8192. A preferred Fixed size is not guaranteed;
//! open-time negotiation may accept another in-range size or platform Default.

/// How to choose the CPAL output buffer size when opening a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputBufferSize {
    /// Probe the device default, prefer ~2× within the supported range, then
    /// walk other in-range catalog sizes and fall back to platform Default.
    PreferPlatformDefault,
    /// Prefer this many frames per callback; fall back to
    /// [`PreferPlatformDefault`] negotiation if rejected.
    Frames(u32),
}

/// Result of opening (or reopening) a stream with a buffer-size request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenedPeriod {
    /// Frame count requested as `BufferSize::Fixed`, when a Fixed open was used.
    ///
    /// `None` means the stream opened with `BufferSize::Default`.
    pub requested_frames: Option<u32>,
    /// Actual buffer size reported by the opened stream.
    pub actual_frames: u32,
}

/// AoIP-oriented sizes and powers of two through 8192, sorted ascending.
pub fn buffer_size_frame_catalog() -> Vec<u32> {
    let mut frames = vec![16, 32, 48, 64, 128, 192, 288, 480];
    for power in [16u32, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192] {
        if !frames.contains(&power) {
            frames.push(power);
        }
    }
    frames.sort_unstable();
    frames
}

/// Alias for [`buffer_size_frame_catalog`] (settings UI / older call sites).
pub fn period_frame_catalog() -> Vec<u32> {
    buffer_size_frame_catalog()
}

/// Supported buffer-size bounds from CPAL, when known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferSizeRange {
    /// Inclusive minimum period size in frames.
    pub min: u32,
    /// Inclusive maximum period size in frames.
    pub max: u32,
}

impl BufferSizeRange {
    /// Whether `frames` lies inside this inclusive range.
    pub fn contains(self, frames: u32) -> bool {
        frames >= self.min && frames <= self.max
    }
}

/// Filter `catalog` to entries inside `range` (or the full catalog when `None`).
pub fn catalog_in_range(catalog: &[u32], range: Option<BufferSizeRange>) -> Vec<u32> {
    match range {
        None => catalog.to_vec(),
        Some(range) => catalog
            .iter()
            .copied()
            .filter(|&frames| range.contains(frames))
            .collect(),
    }
}

/// Preferred Fixed target: smallest in-range catalog entry ≥ `2 * negotiated`,
/// else largest ≤ that target, else nearest in-range entry.
pub fn preferred_target_frames(negotiated: u32, in_range: &[u32]) -> Option<u32> {
    if in_range.is_empty() {
        return None;
    }
    let target = negotiated.saturating_mul(2).max(1);
    if let Some(frames) = in_range.iter().copied().find(|&frames| frames >= target) {
        return Some(frames);
    }
    if let Some(frames) = in_range
        .iter()
        .copied()
        .rev()
        .find(|&frames| frames <= target)
    {
        return Some(frames);
    }
    // Unreachable when in_range is non-empty, but keep nearest for safety.
    Some(
        in_range
            .iter()
            .copied()
            .min_by_key(|&frames| frames.abs_diff(target))
            .unwrap_or(target),
    )
}

/// Fixed candidates: preferred target, then larger ascending, then smaller
/// descending, deduped. Empty when the filtered catalog is empty.
pub fn negotiate_fixed_candidates(negotiated: u32, in_range: &[u32]) -> Vec<u32> {
    let Some(target) = preferred_target_frames(negotiated, in_range) else {
        return Vec::new();
    };
    let mut candidates = Vec::with_capacity(in_range.len());
    candidates.push(target);
    for frames in in_range.iter().copied().filter(|&f| f > target) {
        candidates.push(frames);
    }
    for frames in in_range.iter().copied().rev().filter(|&f| f < target) {
        candidates.push(frames);
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_unique_sorted_and_covers_aoip_and_powers() {
        let catalog = buffer_size_frame_catalog();
        assert!(catalog.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(
            catalog,
            vec![16, 32, 48, 64, 128, 192, 256, 288, 480, 512, 1024, 2048, 4096, 8192]
        );
        assert_eq!(period_frame_catalog(), catalog);
    }

    #[test]
    fn preferred_target_doubles_when_in_range() {
        let catalog = buffer_size_frame_catalog();
        assert_eq!(preferred_target_frames(512, &catalog), Some(1024));
        assert_eq!(preferred_target_frames(480, &catalog), Some(1024));
        assert_eq!(preferred_target_frames(144, &catalog), Some(288));
        assert_eq!(preferred_target_frames(1, &catalog), Some(16));
    }

    #[test]
    fn preferred_target_clamps_when_range_caps_below_twice() {
        let in_range = catalog_in_range(
            &buffer_size_frame_catalog(),
            Some(BufferSizeRange { min: 16, max: 1024 }),
        );
        // 1024 * 2 = 2048 is out of range → largest ≤ 2048 in range = 1024.
        assert_eq!(preferred_target_frames(1024, &in_range), Some(1024));
        let candidates = negotiate_fixed_candidates(1024, &in_range);
        assert_eq!(candidates.first().copied(), Some(1024));
        assert!(candidates.contains(&512));
        assert!(!candidates.contains(&2048));
        // After target: larger (none), then smaller descending.
        assert_eq!(
            &candidates[1..],
            &[512, 480, 288, 256, 192, 128, 64, 48, 32, 16]
        );
    }

    #[test]
    fn unknown_range_keeps_full_catalog_and_walks_both_ways() {
        let catalog = buffer_size_frame_catalog();
        let in_range = catalog_in_range(&catalog, None);
        assert_eq!(in_range, catalog);
        let candidates = negotiate_fixed_candidates(1024, &in_range);
        assert_eq!(candidates[0], 2048);
        assert_eq!(&candidates[1..4], &[4096, 8192, 1024]);
        assert!(candidates.ends_with(&[16]));
    }

    #[test]
    fn empty_filtered_catalog_yields_no_fixed_candidates() {
        let in_range = catalog_in_range(
            &buffer_size_frame_catalog(),
            Some(BufferSizeRange {
                min: 9000,
                max: 9001,
            }),
        );
        assert!(in_range.is_empty());
        assert!(negotiate_fixed_candidates(1024, &in_range).is_empty());
        assert_eq!(preferred_target_frames(1024, &in_range), None);
    }
}
