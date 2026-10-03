// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Output period catalog and snap helpers.
//!
//! Period sizes are listed in frames. The catalog is common AoIP / AES67
//! packet sizes plus powers of two through 8192.

/// How to choose the CPAL output period when opening a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputPeriod {
    /// Probe the device default, then snap up to the period catalog.
    SnapTwicePlatformDefault,
    /// Request this many frames per period.
    Frames(u32),
}

/// Result of opening (or reopening) a stream with a period request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenedPeriod {
    /// Frame count requested as `BufferSize::Fixed`, when a listed size was used.
    ///
    /// `None` means the stream fell back to `BufferSize::Default`.
    pub requested_frames: Option<u32>,
}

/// AoIP-oriented sizes and powers of two through 8192, sorted ascending.
pub fn period_frame_catalog() -> Vec<u32> {
    let mut frames = vec![16, 32, 48, 64, 128, 192, 288, 480];
    for power in [16u32, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192] {
        if !frames.contains(&power) {
            frames.push(power);
        }
    }
    frames.sort_unstable();
    frames
}

/// Smallest catalog entry greater than or equal to `negotiated * 2`.
///
/// Exact hits stay put. Values between entries round up. Targets above the
/// largest catalog entry use that entry.
pub fn snap_period_frames(negotiated: u32, catalog: &[u32]) -> u32 {
    debug_assert!(!catalog.is_empty());
    let target = negotiated.saturating_mul(2).max(1);
    catalog
        .iter()
        .copied()
        .find(|&frames| frames >= target)
        .unwrap_or_else(|| *catalog.last().unwrap_or(&target))
}

/// Catalog entries strictly larger than `frames`, ascending.
pub fn larger_period_frames(frames: u32, catalog: &[u32]) -> impl Iterator<Item = u32> + '_ {
    catalog.iter().copied().filter(move |&f| f > frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_unique_sorted_and_covers_aoip_and_powers() {
        let catalog = period_frame_catalog();
        assert!(catalog.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(
            catalog,
            vec![16, 32, 48, 64, 128, 192, 256, 288, 480, 512, 1024, 2048, 4096, 8192]
        );
    }

    #[test]
    fn snap_doubles_and_rounds_up() {
        let catalog = period_frame_catalog();
        // Exact: 512 * 2 = 1024.
        assert_eq!(snap_period_frames(512, &catalog), 1024);
        // Between entries: 480 * 2 = 960 → 1024.
        assert_eq!(snap_period_frames(480, &catalog), 1024);
        // 144 * 2 = 288 exact.
        assert_eq!(snap_period_frames(144, &catalog), 288);
        // Below catalog: 1 * 2 = 2 → 16.
        assert_eq!(snap_period_frames(1, &catalog), 16);
        // At / above max: stay on 8192.
        assert_eq!(snap_period_frames(4096, &catalog), 8192);
        assert_eq!(snap_period_frames(8192, &catalog), 8192);
        assert_eq!(snap_period_frames(10_000, &catalog), 8192);
    }

    #[test]
    fn larger_period_frames_skips_equal_and_smaller() {
        let catalog = period_frame_catalog();
        let next: Vec<u32> = larger_period_frames(1024, &catalog).collect();
        assert_eq!(next, vec![2048, 4096, 8192]);
    }
}
