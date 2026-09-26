// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use super::buffer::{Buffer, ChannelScope, RegionId};
use super::composition::{
    map_inclusive_through_inverse, map_inclusive_through_op, map_point_through_inverse,
    map_point_through_op, AnalysisKind, Composition, EditId, EditOp, MarkerId, MarkerType,
};
use super::regions::{RegionCollection, RegionEndpoint, SELECTION_COLLECTION};
use super::selection::SamplePosition;
use super::snap::nearest_zero_crossing;
use crate::monitor::MonitorChain;
use crate::progress::ProgressHandle;
use field_ui_components::{
    LaneScope, MarkerRow, MarkersData, PaintRegion, PeakStatus, RegionGroup, RegionRow,
    RegionsData, WaveformDataProvider, WaveformEditor, WaveformRepresentation,
};

const DRAG_THRESHOLD_SAMPLES: usize = 0;

pub struct BufferDocument {
    pub composition: Arc<RwLock<Composition>>,
    #[allow(dead_code)] // twin Arc also kept on DocumentViews for UI
    pub buffer: Arc<RwLock<Buffer>>,
    pub selection: RegionCollection,
    /// Header multi-select of composition channels (session-only).
    selected_channels: BTreeSet<usize>,
    /// Anchor for shift-extend channel header selection.
    channel_selection_anchor: usize,
    pub current_position: Option<SamplePosition>,
    pub snap_zero_crossings: bool,
    pub snap_to_marker: bool,
    pub snap_marker_disabled: HashSet<String>,
    /// When set, this composition restores its own monitor DSP parameters.
    pub monitor_params_pinned: bool,
    pub pinned_monitor_params: HashMap<MonitorChain, HashMap<String, f32>>,
    pub progress: ProgressHandle,
    /// Pull-based analysis requests drained by the app job loop.
    pub analysis_requests: Arc<Mutex<Vec<AnalysisKind>>>,
    /// View → Show Envelope Peak overlay.
    pub show_envelope_peak: bool,
    /// Active waveform body; mirrored from the app-global View menu choice.
    pub waveform_representation: WaveformRepresentation,
    /// Overview peak paint style from settings.
    pub peak_rendering: field_ui_components::PeakRendering,
    /// HSV value reduction for Threaded shell bars (`0.0..=1.0`).
    pub threaded_shell_value_reduce: f32,
    /// Ribbon amplitude scale in dBFS for Threaded peak paint.
    pub threaded_ribbon_db: f32,
    /// Shared peaks-pane height fraction for Peaks + Spectrum (0.15..=0.85).
    pub peaks_spectrum_split: f32,
    /// Analyze → Selection Only: limit envelope/transient jobs to the selection.
    pub analyze_selection_only: bool,
    /// Snapshot of analysis target ranges for the next envelope/transient job.
    ///
    /// `None` means the spawn path should derive the target. `Some(None)` means
    /// whole timeline. `Some(Some(ranges))` means half-open selection spans
    /// captured when the command ran (so a later menu click cannot clear them).
    pub pending_analysis_target: Mutex<Option<Option<Vec<(u64, u64)>>>>,
    region_drag_anchor: Option<usize>,
    region_drag_id: Option<RegionId>,
    latched_marker: Option<usize>,
    next_region_id: u64,
}

impl BufferDocument {
    #[cfg(test)]
    pub fn new(composition: Composition) -> Self {
        Self::with_shared(
            Arc::new(RwLock::new(composition)),
            Arc::new(RwLock::new(Buffer::empty())),
        )
    }

    pub fn with_shared(composition: Arc<RwLock<Composition>>, buffer: Arc<RwLock<Buffer>>) -> Self {
        Self {
            composition,
            buffer,
            selection: RegionCollection::new(SELECTION_COLLECTION),
            selected_channels: BTreeSet::new(),
            channel_selection_anchor: 0,
            current_position: Some(SamplePosition {
                sample: 0,
                channels: ChannelScope::all(),
            }),
            snap_zero_crossings: true,
            snap_to_marker: false,
            snap_marker_disabled: HashSet::new(),
            monitor_params_pinned: false,
            pinned_monitor_params: HashMap::new(),
            progress: ProgressHandle::new(),
            analysis_requests: Arc::new(Mutex::new(Vec::new())),
            show_envelope_peak: false,
            waveform_representation: WaveformRepresentation::Peaks,
            peak_rendering: field_ui_components::PeakRendering::Threaded,
            threaded_shell_value_reduce: field_ui_components::DEFAULT_THREADED_SHELL_VALUE_REDUCE,
            threaded_ribbon_db: field_ui_components::DEFAULT_THREADED_RIBBON_DB,
            peaks_spectrum_split: field_ui_components::DEFAULT_PEAKS_SPECTRUM_SPLIT,
            analyze_selection_only: false,
            pending_analysis_target: Mutex::new(None),
            region_drag_anchor: None,
            region_drag_id: None,
            latched_marker: None,
            next_region_id: 1,
        }
    }

    pub fn frames(&self) -> usize {
        self.composition.read().unwrap().frames() as usize
    }

    pub fn sample_rate(&self) -> u32 {
        self.composition.read().unwrap().sample_rate()
    }

    /// Queue `kind` for the app analysis job loop (deduped).
    pub fn request_analysis(&self, kind: AnalysisKind) {
        let mut queue = self.analysis_requests.lock().unwrap();
        if !queue.contains(&kind) {
            queue.push(kind);
        }
    }

    /// Take and clear pending analysis requests.
    pub fn take_analysis_requests(&self) -> Vec<AnalysisKind> {
        std::mem::take(&mut *self.analysis_requests.lock().unwrap())
    }

    pub fn is_loaded(&self) -> bool {
        self.frames() > 0
    }

    pub fn toggle_zero_crossing_snap(&mut self) {
        self.snap_zero_crossings = !self.snap_zero_crossings;
    }

    pub fn toggle_marker_snap(&mut self) {
        self.snap_to_marker = !self.snap_to_marker;
        if !self.snap_to_marker {
            self.latched_marker = None;
        }
    }

    pub fn marker_type_snaps(&self, name: &str) -> bool {
        !self.snap_marker_disabled.contains(name)
    }

    pub fn toggle_snap_marker_type(&mut self, name: &str) {
        if !self.snap_marker_disabled.remove(name) {
            self.snap_marker_disabled.insert(name.to_string());
        }
    }

    pub fn is_region_drag_active(&self) -> bool {
        self.region_drag_anchor.is_some()
    }

    fn snap_channel(&self, channel: usize, sample: usize) -> usize {
        if !self.snap_zero_crossings {
            return sample;
        }
        let frames = self.frames();
        if frames == 0 {
            return 0;
        }
        let radius = 4096;
        let start = sample.saturating_sub(radius);
        let end = (sample + radius + 1).min(frames);
        let mut buf = vec![0.0; end.saturating_sub(start)];
        let _ = self
            .composition
            .read()
            .unwrap()
            .read_channel(channel, start as u64, &mut buf);
        let local = nearest_zero_crossing(&buf, sample.saturating_sub(start), radius);
        start + local
    }

    fn snap_zero(&self, scope: &ChannelScope, sample: usize) -> usize {
        if !self.snap_zero_crossings {
            return sample;
        }
        let channel = match scope {
            ChannelScope::AllChannels => 0,
            ChannelScope::Channels(chs) => chs.first().copied().unwrap_or(0),
        };
        self.snap_channel(channel, sample)
    }

    pub fn snap_sample(&mut self, scope: &ChannelScope, sample: usize, radius: usize) -> usize {
        let sample = self.clamp_sample(sample);
        // Snap To Marker wins inside the latch radius; Zero Crossing is the
        // fallback when no marker is nearby (issue #13).
        if self.snap_to_marker {
            if let Some(latched) = self.latched_marker {
                if sample.abs_diff(latched) > radius {
                    self.latched_marker = None;
                } else {
                    return latched;
                }
            }
            if let Some(marker) = self.nearest_snap_marker(sample, radius) {
                self.latched_marker = Some(marker);
                return marker;
            }
        }
        self.snap_zero(scope, sample)
    }

    fn nearest_snap_marker(&self, sample: usize, radius: usize) -> Option<usize> {
        if radius == 0 {
            return None;
        }
        let composition = self.composition.read().unwrap();
        let mut best: Option<(usize, usize)> = None;
        for marker in composition.markers().iter() {
            if !self.marker_type_snaps(&marker.marker_type) {
                continue;
            }
            let frame = marker.frame as usize;
            let dist = sample.abs_diff(frame);
            if dist > radius {
                continue;
            }
            if best.is_none_or(|(best_dist, _)| dist < best_dist) {
                best = Some((dist, frame));
            }
        }
        best.map(|(_, frame)| frame)
    }

    fn clamp_sample(&self, sample: usize) -> usize {
        let max = self.frames().saturating_sub(1);
        sample.min(max)
    }

    pub fn sample_to_secs(&self, sample: usize) -> f64 {
        let rate = self.sample_rate();
        if rate == 0 {
            0.0
        } else {
            sample as f64 / f64::from(rate)
        }
    }

    pub fn normalized_region_bounds(start: usize, end: usize) -> (usize, usize) {
        if start <= end {
            (start, end)
        } else {
            (end, start)
        }
    }

    fn set_current_position_sample(&mut self, sample: usize, channels: ChannelScope) {
        self.current_position = Some(SamplePosition { sample, channels });
    }

    pub fn channel_scope_for_lane(&self, lane: usize, alt: bool) -> ChannelScope {
        if alt {
            ChannelScope::single(lane)
        } else {
            ChannelScope::all()
        }
    }

    pub fn set_position(&mut self, sample: usize, scope: ChannelScope) {
        let sample = self.clamp_sample(self.snap_zero(&scope, sample));
        self.clear_drag();
        self.set_current_position_sample(sample, scope);
    }

    fn clear_drag(&mut self) {
        self.region_drag_anchor = None;
        self.region_drag_id = None;
        self.latched_marker = None;
    }

    fn peek_next_region_id(&self) -> u64 {
        self.next_region_id
            .max(self.composition.read().unwrap().peek_next_region_id())
            .max(1)
    }

    fn commit_next_region_id(&mut self, next: u64) {
        self.next_region_id = next;
        self.composition.write().unwrap().set_next_region_id(next);
    }

    /// Counter used when allocating regions through the scripting bridge.
    pub(crate) fn next_region_id(&self) -> u64 {
        self.peek_next_region_id()
    }

    /// Synchronize a region allocation counter from the scripting bridge.
    pub(crate) fn set_next_region_id(&mut self, next: u64) {
        self.commit_next_region_id(next);
    }

    fn push_selection_region(
        &mut self,
        start: usize,
        end: usize,
        channels: ChannelScope,
        label: Option<String>,
    ) -> RegionId {
        let mut next = self.peek_next_region_id();
        let id = self
            .selection
            .alloc_push(start, end, channels, label, &mut next);
        self.commit_next_region_id(next);
        id
    }

    pub fn begin_region_replace(&mut self, anchor: usize, scope: ChannelScope, radius: usize) {
        let anchor = self.snap_sample(&scope, anchor, radius);
        self.selection.clear();
        let id = self.push_selection_region(anchor, anchor, scope, None);
        self.region_drag_anchor = Some(anchor);
        self.region_drag_id = Some(id);
    }

    pub fn begin_region_disjoint(&mut self, anchor: usize, scope: ChannelScope, radius: usize) {
        let anchor = self.snap_sample(&scope, anchor, radius);
        let id = self.push_selection_region(anchor, anchor, scope, None);
        self.region_drag_anchor = Some(anchor);
        self.region_drag_id = Some(id);
    }

    pub fn begin_region_extend(&mut self, sample: usize, scope: ChannelScope, radius: usize) {
        let sample = self.snap_sample(&scope, sample, radius);
        if self.selection.is_empty() {
            let anchor = self
                .current_position
                .as_ref()
                .map(|pos| pos.sample)
                .unwrap_or(0);
            let anchor = self.clamp_sample(anchor);
            let id = self.push_selection_region(anchor, sample, scope, None);
            self.region_drag_anchor = Some(anchor);
            self.region_drag_id = Some(id);
            return;
        }
        let Some((id, endpoint, _)) = self
            .selection
            .nearest_endpoint(sample, Some(&scope))
            .or_else(|| self.selection.nearest_endpoint(sample, None))
        else {
            return;
        };
        let Some(region) = self.selection.get(id) else {
            return;
        };
        let anchor = match endpoint {
            RegionEndpoint::Start => region.end,
            RegionEndpoint::End => region.start,
        };
        if let Some(region) = self.selection.get_mut(id) {
            let (start, end) = Self::normalized_region_bounds(anchor, sample);
            region.start = start;
            region.end = end;
        }
        self.selection.normalize();
        self.region_drag_anchor = Some(anchor);
        self.region_drag_id = self
            .selection
            .regions
            .iter()
            .find(|region| {
                region.contains(sample, 0) || (region.start..=region.end).contains(&sample)
            })
            .map(|region| region.id)
            .or(Some(id));
    }

    pub fn update_region_drag(&mut self, sample: usize, radius: usize) {
        let Some(anchor) = self.region_drag_anchor else {
            return;
        };
        let Some(id) = self.region_drag_id else {
            return;
        };
        let Some(channels) = self.selection.get(id).map(|region| region.channels.clone()) else {
            return;
        };
        let sample = self.snap_sample(&channels, sample, radius);
        let (start, end) = Self::normalized_region_bounds(anchor, sample);
        if let Some(region) = self.selection.get_mut(id) {
            region.start = start;
            region.end = end;
        }
        self.selection.normalize();
        self.region_drag_id = self
            .selection
            .regions
            .iter()
            .find(|region| {
                region.channels == channels && region.start <= start && region.end >= end
            })
            .map(|region| region.id)
            .or_else(|| {
                self.selection
                    .regions
                    .iter()
                    .find(|region| (region.start..=region.end).contains(&sample))
                    .map(|region| region.id)
            });
    }

    pub fn finish_region_drag(&mut self) {
        let id = self.region_drag_id;
        self.clear_drag();
        let Some(id) = id else {
            return;
        };
        let Some(region) = self.selection.get(id).cloned() else {
            return;
        };
        let (start, end) = Self::normalized_region_bounds(region.start, region.end);
        if end.saturating_sub(start) <= DRAG_THRESHOLD_SAMPLES {
            self.selection.remove(id);
            self.set_current_position_sample(start, region.channels);
            return;
        }
        self.set_current_position_sample(end, region.channels);
        self.selection.normalize();
    }

    pub fn click_without_drag(&mut self, sample: usize, scope: ChannelScope, add: bool) {
        self.clear_drag();
        if !add {
            self.selection.clear();
        }
        let sample = self.clamp_sample(self.snap_zero(&scope, sample));
        self.set_current_position_sample(sample, scope);
    }

    #[allow(dead_code)]
    pub fn add_labeled_region(
        &mut self,
        start: usize,
        end: usize,
        channels: ChannelScope,
        label: Option<String>,
        collection: &str,
    ) -> RegionId {
        let start = self.clamp_sample(start);
        let end = self.clamp_sample(end);
        if collection == SELECTION_COLLECTION {
            return self.push_selection_region(start, end, channels, label);
        }
        let id = self
            .composition
            .write()
            .unwrap()
            .add_named_region(collection, start, end, channels, label)
            .unwrap_or(RegionId(0));
        self.next_region_id = self.peek_next_region_id();
        id
    }

    pub fn remove_region(&mut self, id: RegionId) -> bool {
        if self.selection.remove(id) {
            return true;
        }
        let mut composition = self.composition.write().unwrap();
        for collection in composition.collections().to_vec() {
            if collection.regions.iter().any(|region| region.id == id) {
                if let Some(col) = composition.collection_mut(&collection.name) {
                    return col.remove(id);
                }
            }
        }
        false
    }

    #[allow(dead_code)]
    pub fn collection_names(&self) -> Vec<String> {
        let mut names = vec![SELECTION_COLLECTION.to_string()];
        for collection in self.composition.read().unwrap().collections() {
            names.push(collection.name.clone());
        }
        names
    }

    #[allow(dead_code)]
    pub fn ensure_named_collection(&mut self, name: &str) {
        if name != SELECTION_COLLECTION {
            self.composition.write().unwrap().ensure_collection(name);
        }
    }

    #[allow(dead_code)] // retained for non-script UI/tooling
    #[allow(dead_code)]
    pub fn adopt_collection_as_selection(&mut self, name: &str) {
        if name == SELECTION_COLLECTION {
            return;
        }
        let other = self.composition.read().unwrap().collection(name).cloned();
        self.clear_drag();
        match other {
            Some(other) => {
                let mut next = self.peek_next_region_id();
                self.selection.copy_regions_from(&other, &mut next);
                self.commit_next_region_id(next);
            }
            None => self.selection.clear(),
        }
    }

    pub fn select_range(&mut self, start: usize, stop: usize, channels: ChannelScope) {
        let start = self.clamp_sample(start);
        let stop = self.clamp_sample(stop);
        let (start, end) = Self::normalized_region_bounds(start, stop);
        self.clear_drag();
        self.set_current_position_sample(end, channels.clone());
        let mut next = self.peek_next_region_id();
        self.selection.replace_with(crate::model::Region::new(
            RegionId(next),
            start,
            end,
            channels,
        ));
        next += 1;
        self.commit_next_region_id(next);
    }

    pub fn select_all(&mut self) {
        let frames = self.frames();
        if frames == 0 {
            self.clear_selection();
            return;
        }
        self.select_range(0, frames.saturating_sub(1), ChannelScope::all());
    }

    pub fn clear_selection(&mut self) {
        self.clear_drag();
        self.selection.clear();
        self.selected_channels.clear();
        self.channel_selection_anchor = 0;
    }

    /// Currently selected waveform channel headers (sorted).
    pub fn selected_channels(&self) -> Vec<usize> {
        self.selected_channels.iter().copied().collect()
    }

    /// Whether any channel headers are selected.
    pub fn has_channel_selection(&self) -> bool {
        !self.selected_channels.is_empty()
    }

    /// Click a channel header: replace, shift-extend, or secondary-toggle.
    pub fn click_channel_header(&mut self, index: usize, shift: bool, disjoint: bool) {
        let count = self.composition.read().unwrap().channel_count();
        if index >= count {
            return;
        }
        if shift {
            let start = self.channel_selection_anchor.min(index);
            let end = self.channel_selection_anchor.max(index);
            let range = start..=end;
            if disjoint {
                self.selected_channels.extend(range);
            } else {
                self.selected_channels = range.collect();
            }
        } else if disjoint {
            if !self.selected_channels.remove(&index) {
                self.selected_channels.insert(index);
            }
            self.channel_selection_anchor = index;
        } else if self.selected_channels.len() == 1 && self.selected_channels.contains(&index) {
            self.selected_channels.clear();
            self.channel_selection_anchor = index;
        } else {
            self.selected_channels.clear();
            self.selected_channels.insert(index);
            self.channel_selection_anchor = index;
        }
    }

    pub fn clear_channel_selection(&mut self) {
        self.selected_channels.clear();
        self.channel_selection_anchor = 0;
    }

    pub fn invert_selection(&mut self) {
        let frames = self.frames();
        if frames == 0 {
            self.clear_selection();
            return;
        }
        let last = frames.saturating_sub(1);
        let channel_count = self.composition.read().unwrap().channel_count();
        self.clear_drag();
        let mut next = self.peek_next_region_id();
        self.selection.invert(last, channel_count.max(1), &mut next);
        self.commit_next_region_id(next);
        if let Some(region) = self.selection.regions.last() {
            self.set_current_position_sample(region.end, region.channels.clone());
        }
    }

    pub fn add_marker(
        &mut self,
        sample: usize,
        marker_type: &str,
        note: Option<String>,
    ) -> Option<MarkerId> {
        let sample = self.clamp_sample(sample);
        self.composition
            .write()
            .unwrap()
            .add_marker(sample as u64, marker_type, note)
    }

    pub fn add_marker_of_type(&mut self, sample: usize, marker_type: &str) -> Option<MarkerId> {
        self.add_marker(sample, marker_type, None)
    }

    pub fn marker_types(&self) -> Vec<MarkerType> {
        self.composition.read().unwrap().marker_types().to_vec()
    }

    #[allow(dead_code)] // retained for non-script UI/tooling
    #[allow(dead_code)]
    pub fn add_marker_type(&mut self, name: &str, color: [f32; 4]) -> bool {
        self.composition
            .write()
            .unwrap()
            .add_marker_type(name, color)
    }

    #[allow(dead_code)]
    pub fn remove_marker_type(&mut self, name: &str) -> bool {
        self.composition.write().unwrap().remove_marker_type(name)
    }

    pub fn remove_marker(&mut self, id: MarkerId) -> bool {
        self.composition.write().unwrap().remove_marker(id)
    }

    #[allow(dead_code)]
    pub fn remove_marker_at(&mut self, sample: usize) -> bool {
        self.composition
            .write()
            .unwrap()
            .remove_marker_at(sample as u64)
    }

    #[allow(dead_code)]
    pub fn remove_marker_at_type(&mut self, sample: usize, marker_type: &str) -> bool {
        self.composition
            .write()
            .unwrap()
            .remove_marker_at_type(sample as u64, marker_type)
    }

    #[allow(dead_code)] // retained for non-script UI/tooling
    #[allow(dead_code)]
    pub fn remove_marker_by_type(&mut self, marker_type: &str) -> usize {
        self.composition
            .write()
            .unwrap()
            .remove_marker_by_type(marker_type)
    }

    pub fn selection_position_sample(&self) -> Option<usize> {
        self.current_position.as_ref().map(|pos| pos.sample)
    }

    pub fn set_position_from_playback(&mut self, sample: usize, scope: ChannelScope) {
        if self.is_region_drag_active() {
            return;
        }
        let sample = self.clamp_sample(sample);
        if self
            .current_position
            .as_ref()
            .is_some_and(|pos| pos.sample == sample)
        {
            return;
        }
        self.set_current_position_sample(sample, scope);
    }

    pub fn reset_for_new_buffer(&mut self) {
        self.selection.clear();
        self.current_position = Some(SamplePosition {
            sample: 0,
            channels: ChannelScope::all(),
        });
        self.clear_drag();
    }

    pub fn selection_spans(&self) -> Vec<(u64, u64)> {
        self.selection.edit_spans()
    }

    /// Capture the target for the next envelope/transient analysis job.
    ///
    /// Returns `false` when Selection Only is on and the selection is empty.
    pub fn snapshot_analysis_target(&self) -> bool {
        let target = if self.analyze_selection_only {
            let spans = self.selection_spans();
            if spans.is_empty() {
                return false;
            }
            Some(
                spans
                    .into_iter()
                    .map(|(start, len)| (start, start.saturating_add(len)))
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        };
        *self.pending_analysis_target.lock().unwrap() = Some(target);
        true
    }

    /// Take a previously snapshotted analysis target, if any.
    pub fn take_pending_analysis_target(&self) -> Option<Option<Vec<(u64, u64)>>> {
        self.pending_analysis_target.lock().unwrap().take()
    }

    pub fn caret_frame(&self) -> u64 {
        self.current_position
            .as_ref()
            .map(|pos| pos.sample as u64)
            .unwrap_or(0)
    }

    fn after_tree_changed(&mut self, from_cursor: usize) {
        let to_cursor = self.composition.read().unwrap().edit_cursor();
        self.remap_between_cursors(from_cursor, to_cursor);
        self.clamp_playhead_and_selection();
    }

    fn remap_between_cursors(&mut self, from: usize, to: usize) {
        let ops: Vec<EditOp> = self
            .composition
            .read()
            .unwrap()
            .edits()
            .iter()
            .map(|edit| edit.op.clone())
            .collect();
        if to > from {
            for op in &ops[from + 1..=to] {
                self.remap_through_op(op);
            }
        } else if to < from {
            for op in ops[to + 1..=from].iter().rev() {
                self.remap_through_inverse(op);
            }
        }
    }

    fn remap_through_op(&mut self, op: &EditOp) {
        if let Some(pos) = self.current_position.as_mut() {
            pos.sample = map_point_through_op(pos.sample as u64, op) as usize;
        }
        self.selection
            .remap(|start, end| map_inclusive_through_op(start, end, op));
    }

    fn remap_through_inverse(&mut self, op: &EditOp) {
        if let Some(pos) = self.current_position.as_mut() {
            pos.sample = map_point_through_inverse(pos.sample as u64, op) as usize;
        }
        self.selection
            .remap(|start, end| map_inclusive_through_inverse(start, end, op));
    }

    fn clamp_playhead_and_selection(&mut self) {
        if self.frames() == 0 {
            self.selection.clear();
            self.current_position = None;
            return;
        }
        if let Some(sample) = self.current_position.as_ref().map(|p| p.sample) {
            let sample = self.clamp_sample(sample);
            if let Some(pos) = self.current_position.as_mut() {
                pos.sample = sample;
            }
        }
        let last = self.frames().saturating_sub(1);
        self.selection.clamp(last);
    }

    pub fn edit_undo(&mut self) -> bool {
        let from = self.composition.read().unwrap().edit_cursor();
        let ok = self.composition.write().unwrap().undo();
        if ok {
            self.after_tree_changed(from);
        }
        ok
    }

    pub fn edit_redo(&mut self) -> bool {
        let from = self.composition.read().unwrap().edit_cursor();
        let ok = self.composition.write().unwrap().redo();
        if ok {
            self.after_tree_changed(from);
        }
        ok
    }

    pub fn jump_to_edit(&mut self, id: EditId) -> bool {
        let from = self.composition.read().unwrap().edit_cursor();
        let ok = self.composition.write().unwrap().jump_to_edit(id);
        if ok {
            self.after_tree_changed(from);
        }
        ok
    }

    pub fn edit_copy(&mut self) {
        let spans = self.selection_spans();
        if spans.is_empty() {
            return;
        }
        self.composition.write().unwrap().copy_ranges(&spans);
    }

    pub fn edit_cut(&mut self) {
        let mut spans = self.selection_spans();
        if spans.is_empty() {
            return;
        }
        self.composition.write().unwrap().copy_ranges(&spans);
        spans.sort_by_key(|(start, _)| *start);
        let from = self.composition.read().unwrap().edit_cursor();
        for (start, len) in spans.into_iter().rev() {
            self.composition.write().unwrap().remove(start, len);
        }
        self.after_tree_changed(from);
    }

    pub fn edit_paste(&mut self) {
        let (at, replace) = if let Some((start, len)) = self.selection.first_span() {
            (start, len)
        } else {
            (self.caret_frame(), 0)
        };
        let from = self.composition.read().unwrap().edit_cursor();
        let _ = self
            .composition
            .write()
            .unwrap()
            .paste_replacing(at, replace);
        self.after_tree_changed(from);
    }

    pub fn edit_clear(&mut self) {
        let spans = self.selection_spans();
        if spans.is_empty() {
            return;
        }
        let from = self.composition.read().unwrap().edit_cursor();
        for (start, len) in spans {
            self.composition.write().unwrap().clear(start, len);
        }
        self.after_tree_changed(from);
    }

    pub fn edit_remove(&mut self) {
        let mut spans = self.selection_spans();
        if spans.is_empty() {
            return;
        }
        spans.sort_by_key(|(start, _)| *start);
        let from = self.composition.read().unwrap().edit_cursor();
        for (start, len) in spans.into_iter().rev() {
            self.composition.write().unwrap().remove(start, len);
        }
        self.after_tree_changed(from);
    }

    pub fn edit_duplicate(&mut self) {
        let mut spans = self.selection_spans();
        if spans.is_empty() {
            return;
        }
        spans.sort_by_key(|(start, _)| *start);
        let from = self.composition.read().unwrap().edit_cursor();
        for (start, len) in spans.into_iter().rev() {
            self.composition.write().unwrap().duplicate(start, len);
        }
        self.after_tree_changed(from);
    }

    pub fn edit_trim(&mut self) {
        let spans = self.selection_spans();
        if spans.is_empty() {
            return;
        }
        let from = self.composition.read().unwrap().edit_cursor();
        self.composition.write().unwrap().trim_ranges(&spans);
        self.after_tree_changed(from);
    }

    #[allow(dead_code)] // retained for non-script UI/tooling
    #[allow(dead_code)]
    pub(crate) fn find_region(&self, id: RegionId) -> Option<(String, crate::model::Region)> {
        if let Some(region) = self.selection.get(id) {
            return Some((SELECTION_COLLECTION.into(), region.clone()));
        }
        for collection in self.composition.read().unwrap().collections() {
            if let Some(region) = collection.get(id) {
                return Some((collection.name.clone(), region.clone()));
            }
        }
        None
    }
}

impl RegionsData for BufferDocument {
    fn fingerprint(&self) -> u64 {
        let named = self.composition.read().unwrap();
        let selection_len = self.selection.regions.len() as u64;
        let named_count: u64 = named
            .collections()
            .iter()
            .map(|col| col.regions.len() as u64)
            .sum();
        let collections = named.collections().len() as u64;
        selection_len ^ (named_count << 16) ^ (collections << 40)
    }

    fn snapshot(&self) -> Vec<RegionGroup> {
        let sample_rate = self.sample_rate();
        let mut groups = Vec::new();
        groups.push(region_group(
            SELECTION_COLLECTION,
            &self.selection.regions,
            sample_rate,
        ));
        for collection in self.composition.read().unwrap().collections() {
            groups.push(region_group(
                &collection.name,
                &collection.regions,
                sample_rate,
            ));
        }
        groups
    }
}

impl MarkersData for BufferDocument {
    fn fingerprint(&self) -> u64 {
        let caret = self.current_position.as_ref().map(|pos| pos.sample as u64);
        let composition = self.composition.read().unwrap();
        let markers = composition.markers();
        let hit = caret
            .and_then(|frame| markers.get_at(frame).map(|marker| marker.id.0))
            .unwrap_or(u64::MAX);
        markers.generation() ^ ((markers.len() as u64) << 32) ^ hit.rotate_left(8)
    }

    fn snapshot(&self) -> Vec<MarkerRow> {
        let caret = self.current_position.as_ref().map(|pos| pos.sample as u64);
        let sample_rate = self.sample_rate();
        let composition = self.composition.read().unwrap();
        composition
            .markers()
            .iter()
            .map(|marker| MarkerRow {
                id: marker.id.0,
                frame: marker.frame,
                kind: marker.marker_type.clone(),
                note: marker.note.clone().unwrap_or_default(),
                color: composition.resolved_marker_color(&marker.marker_type),
                stamp: format_stamp(marker.frame, sample_rate),
                caret_highlight: Some(marker.frame) == caret,
            })
            .collect()
    }
}

fn region_group(name: &str, regions: &[crate::model::Region], sample_rate: u32) -> RegionGroup {
    RegionGroup {
        collection: name.to_string(),
        regions: regions
            .iter()
            .map(|region| {
                let rate = f64::from(sample_rate.max(1));
                let start_s = region.start as f64 / rate;
                let end_s = region.end as f64 / rate;
                let len = region.end.saturating_sub(region.start).saturating_add(1);
                RegionRow {
                    id: region.id.0,
                    start: region.start,
                    end: region.end,
                    time_line: format!("{start_s:.2}s – {end_s:.2}s"),
                    sample_line: format!("{}–{}  ·  {len} smp", region.start, region.end),
                }
            })
            .collect(),
    }
}

fn format_stamp(frame: u64, sample_rate: u32) -> String {
    let secs = frame as f64 / f64::from(sample_rate.max(1));
    format!("{secs:.2}s · {frame} smp")
}

impl WaveformDataProvider for BufferDocument {
    fn sample_rate(&self) -> u32 {
        self.composition.read().unwrap().sample_rate()
    }

    fn channel_count(&self) -> usize {
        self.composition.read().unwrap().channel_count()
    }

    fn frames(&self) -> usize {
        self.composition.read().unwrap().frames() as usize
    }

    fn duration_secs(&self) -> f64 {
        self.composition.read().unwrap().duration_secs()
    }

    fn channel_label(&self, channel: usize) -> String {
        self.composition.read().unwrap().channel_label(channel)
    }

    fn read_channel(&self, channel: usize, start: usize, dest: &mut [f32]) {
        let _ = self
            .composition
            .read()
            .unwrap()
            .read_channel(channel, start as u64, dest);
    }

    fn min_max_in_range(&self, channel: usize, start: f64, end: f64) -> (f32, f32) {
        self.composition
            .read()
            .unwrap()
            .min_max_in_range(channel, start, end)
    }

    fn peaks_ready(&self) -> bool {
        self.composition.read().unwrap().can_paint_overview()
    }

    fn peaks_complete(&self) -> bool {
        !self.composition.read().unwrap().needs_peak_build()
    }

    fn ensure_minmax_peaks(&self) {
        if self.composition.read().unwrap().needs_peak_build() {
            self.request_analysis(AnalysisKind::MinMax);
        }
    }

    fn waveform_representation(&self) -> WaveformRepresentation {
        self.waveform_representation
    }

    fn peak_rendering(&self) -> field_ui_components::PeakRendering {
        self.peak_rendering
    }

    fn threaded_shell_value_reduce(&self) -> f32 {
        self.threaded_shell_value_reduce
    }

    fn threaded_ribbon_db(&self) -> f32 {
        self.threaded_ribbon_db
    }

    fn peaks_spectrum_split(&self) -> f32 {
        self.peaks_spectrum_split
    }

    fn envelope_overlay_enabled(&self) -> bool {
        self.show_envelope_peak
    }

    fn envelope_ready(&self) -> bool {
        self.composition.read().unwrap().envelope_peak_has_data()
    }

    fn envelope_complete(&self) -> bool {
        !self.composition.read().unwrap().needs_envelope_peak_build()
    }

    fn ensure_envelope_peak(&self) {
        if self.composition.read().unwrap().needs_envelope_peak_build() {
            self.request_analysis(AnalysisKind::EnvelopePeak);
        }
    }

    fn fill_envelope_columns(
        &self,
        channel: usize,
        start: f64,
        samples_per_pixel: f64,
        dest: &mut [f32],
    ) {
        self.composition.read().unwrap().fill_envelope_columns(
            channel,
            start,
            samples_per_pixel,
            dest,
        );
    }

    fn spectral_ready(&self) -> bool {
        self.composition.read().unwrap().spectral_has_data()
    }

    fn spectral_complete(&self) -> bool {
        !self.composition.read().unwrap().needs_spectral_build()
    }

    fn spectral_coverage_frames(&self) -> u64 {
        self.composition
            .read()
            .unwrap()
            .analysis_streams()
            .spectral()
            .map(|s| s.covered_frames)
            .unwrap_or(0)
    }

    fn ensure_spectral(&self) {
        if self.composition.read().unwrap().needs_spectral_build() {
            self.request_analysis(AnalysisKind::Spectral);
        }
    }

    fn spectral_band_count(&self) -> usize {
        crate::audio::SPECTRAL_BAND_COUNT
    }

    fn spectral_db_floor(&self) -> f32 {
        crate::audio::SPECTRAL_DB_FLOOR
    }

    fn fill_spectral_columns(
        &self,
        channel: usize,
        start: f64,
        samples_per_pixel: f64,
        dest: &mut [f32],
    ) {
        self.composition.read().unwrap().fill_spectral_columns(
            channel,
            start,
            samples_per_pixel,
            dest,
        );
    }

    fn fill_minmax_columns(
        &self,
        channel: usize,
        start: f64,
        samples_per_pixel: f64,
        dest: &mut [(f32, f32)],
    ) {
        let composition = self.composition.read().unwrap();
        composition.fill_minmax_columns(channel, start, samples_per_pixel, dest);
    }

    fn peak_block(&self) -> usize {
        crate::audio::PEAK_BLOCK
    }
}

fn lane_scope_from_channels(channels: &ChannelScope) -> LaneScope {
    match channels {
        ChannelScope::AllChannels => LaneScope::All,
        ChannelScope::Channels(list) => LaneScope::Channels(list.clone()),
    }
}

fn channels_from_lane_scope(lanes: LaneScope) -> ChannelScope {
    match lanes {
        LaneScope::All => ChannelScope::all(),
        LaneScope::Channels(list) => ChannelScope::Channels(list),
    }
}

fn paint_region_from_model(region: &super::buffer::Region) -> PaintRegion {
    PaintRegion {
        start: region.start,
        end: region.end,
        channels: lane_scope_from_channels(&region.channels),
    }
}

impl WaveformEditor for BufferDocument {
    fn selection_span(&self) -> Option<(usize, usize)> {
        self.selection.bounding_span()
    }

    fn playhead(&self) -> Option<(usize, LaneScope)> {
        self.current_position
            .as_ref()
            .map(|pos| (pos.sample, lane_scope_from_channels(&pos.channels)))
    }

    fn channel_lanes(&self, lane: usize, alt: bool) -> LaneScope {
        lane_scope_from_channels(&self.channel_scope_for_lane(lane, alt))
    }

    fn begin_replace(&mut self, anchor: usize, lanes: LaneScope, radius: usize) {
        self.begin_region_replace(anchor, channels_from_lane_scope(lanes), radius);
    }

    fn begin_extend(&mut self, sample: usize, lanes: LaneScope, radius: usize) {
        self.begin_region_extend(sample, channels_from_lane_scope(lanes), radius);
    }

    fn begin_disjoint(&mut self, anchor: usize, lanes: LaneScope, radius: usize) {
        self.begin_region_disjoint(anchor, channels_from_lane_scope(lanes), radius);
    }

    fn update_drag(&mut self, sample: usize, radius: usize) {
        self.update_region_drag(sample, radius);
    }

    fn finish_drag(&mut self) {
        self.finish_region_drag();
    }

    fn click_without_drag(&mut self, sample: usize, lanes: LaneScope, add: bool) {
        BufferDocument::click_without_drag(self, sample, channels_from_lane_scope(lanes), add);
    }

    fn selection_regions(&self) -> Vec<PaintRegion> {
        self.selection
            .regions
            .iter()
            .map(paint_region_from_model)
            .collect()
    }

    fn named_regions(&self) -> Vec<PaintRegion> {
        let composition = self.composition.read().unwrap();
        composition
            .collections()
            .iter()
            .flat_map(|collection| collection.regions.iter().map(paint_region_from_model))
            .collect()
    }

    fn markers_for_paint(&self) -> Vec<(u64, [f32; 4])> {
        let composition = self.composition.read().unwrap();
        composition
            .markers()
            .iter()
            .map(|marker| {
                (
                    marker.frame,
                    composition.resolved_marker_color(&marker.marker_type),
                )
            })
            .collect()
    }

    fn modified_ranges(&self) -> Vec<(u64, u64)> {
        self.composition.read().unwrap().modified_ranges()
    }

    fn ranges_for_edit(&self, id: u64) -> Vec<(u64, u64)> {
        self.composition.read().unwrap().ranges_for_edit(EditId(id))
    }

    fn peak_status(&self) -> Option<PeakStatus> {
        self.progress.snapshot().map(|state| PeakStatus {
            message: state.message(),
            fraction: state.fraction,
        })
    }

    fn selection_position_sample(&self) -> Option<usize> {
        BufferDocument::selection_position_sample(self)
    }

    fn set_peaks_spectrum_split(&mut self, fraction: f32) {
        self.peaks_spectrum_split = field_ui_components::clamp_peaks_spectrum_split(fraction);
    }

    fn selected_channels(&self) -> Vec<usize> {
        BufferDocument::selected_channels(self)
    }

    fn click_channel_header(&mut self, index: usize, shift: bool, disjoint: bool) {
        BufferDocument::click_channel_header(self, index, shift, disjoint);
    }

    fn has_time_selection(&self) -> bool {
        !self.selection_spans().is_empty()
    }

    fn has_channel_selection(&self) -> bool {
        BufferDocument::has_channel_selection(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::composition::{Composition, MediaRef};

    fn test_document(frames: usize) -> BufferDocument {
        let samples = vec![vec![0.0; frames], vec![0.0; frames]];
        let media = MediaRef::from_memory_samples(44100, samples);
        BufferDocument::new(Composition::from_media(media).unwrap())
    }

    #[test]
    fn full_region_spans_buffer() {
        let doc = test_document(1000);
        assert_eq!(doc.frames(), 1000);
        let region = doc.buffer.read().unwrap().full_region();
        assert_eq!(region.start, 0);
        assert!(region.channels.applies_to(0));
        assert!(region.channels.applies_to(1));
        let _ = region.end;
    }

    #[test]
    fn new_document_caret_starts_at_zero() {
        let mut doc = test_document(1000);
        assert_eq!(doc.current_position.as_ref().map(|p| p.sample), Some(0));
        doc.set_position(50, ChannelScope::all());
        doc.reset_for_new_buffer();
        assert_eq!(doc.current_position.as_ref().map(|p| p.sample), Some(0));
        assert!(doc.selection.regions.is_empty());
    }

    #[test]
    fn drag_commits_to_selection_collection() {
        let mut doc = test_document(1000);
        doc.begin_region_replace(10, ChannelScope::all(), 0);
        doc.update_region_drag(50, 0);
        doc.finish_region_drag();
        assert_eq!(doc.selection.regions.len(), 1);
        assert_eq!(doc.selection.regions[0].start, 10);
        assert_eq!(doc.selection.regions[0].end, 50);
        assert_eq!(doc.current_position.as_ref().map(|p| p.sample), Some(50));
        assert!(doc.composition.read().unwrap().collections().is_empty());
    }

    #[test]
    fn reverse_drag_normalizes_bounds_and_sets_position_to_end() {
        let mut doc = test_document(1000);
        doc.begin_region_replace(80, ChannelScope::all(), 0);
        doc.update_region_drag(10, 0);
        doc.finish_region_drag();
        assert_eq!(doc.selection.regions[0].start, 10);
        assert_eq!(doc.selection.regions[0].end, 80);
        assert_eq!(doc.current_position.as_ref().map(|p| p.sample), Some(80));
    }

    #[test]
    fn ctrl_adds_disjoint_regions_and_drag_merges_when_crossing() {
        let mut doc = test_document(1000);
        doc.select_range(10, 20, ChannelScope::all());
        doc.begin_region_disjoint(40, ChannelScope::all(), 0);
        doc.update_region_drag(50, 0);
        doc.finish_region_drag();
        assert_eq!(doc.selection.regions.len(), 2);
        doc.begin_region_extend(22, ChannelScope::all(), 0);
        doc.update_region_drag(45, 0);
        doc.finish_region_drag();
        assert_eq!(doc.selection.regions.len(), 1);
        assert_eq!(doc.selection.regions[0].start, 10);
        assert_eq!(doc.selection.regions[0].end, 50);
    }

    #[test]
    fn cut_and_clear_use_selection_spans() {
        let mut doc = test_document(100);
        doc.begin_region_replace(10, ChannelScope::all(), 0);
        doc.update_region_drag(19, 0);
        doc.finish_region_drag();
        doc.edit_cut();
        assert_eq!(doc.frames(), 90);
        doc.begin_region_replace(0, ChannelScope::all(), 0);
        doc.update_region_drag(4, 0);
        doc.finish_region_drag();
        doc.edit_clear();
        assert_eq!(doc.frames(), 90);
    }

    #[test]
    fn edit_ops_without_selection_are_noops() {
        let mut doc = test_document(50);
        doc.edit_cut();
        doc.edit_copy();
        doc.edit_clear();
        doc.edit_remove();
        doc.edit_duplicate();
        doc.edit_trim();
        assert_eq!(doc.frames(), 50);
    }

    #[test]
    fn channel_header_selection_and_clear() {
        let mut doc = test_document(50);
        doc.click_channel_header(0, false, false);
        assert_eq!(doc.selected_channels(), vec![0]);
        doc.click_channel_header(1, true, false);
        assert_eq!(doc.selected_channels(), vec![0, 1]);
        doc.click_channel_header(0, false, true);
        assert_eq!(doc.selected_channels(), vec![1]);
        doc.select_range(10, 20, ChannelScope::all());
        assert!(!doc.selection.regions.is_empty());
        doc.clear_selection();
        assert!(doc.selection.regions.is_empty());
        assert!(doc.selected_channels().is_empty());
        doc.click_channel_header(1, false, false);
        doc.click_channel_header(1, false, false);
        assert!(doc.selected_channels().is_empty());
    }

    #[test]
    fn paste_before_playhead_shifts_position() {
        let mut doc = test_document(100);
        doc.set_position(50, ChannelScope::all());
        doc.composition.write().unwrap().copy(0, 10);
        let from = doc.composition.read().unwrap().edit_cursor();
        doc.composition.write().unwrap().paste(0).unwrap();
        doc.after_tree_changed(from);
        assert_eq!(doc.frames(), 110);
        assert_eq!(doc.current_position.as_ref().map(|p| p.sample), Some(60));
        assert!(doc.edit_undo());
        assert_eq!(doc.frames(), 100);
        assert_eq!(doc.current_position.as_ref().map(|p| p.sample), Some(50));
    }

    #[test]
    fn paste_at_caret_follows_the_shifted_sample() {
        let mut doc = test_document(100);
        doc.begin_region_replace(0, ChannelScope::all(), 0);
        doc.update_region_drag(9, 0);
        doc.finish_region_drag();
        doc.edit_copy();
        doc.clear_selection();
        doc.set_position(50, ChannelScope::all());
        doc.edit_paste();
        assert_eq!(doc.frames(), 110);
        assert_eq!(doc.current_position.as_ref().map(|p| p.sample), Some(60));
    }

    #[test]
    fn paste_with_selection_replaces_from_first_span() {
        let mut doc = test_document(100);
        doc.begin_region_replace(0, ChannelScope::all(), 0);
        doc.update_region_drag(9, 0);
        doc.finish_region_drag();
        doc.edit_copy();
        doc.select_range(20, 29, ChannelScope::all());
        doc.edit_paste();
        assert_eq!(doc.frames(), 100);
    }

    #[test]
    fn invert_selection_covers_edges_and_interior() {
        let mut doc = test_document(100);
        doc.invert_selection();
        assert_eq!(doc.selection.regions.len(), 1);
        assert_eq!(doc.selection.regions[0].start, 0);
        assert_eq!(doc.selection.regions[0].end, 99);
        doc.invert_selection();
        assert!(doc.selection.is_empty());

        doc.select_range(0, 40, ChannelScope::all());
        doc.invert_selection();
        assert_eq!(doc.selection.regions.len(), 1);
        assert_eq!(doc.selection.regions[0].start, 41);
        assert_eq!(doc.selection.regions[0].end, 99);

        doc.select_range(50, 99, ChannelScope::all());
        doc.invert_selection();
        assert_eq!(doc.selection.regions.len(), 1);
        assert_eq!(doc.selection.regions[0].start, 0);
        assert_eq!(doc.selection.regions[0].end, 49);

        doc.select_range(10, 20, ChannelScope::all());
        doc.invert_selection();
        assert_eq!(doc.selection.regions.len(), 2);
        assert_eq!(
            (doc.selection.regions[0].start, doc.selection.regions[0].end),
            (0, 9)
        );
        assert_eq!(
            (doc.selection.regions[1].start, doc.selection.regions[1].end),
            (21, 99)
        );
    }

    #[test]
    fn remove_applies_right_to_left_across_regions() {
        let mut doc = test_document(100);
        doc.select_range(10, 19, ChannelScope::all());
        doc.begin_region_disjoint(50, ChannelScope::all(), 0);
        doc.update_region_drag(59, 0);
        doc.finish_region_drag();
        assert_eq!(doc.selection.regions.len(), 2);
        doc.edit_remove();
        assert_eq!(doc.frames(), 80);
    }

    #[test]
    fn marker_snap_latches_and_releases() {
        let mut doc = test_document(1000);
        assert!(doc.add_marker_of_type(100, "Blue").is_some());
        doc.snap_to_marker = true;
        assert_eq!(doc.snap_sample(&ChannelScope::all(), 102, 10), 100);
        assert_eq!(doc.latched_marker, Some(100));
        assert_eq!(doc.snap_sample(&ChannelScope::all(), 105, 10), 100);
        assert_eq!(doc.snap_sample(&ChannelScope::all(), 200, 10), 200);
        assert_eq!(doc.latched_marker, None);
    }

    fn document_with_channel(samples: Vec<f32>) -> BufferDocument {
        let media = MediaRef::from_memory_samples(44100, vec![samples]);
        BufferDocument::new(Composition::from_media(media).unwrap())
    }

    #[test]
    fn playhead_click_snaps_to_zero_crossing() {
        let mut samples = vec![1.0; 200];
        for sample in samples.iter_mut().skip(80) {
            *sample = -1.0;
        }
        let mut doc = document_with_channel(samples);
        doc.snap_zero_crossings = true;
        doc.click_without_drag(90, ChannelScope::all(), false);
        assert_eq!(doc.current_position.as_ref().map(|p| p.sample), Some(79));
    }

    #[test]
    fn marker_snap_wins_over_zero_crossing() {
        let mut samples = vec![1.0; 200];
        for sample in samples.iter_mut().skip(102) {
            *sample = -1.0;
        }
        let mut doc = document_with_channel(samples);
        assert!(doc.add_marker_of_type(100, "Blue").is_some());
        doc.snap_zero_crossings = true;
        assert_eq!(doc.snap_sample(&ChannelScope::all(), 101, 10), 101);
        doc.snap_to_marker = true;
        assert_eq!(doc.snap_sample(&ChannelScope::all(), 101, 10), 100);
    }

    #[test]
    fn snapshot_analysis_target_captures_selection_only_spans() {
        let mut doc = test_document(1000);
        doc.analyze_selection_only = true;
        assert!(!doc.snapshot_analysis_target());
        doc.select_range(100, 199, ChannelScope::all());
        assert!(doc.snapshot_analysis_target());
        let target = doc.take_pending_analysis_target();
        assert_eq!(target, Some(Some(vec![(100, 200)])));
        doc.analyze_selection_only = false;
        assert!(doc.snapshot_analysis_target());
        assert_eq!(doc.take_pending_analysis_target(), Some(None));
    }

    #[test]
    fn add_and_remove_marker_at_caret() {
        let mut doc = test_document(100);
        let id = doc.add_marker_of_type(40, "Blue").unwrap();
        assert_eq!(
            doc.composition
                .read()
                .unwrap()
                .markers()
                .get(id)
                .unwrap()
                .frame,
            40
        );
        assert!(doc.remove_marker_at(40));
        assert!(doc.composition.read().unwrap().markers().is_empty());
    }

    #[test]
    fn add_marker_is_unique_per_type_at_position() {
        let mut doc = test_document(100);
        assert!(doc.add_marker_of_type(40, "Blue").is_some());
        assert!(doc.add_marker_of_type(40, "Blue").is_none());
        assert!(doc.add_marker_of_type(40, "Yellow").is_some());
        assert_eq!(doc.composition.read().unwrap().markers().len(), 2);
        assert!(doc.remove_marker_at_type(40, "Blue"));
        assert_eq!(doc.composition.read().unwrap().markers().len(), 1);
    }

    #[test]
    fn removing_marker_type_removes_its_markers() {
        let mut doc = test_document(100);
        assert!(doc.add_marker_of_type(10, "Blue").is_some());
        assert!(doc.add_marker_of_type(20, "Yellow").is_some());
        assert!(doc.remove_marker_type("Blue"));
        let composition = doc.composition.read().unwrap();
        assert_eq!(composition.markers().len(), 1);
        assert!(composition
            .marker_types()
            .iter()
            .all(|ty| ty.name != "Blue"));
    }
}
