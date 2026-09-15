// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! In-memory analysis stream caches (not persisted in `.facomp`).

use std::collections::HashMap;

use field_audio_process::{AnalysisKind, PEAK_BLOCK};

/// Per-channel hop-binned float stream (envelope, etc.).
#[derive(Debug, Clone, Default)]
pub struct FloatStreamSeries {
    /// Hop size in frames used when the bins were produced.
    pub hop: usize,
    /// Frames covered (may exceed `bins.len() * hop` by a partial last hop).
    pub covered_frames: u64,
    /// Per-channel bins.
    pub channels: Vec<Vec<f32>>,
}

impl FloatStreamSeries {
    /// Empty series for `channel_count` channels at `hop`.
    pub fn empty(channel_count: usize, hop: usize) -> Self {
        Self {
            hop,
            covered_frames: 0,
            channels: vec![Vec::new(); channel_count],
        }
    }

    /// Whether the stream covers `frames` timeline samples.
    pub fn covers(&self, frames: u64) -> bool {
        self.covered_frames >= frames && !self.channels.is_empty()
    }
}

/// Composition-level analysis streams (envelope peak, …). Min/max overview
/// bins remain on [`crate::ClipCache`].
#[derive(Debug, Clone, Default)]
pub struct AnalysisStreams {
    float_streams: HashMap<AnalysisKind, FloatStreamSeries>,
}

impl AnalysisStreams {
    /// Clear all derived streams (call after sample-changing edits).
    pub fn clear(&mut self) {
        self.float_streams.clear();
    }

    /// Look up a float stream by kind.
    pub fn float(&self, kind: AnalysisKind) -> Option<&FloatStreamSeries> {
        self.float_streams.get(&kind)
    }

    /// Mutable look up a float stream by kind.
    pub fn float_mut(&mut self, kind: AnalysisKind) -> Option<&mut FloatStreamSeries> {
        self.float_streams.get_mut(&kind)
    }

    /// Ensure a float stream slot exists for `kind`.
    pub fn ensure_float(
        &mut self,
        kind: AnalysisKind,
        channel_count: usize,
        hop: usize,
    ) -> &mut FloatStreamSeries {
        self.float_streams
            .entry(kind)
            .or_insert_with(|| FloatStreamSeries::empty(channel_count, hop))
    }

    /// Whether envelope-peak data covers the composition length.
    pub fn envelope_peak_ready(&self, frames: u64, channel_count: usize) -> bool {
        self.float(AnalysisKind::EnvelopePeak)
            .is_some_and(|s| s.channels.len() == channel_count && s.covers(frames))
    }

    /// Default hop for envelope peak bins.
    pub fn envelope_hop() -> usize {
        PEAK_BLOCK
    }
}
