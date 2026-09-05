// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

use super::buffer::{ChannelScope, Region, RegionId};

pub const SELECTION_COLLECTION: &str = "selection";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionEndpoint {
    Start,
    End,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegionCollection {
    pub name: String,
    #[serde(default)]
    pub regions: Vec<Region>,
}

impl RegionCollection {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            regions: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    pub fn clear(&mut self) {
        self.regions.clear();
    }

    pub fn get(&self, id: RegionId) -> Option<&Region> {
        self.regions.iter().find(|region| region.id == id)
    }

    pub fn get_mut(&mut self, id: RegionId) -> Option<&mut Region> {
        self.regions.iter_mut().find(|region| region.id == id)
    }

    pub fn remove(&mut self, id: RegionId) -> bool {
        let before = self.regions.len();
        self.regions.retain(|region| region.id != id);
        self.regions.len() != before
    }

    pub fn contains_sample(&self, sample: usize, channel: usize) -> bool {
        self.regions
            .iter()
            .any(|region| region.contains(sample, channel))
    }

    /// Half-open edit spans `(start, len)` in time order.
    pub fn edit_spans(&self) -> Vec<(u64, u64)> {
        self.regions
            .iter()
            .filter(|region| region.end >= region.start)
            .map(|region| {
                let start = region.start as u64;
                let len = (region.end as u64).saturating_sub(start).saturating_add(1);
                (start, len)
            })
            .collect()
    }

    pub fn first_span(&self) -> Option<(u64, u64)> {
        self.edit_spans().into_iter().next()
    }

    pub fn bounding_span(&self) -> Option<(usize, usize)> {
        let mut iter = self.regions.iter();
        let first = iter.next()?;
        let mut start = first.start;
        let mut end = first.end;
        for region in iter {
            start = start.min(region.start);
            end = end.max(region.end);
        }
        Some((start, end))
    }

    pub fn push(&mut self, region: Region) {
        self.regions.push(region);
        self.normalize();
    }

    pub fn replace_with(&mut self, region: Region) {
        self.regions.clear();
        self.regions.push(region);
        self.normalize();
    }

    pub fn copy_regions_from(&mut self, other: &RegionCollection, next_id: &mut u64) {
        self.clear();
        for region in &other.regions {
            self.alloc_push(
                region.start,
                region.end,
                region.channels.clone(),
                region.label.clone(),
                next_id,
            );
        }
    }

    pub fn alloc_push(
        &mut self,
        start: usize,
        end: usize,
        channels: ChannelScope,
        label: Option<String>,
        next_id: &mut u64,
    ) -> RegionId {
        let id = RegionId(*next_id);
        *next_id += 1;
        let mut region = Region::new(id, start, end, channels.clone());
        if let Some(label) = label {
            region = region.with_label(label);
        }
        self.push(region);
        self.regions
            .iter()
            .find(|region| {
                region.channels == channels && region.start <= start && region.end >= end
            })
            .map(|region| region.id)
            .unwrap_or(id)
    }

    pub fn clamp(&mut self, last_sample: usize) {
        if self.regions.is_empty() {
            return;
        }
        for region in &mut self.regions {
            region.start = region.start.min(last_sample);
            region.end = region.end.min(last_sample);
            if region.start > region.end {
                std::mem::swap(&mut region.start, &mut region.end);
            }
        }
        self.normalize();
    }

    pub fn remap(&mut self, mut map: impl FnMut(u64, u64) -> Option<(u64, u64)>) {
        let mut kept = Vec::with_capacity(self.regions.len());
        for mut region in self.regions.drain(..) {
            if let Some((start, end)) = map(region.start as u64, region.end as u64) {
                region.start = start as usize;
                region.end = end as usize;
                kept.push(region);
            }
        }
        self.regions = kept;
        self.normalize();
    }

    /// Sort by start and merge overlapping or adjacent regions with the same
    /// channel scope.
    pub fn normalize(&mut self) {
        if self.regions.len() < 2 {
            if let Some(region) = self.regions.first_mut() {
                if region.start > region.end {
                    std::mem::swap(&mut region.start, &mut region.end);
                }
            }
            return;
        }
        for region in &mut self.regions {
            if region.start > region.end {
                std::mem::swap(&mut region.start, &mut region.end);
            }
            region.channels.normalize();
        }
        self.regions
            .sort_by_key(|region| (region.start, region.end, region.id.0));
        let mut merged: Vec<Region> = Vec::with_capacity(self.regions.len());
        for region in self.regions.drain(..) {
            if let Some(prev) = merged.last_mut() {
                if prev.channels == region.channels && region.start <= prev.end.saturating_add(1) {
                    prev.end = prev.end.max(region.end);
                    if prev.label.is_none() {
                        prev.label = region.label;
                    }
                    continue;
                }
            }
            merged.push(region);
        }
        self.regions = merged;
    }

    pub fn nearest_endpoint(
        &self,
        sample: usize,
        scope: Option<&ChannelScope>,
    ) -> Option<(RegionId, RegionEndpoint, usize)> {
        let mut best: Option<(u64, RegionId, RegionEndpoint, usize)> = None;
        for region in &self.regions {
            if let Some(scope) = scope {
                if &region.channels != scope {
                    continue;
                }
            }
            let start_dist = sample.abs_diff(region.start) as u64;
            let end_dist = sample.abs_diff(region.end) as u64;
            let (dist, endpoint, at) = if start_dist <= end_dist {
                (start_dist, RegionEndpoint::Start, region.start)
            } else {
                (end_dist, RegionEndpoint::End, region.end)
            };
            let better = match best {
                None => true,
                Some((best_dist, ..)) => dist < best_dist,
            };
            if better {
                best = Some((dist, region.id, endpoint, at));
            }
        }
        best.map(|(_, id, endpoint, at)| (id, endpoint, at))
    }

    /// Complement of coverage on `[0, last]` per channel. Empty collection
    /// becomes a full all-channel region. Full coverage becomes empty.
    pub fn invert(&mut self, last: usize, channel_count: usize, next_id: &mut u64) {
        if channel_count == 0 {
            self.clear();
            return;
        }
        if self.regions.is_empty() {
            let id = RegionId(*next_id);
            *next_id += 1;
            self.regions
                .push(Region::new(id, 0, last, ChannelScope::all()));
            return;
        }

        let mut per_channel: Vec<Vec<(usize, usize)>> = vec![Vec::new(); channel_count];
        for region in &self.regions {
            match &region.channels {
                ChannelScope::AllChannels => {
                    for ch in 0..channel_count {
                        per_channel[ch].push((region.start, region.end));
                    }
                }
                ChannelScope::Channels(channels) => {
                    for &ch in channels {
                        if ch < channel_count {
                            per_channel[ch].push((region.start, region.end));
                        }
                    }
                }
            }
        }

        let inverted: Vec<Vec<(usize, usize)>> = per_channel
            .into_iter()
            .map(|spans| invert_intervals(&merge_intervals(spans), last))
            .collect();

        let all_same = inverted.windows(2).all(|pair| pair[0] == pair[1]);
        self.regions.clear();
        if all_same {
            for &(start, end) in &inverted[0] {
                let id = RegionId(*next_id);
                *next_id += 1;
                self.regions
                    .push(Region::new(id, start, end, ChannelScope::all()));
            }
        } else {
            for (ch, spans) in inverted.iter().enumerate() {
                for &(start, end) in spans {
                    let id = RegionId(*next_id);
                    *next_id += 1;
                    self.regions
                        .push(Region::new(id, start, end, ChannelScope::single(ch)));
                }
            }
        }
        self.normalize();
    }
}

fn merge_intervals(mut spans: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    if spans.is_empty() {
        return spans;
    }
    spans.sort_unstable();
    let mut out = Vec::with_capacity(spans.len());
    let mut cur = spans[0];
    for span in spans.into_iter().skip(1) {
        if span.0 <= cur.1.saturating_add(1) {
            cur.1 = cur.1.max(span.1);
        } else {
            out.push(cur);
            cur = span;
        }
    }
    out.push(cur);
    out
}

fn invert_intervals(covered: &[(usize, usize)], last: usize) -> Vec<(usize, usize)> {
    if last == 0 && covered.is_empty() {
        return vec![(0, 0)];
    }
    let mut gaps = Vec::new();
    let mut cursor = 0usize;
    for &(start, end) in covered {
        if start > cursor {
            gaps.push((cursor, start.saturating_sub(1)));
        }
        cursor = cursor.max(end.saturating_add(1));
    }
    if cursor <= last {
        gaps.push((cursor, last));
    }
    gaps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn next() -> u64 {
        1
    }

    #[test]
    fn merge_overlapping_and_adjacent_same_scope() {
        let mut col = RegionCollection::new("selection");
        let mut id = 1;
        col.alloc_push(0, 10, ChannelScope::all(), None, &mut id);
        col.alloc_push(8, 20, ChannelScope::all(), None, &mut id);
        col.alloc_push(21, 25, ChannelScope::all(), None, &mut id);
        assert_eq!(col.regions.len(), 1);
        assert_eq!(col.regions[0].start, 0);
        assert_eq!(col.regions[0].end, 25);
    }

    #[test]
    fn does_not_merge_different_channel_scopes() {
        let mut col = RegionCollection::new("selection");
        let mut id = 1;
        col.alloc_push(0, 10, ChannelScope::all(), None, &mut id);
        col.alloc_push(5, 15, ChannelScope::single(0), None, &mut id);
        assert_eq!(col.regions.len(), 2);
    }

    #[test]
    fn invert_interior_keeps_both_gaps() {
        let mut col = RegionCollection::new("selection");
        let mut id = 1;
        col.alloc_push(10, 20, ChannelScope::all(), None, &mut id);
        col.invert(99, 2, &mut id);
        assert_eq!(col.regions.len(), 2);
        assert_eq!(col.regions[0].start, 0);
        assert_eq!(col.regions[0].end, 9);
        assert_eq!(col.regions[1].start, 21);
        assert_eq!(col.regions[1].end, 99);
        assert!(col
            .regions
            .iter()
            .all(|r| r.channels == ChannelScope::all()));
    }

    #[test]
    fn invert_empty_selects_all_and_full_clears() {
        let mut col = RegionCollection::new("selection");
        let mut id = 1;
        col.invert(99, 2, &mut id);
        assert_eq!(col.regions.len(), 1);
        assert_eq!((col.regions[0].start, col.regions[0].end), (0, 99));
        col.invert(99, 2, &mut id);
        assert!(col.is_empty());
    }

    #[test]
    fn invert_channel_scoped_is_independent() {
        let mut col = RegionCollection::new("selection");
        let mut id = 1;
        col.alloc_push(0, 49, ChannelScope::single(0), None, &mut id);
        col.invert(99, 2, &mut id);
        let ch0: Vec<_> = col
            .regions
            .iter()
            .filter(|r| r.channels.applies_to(0) && !r.channels.applies_to(1))
            .map(|r| (r.start, r.end))
            .collect();
        let ch1: Vec<_> = col
            .regions
            .iter()
            .filter(|r| r.channels.applies_to(1) && !r.channels.applies_to(0))
            .map(|r| (r.start, r.end))
            .collect();
        assert_eq!(ch0, vec![(50, 99)]);
        assert_eq!(ch1, vec![(0, 99)]);
    }

    #[test]
    fn nearest_endpoint_picks_closest() {
        let mut col = RegionCollection::new("selection");
        let mut id = 1;
        let a = col.alloc_push(10, 20, ChannelScope::all(), None, &mut id);
        col.alloc_push(80, 90, ChannelScope::all(), None, &mut id);
        let (found, endpoint, at) = col.nearest_endpoint(12, None).unwrap();
        assert_eq!(found, a);
        assert_eq!(endpoint, RegionEndpoint::Start);
        assert_eq!(at, 10);
        let _ = next();
    }
}
