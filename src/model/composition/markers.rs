// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use super::edit_ranges::{map_point_if_kept, map_point_if_kept_inverse};
use super::edl::EditOp;

pub const MARKER_TYPE_BLUE: &str = "Blue";
pub const MARKER_TYPE_YELLOW: &str = "Yellow";
pub const MARKER_TYPE_PURPLE: &str = "Purple";

/// Built-in marker types and their default RGBA colors.
pub const DEFAULT_MARKER_TYPES: &[(&str, [f32; 4])] = &[
    (
        MARKER_TYPE_BLUE,
        [
            0x3b as f32 / 255.0,
            0x82 as f32 / 255.0,
            0xf6 as f32 / 255.0,
            1.0,
        ],
    ),
    (
        MARKER_TYPE_YELLOW,
        [
            0xea as f32 / 255.0,
            0xb3 as f32 / 255.0,
            0x08 as f32 / 255.0,
            1.0,
        ],
    ),
    (
        MARKER_TYPE_PURPLE,
        [
            0xa8 as f32 / 255.0,
            0x55 as f32 / 255.0,
            0xf7 as f32 / 255.0,
            1.0,
        ],
    ),
];

pub fn marker_type_color(name: &str) -> Option<[f32; 4]> {
    DEFAULT_MARKER_TYPES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, color)| *color)
}

pub fn default_marker_type() -> &'static str {
    MARKER_TYPE_BLUE
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarkerType {
    pub name: String,
    pub color: [f32; 4],
}

impl MarkerType {
    pub fn defaults() -> Vec<Self> {
        DEFAULT_MARKER_TYPES
            .iter()
            .map(|(name, color)| Self {
                name: (*name).to_string(),
                color: *color,
            })
            .collect()
    }

    pub fn color_of(types: &[Self], name: &str) -> Option<[f32; 4]> {
        types
            .iter()
            .find(|ty| ty.name == name)
            .map(|ty| ty.color)
            .or_else(|| marker_type_color(name))
    }

    pub fn resolved_color(types: &[Self], name: &str) -> [f32; 4] {
        Self::color_of(types, name)
            .or_else(|| marker_type_color(default_marker_type()))
            .unwrap_or([0.5, 0.5, 0.5, 1.0])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MarkerId(pub u64);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub id: MarkerId,
    pub frame: u64,
    #[serde(rename = "type")]
    pub marker_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Marker {
    pub fn new(
        id: MarkerId,
        frame: u64,
        marker_type: impl Into<String>,
        note: Option<String>,
    ) -> Self {
        Self {
            id,
            frame,
            marker_type: marker_type.into(),
            note,
        }
    }
}

/// On-disk marker instance. Older files stored `color` on each instance;
/// load folds that into [`MarkerType`]. New files omit it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMarker {
    pub id: MarkerId,
    pub frame: u64,
    #[serde(rename = "type")]
    pub marker_type: String,
    #[serde(default, skip_serializing)]
    pub color: Option<[f32; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl From<&Marker> for StoredMarker {
    fn from(marker: &Marker) -> Self {
        Self {
            id: marker.id,
            frame: marker.frame,
            marker_type: marker.marker_type.clone(),
            color: None,
            note: marker.note.clone(),
        }
    }
}

impl From<StoredMarker> for Marker {
    fn from(stored: StoredMarker) -> Self {
        Self {
            id: stored.id,
            frame: stored.frame,
            marker_type: stored.marker_type,
            note: stored.note,
        }
    }
}

/// Time-ordered marker store. Insert and delete are O(log N).
#[derive(Debug, Clone, Default)]
pub struct MarkerList {
    by_key: BTreeMap<(u64, String), MarkerId>,
    by_id: HashMap<MarkerId, Marker>,
    next_id: u64,
    generation: u64,
}

impl MarkerList {
    pub fn new() -> Self {
        Self {
            by_key: BTreeMap::new(),
            by_id: HashMap::new(),
            next_id: 1,
            generation: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn bump(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    pub fn get(&self, id: MarkerId) -> Option<&Marker> {
        self.by_id.get(&id)
    }

    pub fn get_at(&self, frame: u64) -> Option<&Marker> {
        self.at_frame(frame).next()
    }

    pub fn get_at_type(&self, frame: u64, marker_type: &str) -> Option<&Marker> {
        let id = *self.by_key.get(&(frame, marker_type.to_string()))?;
        self.by_id.get(&id)
    }

    pub fn at_frame(&self, frame: u64) -> impl Iterator<Item = &Marker> {
        self.by_key
            .range((frame, String::new())..)
            .take_while(move |((at, _), _)| *at == frame)
            .filter_map(|(_, id)| self.by_id.get(id))
    }

    /// Insert a marker at `frame`. Returns `None` if that type already occupies
    /// the frame.
    pub fn insert(
        &mut self,
        frame: u64,
        marker_type: impl Into<String>,
        note: Option<String>,
    ) -> Option<MarkerId> {
        let marker_type = marker_type.into();
        if self.by_key.contains_key(&(frame, marker_type.clone())) {
            return None;
        }
        let id = MarkerId(self.next_id);
        self.next_id += 1;
        self.bump();
        self.by_key.insert((frame, marker_type.clone()), id);
        self.by_id
            .insert(id, Marker::new(id, frame, marker_type, note));
        Some(id)
    }

    pub fn remove(&mut self, id: MarkerId) -> bool {
        let Some(marker) = self.by_id.remove(&id) else {
            return false;
        };
        self.by_key.remove(&(marker.frame, marker.marker_type));
        self.bump();
        true
    }

    pub fn remove_at_type(&mut self, frame: u64, marker_type: &str) -> bool {
        let Some(id) = self.by_key.remove(&(frame, marker_type.to_string())) else {
            return false;
        };
        self.by_id.remove(&id);
        self.bump();
        true
    }

    pub fn remove_type(&mut self, marker_type: &str) -> usize {
        let ids: Vec<MarkerId> = self
            .iter()
            .filter(|marker| marker.marker_type == marker_type)
            .map(|marker| marker.id)
            .collect();
        let count = ids.len();
        for id in ids {
            self.remove(id);
        }
        count
    }

    pub fn remove_at(&mut self, frame: u64) -> bool {
        let ids: Vec<MarkerId> = self.at_frame(frame).map(|marker| marker.id).collect();
        if ids.is_empty() {
            return false;
        }
        for id in ids {
            self.remove(id);
        }
        true
    }

    pub fn iter(&self) -> impl Iterator<Item = &Marker> {
        self.by_key.values().filter_map(|id| self.by_id.get(id))
    }

    pub fn to_vec(&self) -> Vec<Marker> {
        self.iter().cloned().collect()
    }

    pub fn from_vec(markers: Vec<Marker>) -> Self {
        let mut list = Self::new();
        for marker in markers {
            list.next_id = list.next_id.max(marker.id.0.saturating_add(1));
            let key = (marker.frame, marker.marker_type.clone());
            if list.by_key.contains_key(&key) {
                continue;
            }
            list.by_key.insert(key, marker.id);
            list.by_id.insert(marker.id, marker);
            list.bump();
        }
        if list.next_id == 0 {
            list.next_id = 1;
        }
        list
    }

    pub fn remap_through_op(&mut self, op: &EditOp) {
        self.remap(|frame| map_point_if_kept(frame, op));
    }

    pub fn remap_through_inverse(&mut self, op: &EditOp) {
        self.remap(|frame| map_point_if_kept_inverse(frame, op));
    }

    fn remap(&mut self, mut map: impl FnMut(u64) -> Option<u64>) {
        let old = std::mem::take(&mut self.by_id);
        self.by_key.clear();
        let mut entries: Vec<Marker> = old.into_values().collect();
        entries.sort_by_key(|marker| (marker.frame, marker.marker_type.clone(), marker.id.0));
        for mut marker in entries {
            let Some(frame) = map(marker.frame) else {
                continue;
            };
            let key = (frame, marker.marker_type.clone());
            if self.by_key.contains_key(&key) {
                continue;
            }
            marker.frame = frame;
            self.by_key.insert(key, marker.id);
            self.by_id.insert(marker.id, marker);
        }
        self.bump();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_delete_and_lookup_are_ordered() {
        let mut list = MarkerList::new();
        let a = list.insert(100, MARKER_TYPE_BLUE, None).unwrap();
        let b = list
            .insert(20, MARKER_TYPE_YELLOW, Some("note".into()))
            .unwrap();
        assert!(list.insert(100, MARKER_TYPE_BLUE, None).is_none());
        let c = list.insert(100, MARKER_TYPE_PURPLE, None).unwrap();
        let frames: Vec<u64> = list.iter().map(|m| m.frame).collect();
        assert_eq!(frames, vec![20, 100, 100]);
        assert_eq!(list.get_at(20).map(|m| m.id), Some(b));
        assert_eq!(list.get(a).map(|m| m.frame), Some(100));
        assert_eq!(
            list.get_at_type(100, MARKER_TYPE_PURPLE).map(|m| m.id),
            Some(c)
        );
        assert!(list.remove(a));
        assert_eq!(list.at_frame(100).count(), 1);
        assert!(list.remove_at(20));
        assert!(list.remove_at_type(100, MARKER_TYPE_PURPLE));
        assert!(list.is_empty());
    }

    #[test]
    fn from_vec_skips_duplicate_type_at_frame() {
        let list = MarkerList::from_vec(vec![
            Marker::new(MarkerId(4), 10, MARKER_TYPE_BLUE, None),
            Marker::new(MarkerId(5), 10, MARKER_TYPE_BLUE, None),
            Marker::new(MarkerId(6), 10, MARKER_TYPE_PURPLE, None),
        ]);
        assert_eq!(list.len(), 2);
        assert_eq!(
            list.get_at_type(10, MARKER_TYPE_BLUE).unwrap().id,
            MarkerId(4)
        );
        assert_eq!(
            list.get_at_type(10, MARKER_TYPE_PURPLE).unwrap().id,
            MarkerId(6)
        );
        let id = {
            let mut list = list;
            list.insert(11, MARKER_TYPE_BLUE, None).unwrap()
        };
        assert_eq!(id, MarkerId(7));
    }

    #[test]
    fn remap_cut_shifts_and_drops() {
        let mut list = MarkerList::new();
        list.insert(5, MARKER_TYPE_BLUE, None).unwrap();
        list.insert(15, MARKER_TYPE_BLUE, None).unwrap();
        list.insert(25, MARKER_TYPE_BLUE, None).unwrap();
        list.remap_through_op(&EditOp::Cut { start: 10, len: 10 });
        let frames: Vec<u64> = list.iter().map(|m| m.frame).collect();
        assert_eq!(frames, vec![5, 15]);
        list.remap_through_inverse(&EditOp::Cut { start: 10, len: 10 });
        let frames: Vec<u64> = list.iter().map(|m| m.frame).collect();
        assert_eq!(frames, vec![5, 25]);
    }

    #[test]
    fn stored_marker_reads_legacy_color_and_omits_it_on_save() {
        let stored: StoredMarker = serde_json::from_str(
            r#"{"id":1,"frame":10,"type":"Red","color":[1.0,0.0,0.0,1.0],"note":"hit"}"#,
        )
        .unwrap();
        assert_eq!(stored.color, Some([1.0, 0.0, 0.0, 1.0]));
        let marker = Marker::from(stored.clone());
        assert_eq!(marker.marker_type, "Red");
        let json = serde_json::to_value(StoredMarker::from(&marker)).unwrap();
        assert!(json.get("color").is_none());
        assert_eq!(json["type"], "Red");
        assert_eq!(json["frame"], 10);
    }
}
