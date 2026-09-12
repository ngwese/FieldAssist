// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Overlay and mutation API for [`crate::WaveformDisplay`].
//!
//! Host document types implement [`WaveformEditor`] using only UI-crate DTOs
//! ([`LaneScope`], [`PaintRegion`], [`PeakStatus`]). Model types such as
//! channel scopes and edit ids stay in the application layer and are mapped
//! at the trait boundary.

/// Which waveform lanes a paint/mutation operation covers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LaneScope {
    /// Every channel lane.
    All,
    /// Explicit channel indices.
    Channels(Vec<usize>),
}

impl LaneScope {
    /// Scope covering all lanes.
    pub fn all() -> Self {
        Self::All
    }

    /// Scope covering a single channel index.
    pub fn single(channel: usize) -> Self {
        Self::Channels(vec![channel])
    }

    /// Whether this scope includes `channel`.
    pub fn applies_to(&self, channel: usize) -> bool {
        match self {
            Self::All => true,
            Self::Channels(channels) => channels.contains(&channel),
        }
    }
}

/// Inclusive sample span for region overlay painting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaintRegion {
    /// Inclusive start sample.
    pub start: usize,
    /// Inclusive end sample.
    pub end: usize,
    /// Lanes this region covers.
    pub channels: LaneScope,
}

/// Thin progress overlay while peaks (or similar jobs) are building.
#[derive(Debug, Clone, PartialEq)]
pub struct PeakStatus {
    /// Human-readable status (for example `"building peaks 42%"`).
    pub message: String,
    /// Bar fill in `[0.0, 1.0]`.
    pub fraction: f32,
}

/// Selection, overlays, and region-drag mutations for the waveform widget.
///
/// Implemented by the host document; the display never sees model-crate types.
pub trait WaveformEditor: Send + Sync {
    /// Inclusive selection span used by Frame, if any.
    fn selection_span(&self) -> Option<(usize, usize)>;

    /// Playhead sample and lane coverage, if any.
    fn playhead(&self) -> Option<(usize, LaneScope)>;

    /// Whether zero-crossing snap is enabled.
    fn snap_zero_crossings(&self) -> bool;

    /// Toggle zero-crossing snap.
    fn toggle_zero_crossing_snap(&mut self);

    /// Lane coverage for a click/drag that started on `lane` (Alt = single).
    fn channel_lanes(&self, lane: usize, alt: bool) -> LaneScope;

    /// Begin a replace-selection drag at `anchor`.
    fn begin_replace(&mut self, anchor: usize, lanes: LaneScope, radius: usize);

    /// Begin an extend-selection drag at `sample`.
    fn begin_extend(&mut self, sample: usize, lanes: LaneScope, radius: usize);

    /// Begin an additive (disjoint) selection drag at `anchor`.
    fn begin_disjoint(&mut self, anchor: usize, lanes: LaneScope, radius: usize);

    /// Update an in-progress region drag.
    fn update_drag(&mut self, sample: usize, radius: usize);

    /// Commit an in-progress region drag.
    fn finish_drag(&mut self);

    /// Click that never became a drag (caret / additive caret).
    fn click_without_drag(&mut self, sample: usize, lanes: LaneScope, add: bool);

    /// Current selection regions for overlay paint.
    fn selection_regions(&self) -> Vec<PaintRegion>;

    /// Named collection regions for overlay paint.
    fn named_regions(&self) -> Vec<PaintRegion>;

    /// Markers as `(frame, rgba)` for overlay paint.
    fn markers_for_paint(&self) -> Vec<(u64, [f32; 4])>;

    /// Modified timeline ranges for the bottom edit bars.
    fn modified_ranges(&self) -> Vec<(u64, u64)>;

    /// Timeline ranges owned by edit `id` (for hover highlight / scroll).
    fn ranges_for_edit(&self, id: u64) -> Vec<(u64, u64)>;

    /// Optional peak-build progress for the thin overlay bar.
    fn peak_status(&self) -> Option<PeakStatus> {
        None
    }

    /// Zoom-anchor sample (typically the playhead), if any.
    fn selection_position_sample(&self) -> Option<usize>;
}
