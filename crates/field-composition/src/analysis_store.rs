// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! In-memory analysis stream caches (not persisted in `.facomp`).

use std::collections::HashMap;

use field_audio_process::{
    AnalysisKind, RecomputeScope, PEAK_BLOCK, SPECTRAL_BAND_COUNT, SPECTRAL_DB_FLOOR,
    SPECTRAL_FFT_SIZE,
};

use crate::edit_ranges::landing_ranges;
use crate::edl::EditOp;
use crate::tree::ClipTree;

/// Half-open timeline range `[start, end)` in frames.
pub type FrameRange = (u64, u64);

/// Merge overlapping / adjacent half-open ranges, clamped to `[0, frames)`.
pub fn merge_frame_ranges(ranges: Vec<FrameRange>, frames: u64) -> Vec<FrameRange> {
    let mut out: Vec<FrameRange> = ranges
        .into_iter()
        .filter_map(|(start, end)| {
            let start = start.min(frames);
            let end = end.min(frames);
            (end > start).then_some((start, end))
        })
        .collect();
    out.sort_by_key(|(start, _)| *start);
    let mut merged: Vec<FrameRange> = Vec::new();
    for (start, end) in out {
        if let Some((_, last_end)) = merged.last_mut() {
            if start <= *last_end {
                *last_end = (*last_end).max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
}

/// Expand each range by `radius` frames on both sides, then merge.
///
/// Zero-length seeds (`start == end`) become `[start-radius, start+radius)`.
pub fn expand_frame_ranges(ranges: &[FrameRange], radius: u64, frames: u64) -> Vec<FrameRange> {
    let expanded = ranges
        .iter()
        .map(|(start, end)| {
            let end = if end > start {
                *end
            } else {
                // Join point: expand around the point.
                start.saturating_add(1).min(frames)
            };
            (start.saturating_sub(radius), end.saturating_add(radius))
        })
        .collect();
    merge_frame_ranges(expanded, frames)
}

/// Snap range endpoints outward to hop boundaries.
pub fn hop_align_frame_ranges(ranges: &[FrameRange], hop: usize, frames: u64) -> Vec<FrameRange> {
    if hop == 0 {
        return merge_frame_ranges(ranges.to_vec(), frames);
    }
    let hop = hop as u64;
    let aligned = ranges
        .iter()
        .map(|(start, end)| {
            let start = (start / hop) * hop;
            let end = ((end + hop - 1) / hop) * hop;
            (start, end)
        })
        .collect();
    merge_frame_ranges(aligned, frames)
}

/// Subtract a completed half-open span from dirty ranges.
pub fn subtract_frame_range(
    dirty: &[FrameRange],
    done_start: u64,
    done_end: u64,
) -> Vec<FrameRange> {
    if done_start >= done_end {
        return dirty.to_vec();
    }
    let mut out = Vec::new();
    for &(start, end) in dirty {
        if end <= done_start || start >= done_end {
            out.push((start, end));
            continue;
        }
        if start < done_start {
            out.push((start, done_start));
        }
        if end > done_end {
            out.push((done_end, end));
        }
    }
    out.into_iter().filter(|(s, e)| s < e).collect()
}

/// Dirty seeds (pre-radius) for a sample-changing edit on the post-op timeline.
///
/// May include zero-length join points; callers must expand by radius before
/// merging so those joins survive.
pub fn analysis_dirty_seeds(op: &EditOp, pre_tree: &ClipTree, post_frames: u64) -> Vec<FrameRange> {
    let seeds = match op {
        EditOp::Init | EditOp::Copy { .. } => Vec::new(),
        EditOp::Delete { start, len } if *len > 0 => {
            vec![(*start, start.saturating_add(*len))]
        }
        EditOp::Delete { .. } => Vec::new(),
        EditOp::Cut { start, .. } | EditOp::Remove { start, .. } => {
            // Join at the hole; a zero-length seed expands by radius.
            vec![(*start, *start)]
        }
        EditOp::Paste { at, len } if *len > 0 => {
            vec![(*at, at.saturating_add(*len))]
        }
        EditOp::Paste { .. } => Vec::new(),
        EditOp::Duplicate { start, len } if *len > 0 => {
            let insert_at = start.saturating_add(*len);
            vec![(insert_at, insert_at.saturating_add(*len))]
        }
        EditOp::Duplicate { .. } => Vec::new(),
        EditOp::Trim { len, .. } if *len > 0 => {
            // New edges after remap to [0, len).
            vec![(0, 0), (*len, *len)]
        }
        EditOp::Trim { .. } => Vec::new(),
        EditOp::Move { from, len, dest } if *len > 0 => {
            let insert_at = if *dest > *from {
                dest.saturating_sub(*len)
            } else {
                *dest
            };
            let join = (*from).min(insert_at);
            vec![(join, join), (insert_at, insert_at.saturating_add(*len))]
        }
        EditOp::Move { .. } => Vec::new(),
        EditOp::Roll { .. } => landing_ranges(op, pre_tree),
    };
    // Clamp only; keep zero-length joins for radius expansion.
    seeds
        .into_iter()
        .map(|(start, end)| (start.min(post_frames), end.min(post_frames)))
        .collect()
}

fn hop_bin(frame: u64, hop: usize) -> usize {
    if hop == 0 {
        0
    } else {
        (frame / hop as u64) as usize
    }
}

fn hops_for_frames(frames: u64, hop: usize) -> usize {
    if hop == 0 || frames == 0 {
        0
    } else {
        ((frames as usize) + hop - 1) / hop
    }
}

/// Splice a packed hop×stride channel buffer through an edit (ClipCache-style).
fn splice_packed_channel(
    data: &[f32],
    hop: usize,
    stride: usize,
    op: &EditOp,
    post_frames: u64,
    empty: f32,
) -> Vec<f32> {
    let old_hops = if stride == 0 { 0 } else { data.len() / stride };
    let new_hops = hops_for_frames(post_frames, hop);
    let mut out = vec![empty; new_hops.saturating_mul(stride)];

    match op {
        EditOp::Init | EditOp::Copy { .. } | EditOp::Roll { .. } | EditOp::Delete { .. } => {
            let copy_hops = old_hops.min(new_hops);
            if stride > 0 {
                let n = copy_hops * stride;
                out[..n].copy_from_slice(&data[..n.min(data.len())]);
            }
        }
        EditOp::Cut { start, len } | EditOp::Remove { start, len } => {
            let left = hop_bin(*start, hop).min(old_hops);
            let right = hop_bin(start.saturating_add(*len), hop).min(old_hops);
            copy_hops_into(&mut out, 0, data, 0, left, stride);
            let suffix = old_hops.saturating_sub(right);
            copy_hops_into(&mut out, left, data, right, suffix, stride);
        }
        EditOp::Paste { at, len } => {
            let at_hop = hop_bin(*at, hop).min(old_hops);
            let insert_hops = hops_for_frames(*len, hop);
            copy_hops_into(&mut out, 0, data, 0, at_hop, stride);
            // Inserted hops stay `empty`.
            let dest = at_hop.saturating_add(insert_hops);
            let suffix = old_hops.saturating_sub(at_hop);
            copy_hops_into(&mut out, dest, data, at_hop, suffix, stride);
        }
        EditOp::Duplicate { start, len } => {
            let insert_at = start.saturating_add(*len);
            let at_hop = hop_bin(insert_at, hop).min(old_hops);
            let insert_hops = hops_for_frames(*len, hop);
            copy_hops_into(&mut out, 0, data, 0, at_hop, stride);
            let dest = at_hop.saturating_add(insert_hops);
            let suffix = old_hops.saturating_sub(at_hop);
            copy_hops_into(&mut out, dest, data, at_hop, suffix, stride);
        }
        EditOp::Trim { start, len } => {
            let left = hop_bin(*start, hop).min(old_hops);
            let right = hop_bin(start.saturating_add(*len), hop).min(old_hops);
            let keep = right.saturating_sub(left).min(new_hops);
            copy_hops_into(&mut out, 0, data, left, keep, stride);
        }
        EditOp::Move { from, len, dest } => {
            // Remove then insert empty hops at the landing position.
            let from_h = hop_bin(*from, hop).min(old_hops);
            let from_end = hop_bin(from.saturating_add(*len), hop).min(old_hops);
            let mut temp = Vec::with_capacity(old_hops.saturating_sub(from_end - from_h) * stride);
            append_hops(&mut temp, data, 0, from_h, stride);
            append_hops(
                &mut temp,
                data,
                from_end,
                old_hops.saturating_sub(from_end),
                stride,
            );
            let temp_hops = if stride == 0 { 0 } else { temp.len() / stride };
            let insert_at = if *dest > *from {
                dest.saturating_sub(*len)
            } else {
                *dest
            };
            let at_hop = hop_bin(insert_at, hop).min(temp_hops);
            let insert_hops = hops_for_frames(*len, hop);
            copy_hops_into(&mut out, 0, &temp, 0, at_hop, stride);
            let dest_hop = at_hop.saturating_add(insert_hops);
            let suffix = temp_hops.saturating_sub(at_hop);
            copy_hops_into(&mut out, dest_hop, &temp, at_hop, suffix, stride);
        }
    }
    out
}

fn copy_hops_into(
    dest: &mut [f32],
    dest_hop: usize,
    src: &[f32],
    src_hop: usize,
    hop_count: usize,
    stride: usize,
) {
    if hop_count == 0 || stride == 0 {
        return;
    }
    let dest_base = dest_hop.saturating_mul(stride);
    let src_base = src_hop.saturating_mul(stride);
    let n = hop_count.saturating_mul(stride);
    if dest_base + n > dest.len() || src_base + n > src.len() {
        let n = n
            .min(dest.len().saturating_sub(dest_base))
            .min(src.len().saturating_sub(src_base));
        if n > 0 {
            dest[dest_base..dest_base + n].copy_from_slice(&src[src_base..src_base + n]);
        }
        return;
    }
    dest[dest_base..dest_base + n].copy_from_slice(&src[src_base..src_base + n]);
}

fn append_hops(dest: &mut Vec<f32>, src: &[f32], src_hop: usize, hop_count: usize, stride: usize) {
    if hop_count == 0 || stride == 0 {
        return;
    }
    let src_base = src_hop.saturating_mul(stride);
    let n = hop_count
        .saturating_mul(stride)
        .min(src.len().saturating_sub(src_base));
    if n > 0 {
        dest.extend_from_slice(&src[src_base..src_base + n]);
    }
}

fn fill_packed_hops(
    data: &mut [f32],
    hop: usize,
    stride: usize,
    ranges: &[FrameRange],
    empty: f32,
) {
    if hop == 0 || stride == 0 {
        return;
    }
    let hop_count = data.len() / stride;
    for &(start, end) in ranges {
        let start_h = ((start / hop as u64) as usize).min(hop_count);
        let end_h = (((end + hop as u64 - 1) / hop as u64) as usize).min(hop_count);
        for h in start_h..end_h {
            let base = h * stride;
            for slot in &mut data[base..base + stride] {
                *slot = empty;
            }
        }
    }
}

/// Per-channel hop-binned float stream (envelope, etc.).
#[derive(Debug, Clone, Default)]
pub struct FloatStreamSeries {
    /// Hop size in frames used when the bins were produced.
    pub hop: usize,
    /// Frames covered (may exceed `bins.len() * hop` by a partial last hop).
    pub covered_frames: u64,
    /// Per-channel bins.
    pub channels: Vec<Vec<f32>>,
    /// Half-open frame ranges that still need rebuild (hop-aligned).
    pub dirty_ranges: Vec<FrameRange>,
}

impl FloatStreamSeries {
    /// Empty series for `channel_count` channels at `hop`.
    pub fn empty(channel_count: usize, hop: usize) -> Self {
        Self {
            hop,
            covered_frames: 0,
            channels: vec![Vec::new(); channel_count],
            dirty_ranges: Vec::new(),
        }
    }

    /// Whether the stream covers `frames` with no dirty holes.
    pub fn covers(&self, frames: u64) -> bool {
        self.covered_frames >= frames
            && !self.channels.is_empty()
            && self.dirty_ranges.is_empty()
            && !self.channels.iter().any(|ch| ch.is_empty())
    }

    /// Whether any hop bins are available to paint (holes allowed).
    pub fn has_data(&self) -> bool {
        !self.channels.is_empty() && self.channels.iter().any(|ch| !ch.is_empty())
    }

    /// Hops that may be read for paint (clamped to coverage).
    pub fn covered_hops(&self) -> usize {
        if self.hop == 0 {
            return 0;
        }
        ((self.covered_frames as usize) + self.hop - 1) / self.hop
    }

    /// True when `frame` falls inside a dirty range.
    pub fn is_dirty_frame(&self, frame: u64) -> bool {
        self.dirty_ranges
            .iter()
            .any(|&(start, end)| frame >= start && frame < end)
    }

    /// Mark ranges dirty, expand by radius, hop-align, and blank those bins.
    pub fn mark_dirty(&mut self, seeds: &[FrameRange], radius: u64, frames: u64) {
        let expanded = expand_frame_ranges(seeds, radius, frames);
        let aligned = hop_align_frame_ranges(&expanded, self.hop, frames);
        self.dirty_ranges = merge_frame_ranges(
            self.dirty_ranges
                .iter()
                .copied()
                .chain(aligned.iter().copied())
                .collect(),
            frames,
        );
        for ch in &mut self.channels {
            fill_float_hops(ch, self.hop, &self.dirty_ranges, 0.0);
        }
        if self.covered_frames == 0 && !self.channels.is_empty() && !self.channels[0].is_empty() {
            self.covered_frames = frames;
        }
    }

    /// Remove a completed write span from dirty ranges.
    pub fn clear_dirty_completed(&mut self, start: u64, end: u64) {
        self.dirty_ranges = subtract_frame_range(&self.dirty_ranges, start, end);
    }

    /// Splice hop bins through `op` and mark radius-padded dirty seeds.
    pub fn splice_through_op(
        &mut self,
        op: &EditOp,
        pre_tree: &ClipTree,
        post_frames: u64,
        radius: u64,
    ) {
        let hop = self.hop.max(1);
        for ch in &mut self.channels {
            *ch = splice_packed_channel(ch, hop, 1, op, post_frames, 0.0);
        }
        self.covered_frames = post_frames;
        let seeds = analysis_dirty_seeds(op, pre_tree, post_frames);
        let mut remapped = Vec::new();
        for &(start, end) in &self.dirty_ranges {
            remapped.extend(map_dirty_through_op(start, end, op));
        }
        self.dirty_ranges.clear();
        // Do not merge before expand — zero-length join seeds must survive.
        remapped.extend(seeds);
        self.mark_dirty(&remapped, radius, post_frames);
    }
}

fn fill_float_hops(data: &mut [f32], hop: usize, ranges: &[FrameRange], empty: f32) {
    if hop == 0 {
        return;
    }
    let hop_count = data.len();
    for &(start, end) in ranges {
        let start_h = ((start / hop as u64) as usize).min(hop_count);
        let end_h = (((end + hop as u64 - 1) / hop as u64) as usize).min(hop_count);
        for slot in &mut data[start_h..end_h] {
            *slot = empty;
        }
    }
}

fn map_dirty_through_op(start: u64, end: u64, op: &EditOp) -> Vec<FrameRange> {
    use crate::edit_ranges::map_range_through_op;
    map_range_through_op(start, end, op)
}

/// Per-channel hop × band spectrogram (dB), packed row-major.
#[derive(Debug, Clone, Default)]
pub struct SpectralStreamSeries {
    /// Hop size in frames.
    pub hop: usize,
    /// FFT size used when the series was produced.
    pub fft_size: usize,
    /// Log bands per hop.
    pub band_count: usize,
    /// Frames covered by the series.
    pub covered_frames: u64,
    /// Per channel: packed `hop_count * band_count` dB values.
    pub channels: Vec<Vec<f32>>,
    /// Half-open frame ranges that still need rebuild (hop-aligned).
    pub dirty_ranges: Vec<FrameRange>,
}

impl SpectralStreamSeries {
    /// Empty series for `channel_count` channels.
    pub fn empty(channel_count: usize) -> Self {
        Self {
            hop: PEAK_BLOCK,
            fft_size: SPECTRAL_FFT_SIZE,
            band_count: SPECTRAL_BAND_COUNT,
            covered_frames: 0,
            channels: vec![Vec::new(); channel_count],
            dirty_ranges: Vec::new(),
        }
    }

    /// Whether the stream covers `frames` with the expected shape and no holes.
    pub fn covers(&self, frames: u64, channel_count: usize) -> bool {
        self.covered_frames >= frames
            && self.channels.len() == channel_count
            && self.band_count == SPECTRAL_BAND_COUNT
            && self.hop > 0
            && !self.channels.is_empty()
            && self.dirty_ranges.is_empty()
            && !self.channels.iter().any(|ch| ch.is_empty())
    }

    /// Whether any spectral hops are available to paint (holes allowed).
    pub fn has_data(&self) -> bool {
        !self.channels.is_empty() && self.channels.iter().any(|ch| !ch.is_empty())
    }

    /// Hops that may be read for paint (clamped to coverage).
    pub fn covered_hops(&self) -> usize {
        if self.hop == 0 {
            return 0;
        }
        ((self.covered_frames as usize) + self.hop - 1) / self.hop
    }

    /// Number of spectral hops stored for `channel`.
    pub fn hop_count(&self, channel: usize) -> usize {
        let Some(data) = self.channels.get(channel) else {
            return 0;
        };
        if self.band_count == 0 {
            return 0;
        }
        data.len() / self.band_count
    }

    /// True when `frame` falls inside a dirty range.
    pub fn is_dirty_frame(&self, frame: u64) -> bool {
        self.dirty_ranges
            .iter()
            .any(|&(start, end)| frame >= start && frame < end)
    }

    /// Mark ranges dirty, expand by radius, hop-align, and blank those hops.
    pub fn mark_dirty(&mut self, seeds: &[FrameRange], radius: u64, frames: u64) {
        let expanded = expand_frame_ranges(seeds, radius, frames);
        let aligned = hop_align_frame_ranges(&expanded, self.hop, frames);
        self.dirty_ranges = merge_frame_ranges(
            self.dirty_ranges
                .iter()
                .copied()
                .chain(aligned.iter().copied())
                .collect(),
            frames,
        );
        let stride = self.band_count;
        let hop = self.hop;
        for ch in &mut self.channels {
            fill_packed_hops(ch, hop, stride, &self.dirty_ranges, SPECTRAL_DB_FLOOR);
        }
        if self.covered_frames == 0 && self.has_data() {
            self.covered_frames = frames;
        }
    }

    /// Remove a completed write span from dirty ranges.
    pub fn clear_dirty_completed(&mut self, start: u64, end: u64) {
        self.dirty_ranges = subtract_frame_range(&self.dirty_ranges, start, end);
    }

    /// Splice hop frames through `op` and mark radius-padded dirty seeds.
    pub fn splice_through_op(
        &mut self,
        op: &EditOp,
        pre_tree: &ClipTree,
        post_frames: u64,
        radius: u64,
    ) {
        let hop = self.hop.max(1);
        let stride = self.band_count.max(1);
        for ch in &mut self.channels {
            *ch = splice_packed_channel(ch, hop, stride, op, post_frames, SPECTRAL_DB_FLOOR);
        }
        self.covered_frames = post_frames;
        let seeds = analysis_dirty_seeds(op, pre_tree, post_frames);
        let mut remapped = Vec::new();
        for &(start, end) in &self.dirty_ranges {
            remapped.extend(map_dirty_through_op(start, end, op));
        }
        self.dirty_ranges.clear();
        // Do not merge before expand — zero-length join seeds must survive.
        remapped.extend(seeds);
        self.mark_dirty(&remapped, radius, post_frames);
    }
}

/// Composition-level analysis streams (envelope peak, spectral, …). Min/max
/// overview bins remain on [`crate::ClipCache`].
#[derive(Debug, Clone, Default)]
pub struct AnalysisStreams {
    float_streams: HashMap<AnalysisKind, FloatStreamSeries>,
    spectral: Option<SpectralStreamSeries>,
}

impl AnalysisStreams {
    /// Clear all derived streams (full wipe).
    pub fn clear(&mut self) {
        self.float_streams.clear();
        self.spectral = None;
    }

    /// Splice regional streams through `op`; drop full-timeline streams.
    pub fn invalidate_through_op(
        &mut self,
        op: &EditOp,
        pre_tree: &ClipTree,
        post_frames: u64,
        sample_rate: u32,
    ) {
        // Envelope peak (regional).
        if let Some(series) = self.float_streams.get_mut(&AnalysisKind::EnvelopePeak) {
            match AnalysisKind::EnvelopePeak.recompute_scope(sample_rate) {
                RecomputeScope::Regional(regional) => {
                    if series.has_data() {
                        series.splice_through_op(op, pre_tree, post_frames, regional.radius_frames);
                    } else {
                        self.float_streams.remove(&AnalysisKind::EnvelopePeak);
                    }
                }
                RecomputeScope::FullTimeline => {
                    self.float_streams.remove(&AnalysisKind::EnvelopePeak);
                }
            }
        }

        // Spectral (regional).
        if let Some(series) = self.spectral.as_mut() {
            match AnalysisKind::Spectral.recompute_scope(sample_rate) {
                RecomputeScope::Regional(regional) => {
                    if series.has_data() {
                        series.splice_through_op(op, pre_tree, post_frames, regional.radius_frames);
                    } else {
                        self.spectral = None;
                    }
                }
                RecomputeScope::FullTimeline => {
                    self.spectral = None;
                }
            }
        }
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

    /// Whether any envelope-peak bins are available to paint.
    pub fn envelope_peak_has_data(&self, channel_count: usize) -> bool {
        self.float(AnalysisKind::EnvelopePeak)
            .is_some_and(|s| s.channels.len() == channel_count && s.has_data())
    }

    /// Default hop for envelope peak bins.
    pub fn envelope_hop() -> usize {
        PEAK_BLOCK
    }

    /// Look up the spectral stream.
    pub fn spectral(&self) -> Option<&SpectralStreamSeries> {
        self.spectral.as_ref()
    }

    /// Mutable look up the spectral stream.
    pub fn spectral_mut(&mut self) -> Option<&mut SpectralStreamSeries> {
        self.spectral.as_mut()
    }

    /// Ensure a spectral stream slot exists.
    pub fn ensure_spectral(&mut self, channel_count: usize) -> &mut SpectralStreamSeries {
        if self
            .spectral
            .as_ref()
            .is_none_or(|s| s.channels.len() != channel_count)
        {
            self.spectral = Some(SpectralStreamSeries::empty(channel_count));
        }
        self.spectral.as_mut().expect("spectral just ensured")
    }

    /// Whether spectral data covers the composition length.
    pub fn spectral_ready(&self, frames: u64, channel_count: usize) -> bool {
        self.spectral()
            .is_some_and(|s| s.covers(frames, channel_count))
    }

    /// Whether any spectral hops are available to paint.
    pub fn spectral_has_data(&self, channel_count: usize) -> bool {
        self.spectral()
            .is_some_and(|s| s.channels.len() == channel_count && s.has_data())
    }

    /// Default hop for spectral frames.
    pub fn spectral_hop() -> usize {
        PEAK_BLOCK
    }

    /// Floor used when a spectral column has no data.
    pub fn spectral_db_floor() -> f32 {
        SPECTRAL_DB_FLOOR
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{Clip, ClipId};
    use field_audio_model::MediaId;

    fn media_tree(frames: u64) -> ClipTree {
        ClipTree::from_clip(Clip::from_media(ClipId(1), MediaId([0u8; 32]), 0, frames))
    }

    #[test]
    fn spectral_delete_marks_radius_padded_dirty_without_wipe() {
        let frames = 10_000u64;
        let hop = PEAK_BLOCK;
        let bands = SPECTRAL_BAND_COUNT;
        let hops = hops_for_frames(frames, hop);
        let mut series = SpectralStreamSeries::empty(1);
        series.channels[0] = vec![1.0; hops * bands];
        series.covered_frames = frames;

        let pre = media_tree(frames);
        let op = EditOp::Delete {
            start: 4_000,
            len: 500,
        };
        let radius = (SPECTRAL_FFT_SIZE - PEAK_BLOCK) as u64;
        series.splice_through_op(&op, &pre, frames, radius);

        assert!(series.has_data());
        assert!(!series.dirty_ranges.is_empty());
        assert!(!series.covers(frames, 1));

        // Far from the edit: values preserved.
        let far_hop = 0;
        assert_eq!(series.channels[0][far_hop * bands], 1.0);

        // Inside dirty: blanked to floor.
        let dirty_hop = (4_000 / hop as u64) as usize;
        assert_eq!(series.channels[0][dirty_hop * bands], SPECTRAL_DB_FLOOR);
    }

    #[test]
    fn spectral_cut_shortens_and_keeps_suffix() {
        let frames = 8_000u64;
        let hop = PEAK_BLOCK;
        let bands = SPECTRAL_BAND_COUNT;
        let hops = hops_for_frames(frames, hop);
        let mut series = SpectralStreamSeries::empty(1);
        // Tag each hop with its index in band 0.
        let mut data = vec![SPECTRAL_DB_FLOOR; hops * bands];
        for h in 0..hops {
            data[h * bands] = h as f32;
        }
        series.channels[0] = data;
        series.covered_frames = frames;

        let pre = media_tree(frames);
        let cut_start = 2_000u64;
        let cut_len = 1_000u64;
        let op = EditOp::Cut {
            start: cut_start,
            len: cut_len,
        };
        let post_frames = frames - cut_len;
        series.splice_through_op(&op, &pre, post_frames, 0);

        let left = (cut_start / hop as u64) as usize;
        let right = ((cut_start + cut_len) / hop as u64) as usize;
        // Join hop is blanked (zero-length seed expands to one frame); the next
        // reused suffix hop keeps its pre-cut tag.
        assert_eq!(series.channels[0][(left + 1) * bands], (right + 1) as f32);
        assert!(!series.dirty_ranges.is_empty());
        assert_eq!(series.hop_count(0), hops_for_frames(post_frames, hop));
    }
}
