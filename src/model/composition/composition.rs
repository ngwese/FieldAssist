// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use anyhow::{bail, Context, Result};

use super::clip::{Clip, ClipId, ClipSpan};
use super::edit_ranges::{map_inclusive_through_inverse, map_inclusive_through_op};
use super::edl::{EditId, EditOp, Edl, InitialState, ProjectEnvelope, ProjectFile};
use super::markers::{Marker, MarkerId, MarkerList, MarkerType, StoredMarker};
use super::media::{MediaId, MediaPool, MediaRef};
use super::pager::BlockPager;
use super::tree::ClipTree;
use crate::audio::{ProbedFile, PEAK_BLOCK};
use crate::model::regions::{RegionCollection, SELECTION_COLLECTION};
use crate::progress::ProgressHandle;

#[derive(Debug, Clone, Default)]
pub struct Clipboard {
    pub sample_rate: u32,
    pub channel_count: usize,
    pub clips: Vec<Clip>,
}

impl Clipboard {
    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    pub fn frames(&self) -> u64 {
        self.clips.iter().map(|clip| clip.len).sum()
    }
}

pub struct Composition {
    sample_rate: u32,
    channel_count: usize,
    tree: ClipTree,
    pool: MediaPool,
    pager: Mutex<BlockPager>,
    edl: Edl,
    next_clip_id: u64,
    clipboard: Clipboard,
    initial: InitialState,
    markers: MarkerList,
    marker_types: Vec<MarkerType>,
    collections: Vec<RegionCollection>,
    next_region_id: u64,
    clean_edit_id: EditId,
    clean_markers: Vec<Marker>,
    clean_marker_types: Vec<MarkerType>,
    clean_collections: Vec<RegionCollection>,
}

impl Composition {
    pub fn new(sample_rate: u32, channel_count: usize) -> Self {
        let tree = ClipTree::empty();
        let mut composition = Self {
            sample_rate,
            channel_count,
            edl: Edl::new(tree.clone()),
            tree,
            pool: MediaPool::new(),
            pager: Mutex::new(BlockPager::in_memory()),
            next_clip_id: 1,
            clipboard: Clipboard {
                sample_rate,
                channel_count,
                clips: Vec::new(),
            },
            initial: InitialState::Empty,
            markers: MarkerList::new(),
            marker_types: MarkerType::defaults(),
            collections: Vec::new(),
            next_region_id: 1,
            clean_edit_id: EditId(0),
            clean_markers: Vec::new(),
            clean_marker_types: Vec::new(),
            clean_collections: Vec::new(),
        };
        composition.mark_clean();
        composition
    }

    pub fn from_media(media: MediaRef) -> Result<Self> {
        if media.sample_rate == 0 {
            bail!("media has no sample rate");
        }
        if media.channel_count == 0 {
            bail!("media has no channels");
        }
        let mut pool = MediaPool::new();
        let sample_rate = media.sample_rate;
        let channel_count = media.channel_count;
        let frame_count = media.frame_count;
        let media_id = pool.insert(media);
        let mut next_clip_id = 1;
        let clip = Clip::from_media(ClipId(next_clip_id), media_id, 0, frame_count);
        next_clip_id += 1;
        let tree = ClipTree::from_clip(clip);
        let mut composed = Self {
            sample_rate,
            channel_count,
            edl: Edl::new(tree.clone()),
            tree,
            pool,
            pager: Mutex::new(BlockPager::in_memory()),
            next_clip_id,
            clipboard: Clipboard {
                sample_rate,
                channel_count,
                clips: Vec::new(),
            },
            initial: InitialState::FromMedia {
                media_id: media_id.0,
            },
            markers: MarkerList::new(),
            marker_types: MarkerType::defaults(),
            collections: Vec::new(),
            next_region_id: 1,
            clean_edit_id: EditId(0),
            clean_markers: Vec::new(),
            clean_marker_types: Vec::new(),
            clean_collections: Vec::new(),
        };
        let peaked = composed
            .pool
            .first()
            .is_some_and(|media| media.samples.is_some());
        if peaked {
            composed.ensure_clip_peaks().ok();
            composed.edl = Edl::new(composed.tree.clone());
        }
        composed.mark_clean();
        Ok(composed)
    }

    pub fn load_from_path(path: &Path) -> Result<Self> {
        Ok(Self::load_from_path_with_warnings(path)?.0)
    }

    pub fn load_from_path_with_warnings(path: &Path) -> Result<(Self, Vec<String>)> {
        Self::load_from_path_with_progress(path, None, 0)
    }

    pub fn load_from_path_with_progress(
        path: &Path,
        progress: Option<&ProgressHandle>,
        epoch: u64,
    ) -> Result<(Self, Vec<String>)> {
        if is_facomp_path(path) {
            return Self::load_facomp_with_progress(path, progress, epoch);
        }
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        if let Some(progress) = progress {
            progress.set_fraction(epoch, 0.2);
        }
        let probed = crate::audio::probe_file(path)?;
        if let Some(progress) = progress {
            progress.set_fraction(epoch, 0.8);
        }
        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        std::time::SystemTime::now().hash(&mut hasher);
        let spill = std::env::temp_dir()
            .join("snd-review")
            .join("blocks")
            .join(format!("{:x}", hasher.finish()));
        Ok((
            Self::from_media(media_ref_from_probed(probed))?.with_spill_dir(spill)?,
            Vec::new(),
        ))
    }

    pub fn load_facomp(path: &Path) -> Result<(Self, Vec<String>)> {
        Self::load_facomp_with_progress(path, None, 0)
    }

    fn load_facomp_with_progress(
        path: &Path,
        progress: Option<&ProgressHandle>,
        epoch: u64,
    ) -> Result<(Self, Vec<String>)> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        if let Some(progress) = progress {
            progress.set_fraction(epoch, 0.05);
        }
        let json = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if let Some(progress) = progress {
            progress.set_fraction(epoch, 0.2);
        }
        let (mut composition, warnings) = Self::from_json_reprobing(&json)?;
        if let Some(progress) = progress {
            progress.set_fraction(epoch, 0.85);
        }
        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        std::time::SystemTime::now().hash(&mut hasher);
        let spill = std::env::temp_dir()
            .join("snd-review")
            .join("blocks")
            .join(format!("{:x}", hasher.finish()));
        composition = composition.with_spill_dir(spill)?;
        if let Some(progress) = progress {
            progress.set_fraction(epoch, 1.0);
        }
        Ok((composition, warnings))
    }

    pub fn suggested_facomp_name(&self) -> String {
        format!("{}.facomp", self.display_name())
    }

    pub fn save_to_path(&mut self, path: &Path) -> Result<()> {
        write_atomic(path, &self.to_json()?)?;
        self.mark_clean();
        Ok(())
    }

    pub fn is_modified(&self) -> bool {
        self.edl.current_id() != self.clean_edit_id
            || self.markers.to_vec() != self.clean_markers
            || self.marker_types != self.clean_marker_types
            || self.collections != self.clean_collections
    }

    fn mark_clean(&mut self) {
        self.clean_edit_id = self.edl.current_id();
        self.clean_markers = self.markers.to_vec();
        self.clean_marker_types = self.marker_types.clone();
        self.clean_collections = self.collections.clone();
    }

    pub fn with_spill_dir(mut self, dir: impl AsRef<Path>) -> Result<Self> {
        self.pager = Mutex::new(BlockPager::new(dir.as_ref().to_path_buf())?);
        Ok(self)
    }

    pub fn display_name(&self) -> String {
        self.pool()
            .first()
            .and_then(|media| media.path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| crate::APP_NAME.into())
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channel_count(&self) -> usize {
        self.channel_count
    }

    pub fn frames(&self) -> u64 {
        self.tree.frames()
    }

    pub fn duration_secs(&self) -> f64 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.frames() as f64 / f64::from(self.sample_rate)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tree.frames() == 0
    }

    pub fn markers(&self) -> &MarkerList {
        &self.markers
    }

    pub fn add_marker(
        &mut self,
        frame: u64,
        marker_type: impl Into<String>,
        note: Option<String>,
    ) -> Option<MarkerId> {
        self.markers.insert(frame, marker_type, note)
    }

    pub fn remove_marker(&mut self, id: MarkerId) -> bool {
        self.markers.remove(id)
    }

    pub fn remove_marker_at(&mut self, frame: u64) -> bool {
        self.markers.remove_at(frame)
    }

    pub fn remove_marker_at_type(&mut self, frame: u64, marker_type: &str) -> bool {
        self.markers.remove_at_type(frame, marker_type)
    }

    pub fn marker_types(&self) -> &[MarkerType] {
        &self.marker_types
    }

    pub fn marker_type_color(&self, name: &str) -> Option<[f32; 4]> {
        MarkerType::color_of(&self.marker_types, name)
    }

    pub fn resolved_marker_color(&self, name: &str) -> [f32; 4] {
        MarkerType::resolved_color(&self.marker_types, name)
    }

    pub fn add_marker_type(&mut self, name: impl Into<String>, color: [f32; 4]) -> bool {
        let name = name.into();
        if name.is_empty() {
            return false;
        }
        if self.marker_types.iter().any(|ty| ty.name == name) {
            return false;
        }
        self.marker_types.push(MarkerType { name, color });
        true
    }

    pub fn remove_marker_type(&mut self, name: &str) -> bool {
        let Some(index) = self.marker_types.iter().position(|ty| ty.name == name) else {
            return false;
        };
        self.marker_types.remove(index);
        self.markers.remove_type(name);
        true
    }

    pub fn collections(&self) -> &[RegionCollection] {
        &self.collections
    }

    pub fn collection(&self, name: &str) -> Option<&RegionCollection> {
        self.collections.iter().find(|col| col.name == name)
    }

    pub fn collection_mut(&mut self, name: &str) -> Option<&mut RegionCollection> {
        self.collections.iter_mut().find(|col| col.name == name)
    }

    pub fn ensure_collection(&mut self, name: &str) -> Option<&mut RegionCollection> {
        if name.is_empty() || name == SELECTION_COLLECTION {
            return None;
        }
        if !self.collections.iter().any(|col| col.name == name) {
            self.collections.push(RegionCollection::new(name));
        }
        self.collection_mut(name)
    }

    pub fn add_named_region(
        &mut self,
        name: &str,
        start: usize,
        end: usize,
        channels: crate::model::buffer::ChannelScope,
        label: Option<String>,
    ) -> Option<crate::model::buffer::RegionId> {
        if name.is_empty() || name == SELECTION_COLLECTION {
            return None;
        }
        let index = match self.collections.iter().position(|col| col.name == name) {
            Some(index) => index,
            None => {
                self.collections.push(RegionCollection::new(name));
                self.collections.len() - 1
            }
        };
        Some(self.collections[index].alloc_push(
            start,
            end,
            channels,
            label,
            &mut self.next_region_id,
        ))
    }

    pub fn alloc_region_id(&mut self) -> u64 {
        let id = self.next_region_id;
        self.next_region_id += 1;
        id
    }

    pub fn peek_next_region_id(&self) -> u64 {
        self.next_region_id.max(1)
    }

    pub fn set_next_region_id(&mut self, next: u64) {
        self.next_region_id = self.next_region_id.max(next).max(1);
    }

    fn bump_next_region_id_from_collections(&mut self) {
        let max_id = self
            .collections
            .iter()
            .flat_map(|col| col.regions.iter())
            .map(|region| region.id.0)
            .max()
            .unwrap_or(0);
        self.next_region_id = self.next_region_id.max(max_id.saturating_add(1)).max(1);
    }

    fn remap_collections_op(&mut self, op: &EditOp) {
        for collection in &mut self.collections {
            collection.remap(|start, end| map_inclusive_through_op(start, end, op));
        }
    }

    fn remap_collections_inverse(&mut self, op: &EditOp) {
        for collection in &mut self.collections {
            collection.remap(|start, end| map_inclusive_through_inverse(start, end, op));
        }
    }

    pub fn pool(&self) -> &MediaPool {
        &self.pool
    }

    pub fn clipboard(&self) -> &Clipboard {
        &self.clipboard
    }

    pub fn current_edit(&self) -> EditId {
        self.edl.current_id()
    }

    pub fn edit_cursor(&self) -> usize {
        self.edl.cursor()
    }

    pub fn can_undo(&self) -> bool {
        self.edl.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.edl.can_redo()
    }

    pub fn edits(&self) -> &[super::edl::Edit] {
        self.edl.edits()
    }

    pub fn spans(&self) -> Vec<ClipSpan> {
        self.tree.spans()
    }

    pub fn clip_at(&self, frame: u64) -> Option<ClipSpan> {
        self.tree.at(frame)
    }

    fn alloc_clip_id(&mut self) -> ClipId {
        let id = ClipId(self.next_clip_id);
        self.next_clip_id += 1;
        id
    }

    fn remap_clips(&mut self, clips: &[Clip]) -> Vec<Clip> {
        clips
            .iter()
            .map(|clip| {
                let mut clip = clip.clone();
                clip.id = self.alloc_clip_id();
                clip
            })
            .collect()
    }

    fn commit(&mut self, op: EditOp, tree: ClipTree) {
        self.markers.remap_through_op(&op);
        self.remap_collections_op(&op);
        self.tree = tree;
        self.edl.push(op, self.tree.clone());
    }

    pub fn undo(&mut self) -> bool {
        if !self.edl.can_undo() {
            return false;
        }
        let op = self.edl.current().op.clone();
        if let Some(tree) = self.edl.undo() {
            self.markers.remap_through_inverse(&op);
            self.remap_collections_inverse(&op);
            self.adopt_tree(tree);
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        if !self.edl.can_redo() {
            return false;
        }
        let op = self.edl.edits()[self.edl.cursor() + 1].op.clone();
        if let Some(tree) = self.edl.redo() {
            self.markers.remap_through_op(&op);
            self.remap_collections_op(&op);
            self.adopt_tree(tree);
            true
        } else {
            false
        }
    }

    pub fn jump_to_edit(&mut self, id: EditId) -> bool {
        let from = self.edl.cursor();
        if let Some(tree) = self.edl.jump_to(id) {
            let to = self.edl.cursor();
            self.remap_markers_between(from, to);
            self.remap_collections_between(from, to);
            self.adopt_tree(tree);
            true
        } else {
            false
        }
    }

    fn remap_markers_between(&mut self, from: usize, to: usize) {
        let ops: Vec<EditOp> = self
            .edl
            .edits()
            .iter()
            .map(|edit| edit.op.clone())
            .collect();
        if to > from {
            for op in &ops[from + 1..=to] {
                self.markers.remap_through_op(op);
            }
        } else if to < from {
            for op in ops[to + 1..=from].iter().rev() {
                self.markers.remap_through_inverse(op);
            }
        }
    }

    fn remap_collections_between(&mut self, from: usize, to: usize) {
        let ops: Vec<EditOp> = self
            .edl
            .edits()
            .iter()
            .map(|edit| edit.op.clone())
            .collect();
        if to > from {
            for op in &ops[from + 1..=to] {
                self.remap_collections_op(op);
            }
        } else if to < from {
            for op in ops[to + 1..=from].iter().rev() {
                self.remap_collections_inverse(op);
            }
        }
    }

    fn adopt_tree(&mut self, tree: ClipTree) {
        let tree = self.tree_with_initial_media(tree);
        self.tree = tree;
    }

    fn rebuild_from_initial_media(&self) -> Option<ClipTree> {
        let InitialState::FromMedia { media_id } = self.initial else {
            return None;
        };
        let media = self.pool.get(MediaId(media_id))?;
        if media.frame_count == 0 {
            return None;
        }
        Some(ClipTree::from_clip(Clip::from_media(
            ClipId(1),
            MediaId(media_id),
            0,
            media.frame_count,
        )))
    }

    fn tree_with_initial_media(&mut self, tree: ClipTree) -> ClipTree {
        if !tree.is_empty() || self.edl.cursor() != 0 {
            return tree;
        }
        let Some(rebuilt) = self.rebuild_from_initial_media() else {
            return tree;
        };
        self.edl.replace_init_snapshot(rebuilt.clone());
        rebuilt
    }

    /// Regions changed by applied edits, in the current timeline.
    ///
    /// Adjacent landings from different edits stay separate so the waveform
    /// can draw a gap between them.
    pub fn modified_ranges(&self) -> Vec<(u64, u64)> {
        super::edit_ranges::modified_ranges(self.edl.edits(), self.edl.cursor())
    }

    /// Where `id` landed on the current timeline, if that edit is applied.
    pub fn ranges_for_edit(&self, id: EditId) -> Vec<(u64, u64)> {
        let edits = self.edl.edits();
        let Some(index) = edits.iter().position(|edit| edit.id == id) else {
            return Vec::new();
        };
        super::edit_ranges::ranges_for_edit(edits, self.edl.cursor(), index)
    }

    fn fill_clipboard(&mut self, start: u64, len: u64) {
        let mut n = self.next_clip_id;
        let clips = self.tree.clips_in_range(start, len, &mut || {
            n += 1;
            ClipId(n)
        });
        self.next_clip_id = n;
        self.clipboard = Clipboard {
            sample_rate: self.sample_rate,
            channel_count: self.channel_count,
            clips,
        };
    }

    pub fn copy(&mut self, start: u64, len: u64) {
        self.copy_ranges(&[(start, len)]);
    }

    pub fn copy_ranges(&mut self, ranges: &[(u64, u64)]) {
        if ranges.is_empty() {
            return;
        }
        let mut n = self.next_clip_id;
        let mut clips = Vec::new();
        for &(start, len) in ranges {
            if len == 0 {
                continue;
            }
            clips.extend(self.tree.clips_in_range(start, len, &mut || {
                n += 1;
                ClipId(n)
            }));
        }
        self.next_clip_id = n;
        self.clipboard = Clipboard {
            sample_rate: self.sample_rate,
            channel_count: self.channel_count,
            clips,
        };
        let (start, len) = ranges[0];
        self.edl
            .push(EditOp::Copy { start, len }, self.tree.clone());
    }

    pub fn trim_ranges(&mut self, ranges: &[(u64, u64)]) {
        if ranges.is_empty() {
            return;
        }
        let mut n = self.next_clip_id;
        let mut clips = Vec::new();
        for &(start, len) in ranges {
            if len == 0 {
                continue;
            }
            clips.extend(self.tree.clips_in_range(start, len, &mut || {
                n += 1;
                ClipId(n)
            }));
        }
        let kept_len: u64 = clips.iter().map(|clip| clip.len).sum();
        let kept = ClipTree::from_clips(clips);
        let tree = self
            .tree
            .replace_range(0, self.tree.frames(), kept, &mut || {
                n += 1;
                ClipId(n)
            });
        self.next_clip_id = n;
        let (start, _) = ranges[0];
        self.commit(
            EditOp::Trim {
                start,
                len: kept_len,
            },
            tree,
        );
    }

    pub fn remove(&mut self, start: u64, len: u64) {
        let mut n = self.next_clip_id;
        let tree = self
            .tree
            .replace_range(start, len, ClipTree::empty(), &mut || {
                n += 1;
                ClipId(n)
            });
        self.next_clip_id = n;
        self.commit(EditOp::Remove { start, len }, tree);
    }

    pub fn cut(&mut self, start: u64, len: u64) {
        self.fill_clipboard(start, len);
        let mut n = self.next_clip_id;
        let tree = self
            .tree
            .replace_range(start, len, ClipTree::empty(), &mut || {
                n += 1;
                ClipId(n)
            });
        self.next_clip_id = n;
        self.commit(EditOp::Cut { start, len }, tree);
    }

    pub fn delete(&mut self, start: u64, len: u64) {
        if len == 0 {
            return;
        }
        let silence = Clip::silence(self.alloc_clip_id(), len);
        let mut n = self.next_clip_id;
        let tree = self
            .tree
            .replace_range(start, len, ClipTree::from_clip(silence), &mut || {
                n += 1;
                ClipId(n)
            });
        self.next_clip_id = n;
        self.commit(EditOp::Delete { start, len }, tree);
    }

    pub fn paste(&mut self, at: u64) -> Result<()> {
        self.paste_replacing(at, 0)
    }

    pub fn paste_replacing(&mut self, at: u64, replace_len: u64) -> Result<()> {
        if self.clipboard.is_empty() {
            return Ok(());
        }
        if self.clipboard.sample_rate != self.sample_rate
            || self.clipboard.channel_count != self.channel_count
        {
            bail!(
                "clipboard is {} Hz {} ch, composition is {} Hz {} ch",
                self.clipboard.sample_rate,
                self.clipboard.channel_count,
                self.sample_rate,
                self.channel_count
            );
        }
        let clipboard_clips = self.clipboard.clips.clone();
        let clips = self.remap_clips(&clipboard_clips);
        let pasted_len: u64 = clips.iter().map(|c| c.len).sum();
        let mut n = self.next_clip_id;
        let tree =
            self.tree
                .replace_range(at, replace_len, ClipTree::from_clips(clips), &mut || {
                    n += 1;
                    ClipId(n)
                });
        self.next_clip_id = n;
        self.commit(
            EditOp::Paste {
                at,
                len: pasted_len,
            },
            tree,
        );
        Ok(())
    }

    pub fn trim(&mut self, start: u64, len: u64) {
        let mut n = self.next_clip_id;
        let kept = ClipTree::from_clips(self.tree.clips_in_range(start, len, &mut || {
            n += 1;
            ClipId(n)
        }));
        let tree = self
            .tree
            .replace_range(0, self.tree.frames(), kept, &mut || {
                n += 1;
                ClipId(n)
            });
        self.next_clip_id = n;
        self.commit(EditOp::Trim { start, len }, tree);
    }

    pub fn duplicate(&mut self, start: u64, len: u64) {
        let mut n = self.next_clip_id;
        let clips = self.tree.clips_in_range(start, len, &mut || {
            n += 1;
            ClipId(n)
        });
        let copies: Vec<_> = clips
            .into_iter()
            .map(|mut clip| {
                n += 1;
                clip.id = ClipId(n);
                clip
            })
            .collect();
        let tree =
            self.tree
                .replace_range(start + len, 0, ClipTree::from_clips(copies), &mut || {
                    n += 1;
                    ClipId(n)
                });
        self.next_clip_id = n;
        self.commit(EditOp::Duplicate { start, len }, tree);
    }

    pub fn move_range(&mut self, from: u64, len: u64, dest: u64) {
        if len == 0 || dest == from {
            return;
        }
        let mut n = self.next_clip_id;
        let clips = self.tree.clips_in_range(from, len, &mut || {
            n += 1;
            ClipId(n)
        });
        let extracted = self
            .tree
            .replace_range(from, len, ClipTree::empty(), &mut || {
                n += 1;
                ClipId(n)
            });
        let insert_at = if dest > from {
            dest.saturating_sub(len)
        } else {
            dest
        };
        let tree = extracted.replace_range(
            insert_at.min(extracted.frames()),
            0,
            ClipTree::from_clips(clips),
            &mut || {
                n += 1;
                ClipId(n)
            },
        );
        self.next_clip_id = n;
        self.commit(EditOp::Move { from, len, dest }, tree);
    }

    pub fn roll(&mut self, at: u64, delta: i64) {
        let Some(span) = self.tree.at(at) else {
            return;
        };
        let Some(source) = &span.clip.source else {
            return;
        };
        let Some(media) = self.pool.get(source.media_id) else {
            return;
        };
        let rolled = span.clip.with_rolled_offset(delta, media.frame_count);
        let tree = self.tree.map_clip_at(at, |_| rolled);
        self.commit(EditOp::Roll { at, delta }, tree);
    }

    pub fn replace_range(&mut self, start: u64, len: u64, clips: Vec<Clip>) {
        let mut n = self.next_clip_id;
        let tree = self
            .tree
            .replace_range(start, len, ClipTree::from_clips(clips), &mut || {
                n += 1;
                ClipId(n)
            });
        self.next_clip_id = n;
        self.commit(EditOp::Remove { start, len }, tree);
    }

    pub fn read_planar(&self, start: u64, count: u64, dest: &mut [&mut [f32]]) -> Result<()> {
        for ch in dest.iter_mut() {
            let n = (*ch).len().min(count as usize);
            (*ch)[..n].fill(0.0);
        }
        if count == 0 || self.tree.is_empty() {
            return Ok(());
        }
        let start = start.min(self.frames());
        let count = count.min(self.frames().saturating_sub(start));
        let mut remaining = count;
        let mut pos = start;
        let mut dest_off = 0usize;
        while remaining > 0 {
            let Some(span) = self.tree.at(pos) else {
                break;
            };
            let local = pos - span.start;
            let take = remaining.min(span.clip.len - local);
            self.read_clip(span.clip.as_ref(), local, take, dest, dest_off)?;
            remaining -= take;
            pos += take;
            dest_off += take as usize;
        }
        Ok(())
    }

    pub fn read_channel(&self, channel: usize, start: u64, dest: &mut [f32]) -> Result<()> {
        dest.fill(0.0);
        if dest.is_empty() || channel >= self.channel_count || self.tree.is_empty() {
            return Ok(());
        }
        let start = start.min(self.frames());
        let mut remaining = (dest.len() as u64).min(self.frames().saturating_sub(start));
        let mut pos = start;
        let mut dest_off = 0usize;
        while remaining > 0 {
            let Some(span) = self.tree.at(pos) else {
                break;
            };
            let local = pos - span.start;
            let take = remaining.min(span.clip.len - local) as usize;
            self.read_clip_channel(
                span.clip.as_ref(),
                channel,
                local,
                &mut dest[dest_off..dest_off + take],
            )?;
            remaining -= take as u64;
            pos += take as u64;
            dest_off += take;
        }
        Ok(())
    }

    pub fn read_interleaved(&self, start: u64, count: u64, dest: &mut [f32]) -> Result<()> {
        let ch = self.channel_count.max(1);
        let frames = count as usize;
        let need = frames * ch;
        if dest.len() < need {
            bail!("destination is shorter than interleaved frame count");
        }
        dest[..need].fill(0.0);
        if count == 0 || self.channel_count == 0 {
            return Ok(());
        }
        let mut plane = vec![0.0; frames];
        for c in 0..self.channel_count {
            self.read_channel(c, start, &mut plane)?;
            for frame in 0..frames {
                dest[frame * ch + c] = plane[frame];
            }
        }
        Ok(())
    }

    pub fn fill_minmax_columns(
        &self,
        channel: usize,
        start: f64,
        samples_per_pixel: f64,
        dest: &mut [(f32, f32)],
    ) {
        dest.fill((0.0, 0.0));
        if samples_per_pixel <= 0.0 || dest.is_empty() || channel >= self.channel_count {
            return;
        }
        let frames = self.frames() as f64;
        for (col, slot) in dest.iter_mut().enumerate() {
            let a = start + col as f64 * samples_per_pixel;
            if a >= frames {
                break;
            }
            *slot = self.min_max_in_range(channel, a, a + samples_per_pixel);
        }
    }

    pub fn frames_iter(&self, start: u64) -> FramesIter<'_> {
        FramesIter {
            composition: self,
            pos: start.min(self.frames()),
            buf: vec![0.0; self.channel_count.max(1)],
        }
    }

    pub fn min_max_in_range(&self, channel: usize, start: f64, end: f64) -> (f32, f32) {
        if self.frames() == 0 || channel >= self.channel_count {
            return (0.0, 0.0);
        }
        let start_i = start.max(0.0).floor() as u64;
        let end_i = (end.ceil() as u64).min(self.frames()).max(start_i);
        if start_i >= end_i {
            return (0.0, 0.0);
        }
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        let mut pos = start_i;
        while pos < end_i {
            let Some(span) = self.tree.at(pos) else {
                break;
            };
            let local = pos - span.start;
            let take = (end_i - pos).min(span.clip.len - local);
            let (cmin, cmax) = self.clip_min_max(span.clip.as_ref(), channel, local, take);
            min = min.min(cmin);
            max = max.max(cmax);
            pos += take;
        }
        if min > max {
            (0.0, 0.0)
        } else {
            (min, max)
        }
    }

    fn clip_min_max(&self, clip: &Clip, channel: usize, local: u64, len: u64) -> (f32, f32) {
        if clip.source.is_none() || len == 0 {
            return (0.0, 0.0);
        }
        let Some(peaks) = clip.cache.peaks.get(channel) else {
            return (0.0, 0.0);
        };
        if peaks.is_empty() {
            // Missing overview bins: do not decode PCM on the UI thread.
            // `ensure_clip_peaks` / `build_missing_peak_caches` refill these.
            return (0.0, 0.0);
        }
        let peak_start = local as usize / PEAK_BLOCK;
        let peak_end = (((local + len) as usize + PEAK_BLOCK - 1) / PEAK_BLOCK).min(peaks.len());
        if peak_start < peak_end {
            let mut min = f32::MAX;
            let mut max = f32::MIN;
            for &(pmin, pmax) in &peaks[peak_start..peak_end] {
                min = min.min(pmin);
                max = max.max(pmax);
            }
            if min <= max {
                return (min, max);
            }
        }
        // Uncovered tail after an unaligned split (at most one peak block).
        let take = (len as usize).min(PEAK_BLOCK);
        let mut buf = vec![0.0; take];
        let _ = self.read_clip_channel(clip, channel, local, &mut buf);
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        for sample in buf {
            min = min.min(sample);
            max = max.max(sample);
        }
        if min > max {
            (0.0, 0.0)
        } else {
            (min, max)
        }
    }

    fn read_clip_channel(
        &self,
        clip: &Clip,
        channel: usize,
        local: u64,
        dest: &mut [f32],
    ) -> Result<()> {
        dest.fill(0.0);
        if dest.is_empty() {
            return Ok(());
        }
        if let Some(source) = &clip.source {
            let mut pager = self.pager.lock().unwrap();
            pager.fill_channel(
                &self.pool,
                source.media_id,
                source.offset + local,
                dest.len() as u64,
                channel,
                dest,
            )?;
        }
        if clip.fade_in == 0 && clip.fade_out == 0 {
            return Ok(());
        }
        for (frame, sample) in dest.iter_mut().enumerate() {
            let gain = clip.gain_at(local + frame as u64);
            if (gain - 1.0).abs() >= f32::EPSILON {
                *sample *= gain;
            }
        }
        Ok(())
    }

    fn read_clip(
        &self,
        clip: &Clip,
        local: u64,
        count: u64,
        dest: &mut [&mut [f32]],
        dest_offset: usize,
    ) -> Result<()> {
        if let Some(source) = &clip.source {
            let mut pager = self.pager.lock().unwrap();
            pager.fill_planar(
                &self.pool,
                source.media_id,
                source.offset + local,
                count,
                dest,
                dest_offset,
            )?;
        }
        if clip.fade_in == 0 && clip.fade_out == 0 {
            return Ok(());
        }
        for frame in 0..count as usize {
            let gain = clip.gain_at(local + frame as u64);
            if (gain - 1.0).abs() < f32::EPSILON {
                continue;
            }
            for plane in dest.iter_mut() {
                let i = dest_offset + frame;
                if i < plane.len() {
                    plane[i] *= gain;
                }
            }
        }
        Ok(())
    }

    pub fn needs_peak_build(&self) -> bool {
        self.spans().iter().any(|span| span.clip.needs_peak_cache())
    }

    /// Overview paint can proceed if some clips already have bins, even while
    /// others are still rebuilding after a split.
    pub fn can_paint_overview(&self) -> bool {
        !self.needs_peak_build()
            || self
                .spans()
                .iter()
                .any(|span| !span.clip.cache.is_missing_peaks())
    }

    pub fn ensure_clip_peaks(&mut self) -> Result<()> {
        let updates = self.build_missing_peak_caches(None, 0)?;
        self.apply_peak_caches(updates);
        Ok(())
    }

    pub fn apply_peak_caches(&mut self, updates: Vec<(u64, Clip)>) {
        for (_, peaked) in updates {
            self.tree = apply_clip_cache(&self.tree, &peaked);
            self.edl
                .map_snapshots(|tree| apply_clip_cache(tree, &peaked));
        }
    }

    pub fn build_missing_peak_caches(
        &self,
        progress: Option<&ProgressHandle>,
        epoch: u64,
    ) -> Result<Vec<(u64, Clip)>> {
        let spans = self.tree.spans();
        let total: u64 = spans
            .iter()
            .filter(|span| span.clip.needs_peak_cache())
            .map(|span| span.clip.len.saturating_mul(self.channel_count as u64))
            .sum();
        let mut done = 0u64;
        let mut updated = Vec::new();
        if let Some(progress) = progress {
            progress.set_ratio(epoch, 0, total.max(1));
        }
        for span in &spans {
            if progress.is_some_and(|p| !p.is_epoch(epoch)) {
                break;
            }
            if !span.clip.needs_peak_cache() {
                continue;
            }
            let mut peaks = vec![Vec::new(); self.channel_count];
            let mut min = f32::MAX;
            let mut max = f32::MIN;
            let mut pos = 0u64;
            let mut chunk = vec![0.0; PEAK_BLOCK];
            while pos < span.clip.len {
                if progress.is_some_and(|p| !p.is_epoch(epoch)) {
                    return Ok(updated);
                }
                let take = ((span.clip.len - pos) as usize).min(PEAK_BLOCK);
                for ch in 0..self.channel_count {
                    let dest = &mut chunk[..take];
                    self.read_clip_channel(span.clip.as_ref(), ch, pos, dest)?;
                    let mut pmin = f32::MAX;
                    let mut pmax = f32::MIN;
                    for &s in dest.iter() {
                        pmin = pmin.min(s);
                        pmax = pmax.max(s);
                        min = min.min(s);
                        max = max.max(s);
                    }
                    peaks[ch].push(if pmin <= pmax {
                        (pmin, pmax)
                    } else {
                        (0.0, 0.0)
                    });
                    done += take as u64;
                    if let Some(progress) = progress {
                        progress.set_ratio(epoch, done, total.max(1));
                    }
                }
                pos += take as u64;
            }
            let mut clip = (*span.clip).clone();
            clip.cache = super::clip::ClipCache {
                min: if min <= max { Some(min) } else { None },
                max: if min <= max { Some(max) } else { None },
                peaks,
            };
            updated.push((span.start, clip));
        }
        if let Some(progress) = progress {
            progress.set_fraction(epoch, 1.0);
        }
        Ok(updated)
    }

    /// Same work as [`Self::build_missing_peak_caches`], but drops the
    /// composition read lock between peak chunks so the UI can paint.
    pub fn build_missing_peak_caches_shared(
        composition: &RwLock<Self>,
        progress: Option<&ProgressHandle>,
        epoch: u64,
    ) -> Result<Vec<(u64, Clip)>> {
        let (jobs, channel_count, total) = {
            let this = composition.read().unwrap();
            let channel_count = this.channel_count;
            let jobs: Vec<(u64, Arc<Clip>)> = this
                .tree
                .spans()
                .into_iter()
                .filter(|span| span.clip.needs_peak_cache())
                .map(|span| (span.start, span.clip.clone()))
                .collect();
            let total: u64 = jobs
                .iter()
                .map(|(_, clip)| clip.len.saturating_mul(channel_count as u64))
                .sum();
            (jobs, channel_count, total)
        };
        let mut done = 0u64;
        let mut updated = Vec::new();
        if let Some(progress) = progress {
            progress.set_ratio(epoch, 0, total.max(1));
        }
        for (start, clip) in jobs {
            if progress.is_some_and(|p| !p.is_epoch(epoch)) {
                break;
            }
            let mut peaks = vec![Vec::new(); channel_count];
            let mut min = f32::MAX;
            let mut max = f32::MIN;
            let mut pos = 0u64;
            let mut chunk = vec![0.0; PEAK_BLOCK];
            while pos < clip.len {
                if progress.is_some_and(|p| !p.is_epoch(epoch)) {
                    return Ok(updated);
                }
                let take = ((clip.len - pos) as usize).min(PEAK_BLOCK);
                {
                    let this = composition.read().unwrap();
                    for ch in 0..channel_count {
                        let dest = &mut chunk[..take];
                        this.read_clip_channel(clip.as_ref(), ch, pos, dest)?;
                        let mut pmin = f32::MAX;
                        let mut pmax = f32::MIN;
                        for &s in dest.iter() {
                            pmin = pmin.min(s);
                            pmax = pmax.max(s);
                            min = min.min(s);
                            max = max.max(s);
                        }
                        peaks[ch].push(if pmin <= pmax {
                            (pmin, pmax)
                        } else {
                            (0.0, 0.0)
                        });
                        done += take as u64;
                        if let Some(progress) = progress {
                            progress.set_ratio(epoch, done, total.max(1));
                        }
                    }
                }
                pos += take as u64;
            }
            let mut peaked = (*clip).clone();
            peaked.cache = super::clip::ClipCache {
                min: if min <= max { Some(min) } else { None },
                max: if min <= max { Some(max) } else { None },
                peaks,
            };
            updated.push((start, peaked));
        }
        if let Some(progress) = progress {
            progress.set_fraction(epoch, 1.0);
        }
        Ok(updated)
    }

    pub fn to_project_file(&self) -> ProjectFile {
        ProjectFile {
            sample_rate: self.sample_rate,
            channel_count: self.channel_count,
            media: self.pool.clone().into_refs(),
            initial: self.initial.clone(),
            edits: self.edl.ops_from_first_user(),
            edit_cursor: self.edl.cursor(),
            markers: self.markers.iter().map(StoredMarker::from).collect(),
            marker_types: self.marker_types.clone(),
            collections: self.collections.clone(),
        }
    }

    pub fn to_json(&self) -> Result<String> {
        ProjectEnvelope::wrap(self.to_project_file()).to_json()
    }

    pub fn from_project_file(file: ProjectFile) -> Result<Self> {
        let sample_rate = file.sample_rate;
        let channel_count = file.channel_count;
        let mut composition = match &file.initial {
            InitialState::Empty => Composition::new(sample_rate, channel_count),
            InitialState::FromMedia { media_id } => {
                let mut pool = MediaPool::new();
                for media in file.media.clone() {
                    pool.insert(media);
                }
                let media = pool
                    .get(MediaId(*media_id))
                    .cloned()
                    .context("initial media missing from project")?;
                let mut composed = Composition::from_media(media)?;
                composed.pool = pool;
                composed
            }
        };
        if matches!(file.initial, InitialState::Empty) {
            for media in file.media {
                composition.pool.insert(media);
            }
        }
        for op in file.edits {
            composition.replay(&op)?;
        }
        if let Some(tree) = composition.edl.jump_to_index(file.edit_cursor) {
            composition.adopt_tree(tree);
        }
        composition.markers =
            MarkerList::from_vec(file.markers.iter().cloned().map(Marker::from).collect());
        if file.marker_types.is_empty() {
            composition.marker_types = MarkerType::defaults();
        } else {
            composition.marker_types = file.marker_types;
        }
        for stored in &file.markers {
            if let Some(color) = stored.color {
                if composition.marker_type_color(&stored.marker_type).is_none() {
                    composition.add_marker_type(&stored.marker_type, color);
                }
            }
        }
        composition.collections = file
            .collections
            .into_iter()
            .filter(|col| col.name != SELECTION_COLLECTION && !col.name.is_empty())
            .collect();
        composition.bump_next_region_id_from_collections();
        composition.mark_clean();
        Ok(composition)
    }

    pub fn from_json(json: &str) -> Result<Self> {
        let envelope = ProjectEnvelope::from_json(json)?;
        Self::from_project_file(envelope.project)
    }

    pub fn from_json_reprobing(json: &str) -> Result<(Self, Vec<String>)> {
        let envelope = ProjectEnvelope::from_json(json)?;
        let mut warnings = Vec::new();
        let mut file = envelope.project;
        let mut resolved = Vec::with_capacity(file.media.len());
        for media in file.media {
            let path_str = media.path.to_string_lossy();
            if path_str.starts_with("memory://") {
                resolved.push(media);
                continue;
            }
            if !media.path.is_file() {
                bail!("missing source media {}", media.path.display());
            }
            let meta = std::fs::metadata(&media.path)
                .with_context(|| format!("failed to stat source media {}", media.path.display()))?;
            if media_stats_match(&media, &meta) {
                resolved.push(media);
                continue;
            }
            warnings.push(format!("source media changed: {}", media.path.display()));
            match crate::audio::probe_header(&media.path) {
                Ok(probed) => {
                    let mut next = media_ref_from_probed(probed);
                    next.id = media.id;
                    next.path = media.path;
                    resolved.push(next);
                }
                Err(_) => {
                    let mut next = media;
                    next.size_bytes = meta.len();
                    next.modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                    resolved.push(next);
                }
            }
        }
        file.media = resolved;
        Ok((Self::from_project_file(file)?, warnings))
    }

    fn replay(&mut self, op: &EditOp) -> Result<()> {
        match op {
            EditOp::Init => Ok(()),
            EditOp::Copy { start, len } => {
                self.copy(*start, *len);
                Ok(())
            }
            EditOp::Cut { start, len } => {
                self.cut(*start, *len);
                Ok(())
            }
            EditOp::Remove { start, len } => {
                self.remove(*start, *len);
                Ok(())
            }
            EditOp::Delete { start, len } => {
                self.delete(*start, *len);
                Ok(())
            }
            EditOp::Paste { at, .. } => self.paste(*at),
            EditOp::Trim { start, len } => {
                self.trim(*start, *len);
                Ok(())
            }
            EditOp::Duplicate { start, len } => {
                self.duplicate(*start, *len);
                Ok(())
            }
            EditOp::Move { from, len, dest } => {
                self.move_range(*from, *len, *dest);
                Ok(())
            }
            EditOp::Roll { at, delta } => {
                self.roll(*at, *delta);
                Ok(())
            }
        }
    }

    pub fn assert_invariants(&self) {
        let mut t = 0u64;
        for span in self.spans() {
            assert!(span.clip.len > 0, "zero-length clip");
            assert_eq!(span.start, t, "gap or overlap at {t}");
            t += span.clip.len;
        }
        assert_eq!(t, self.frames());
    }

    #[cfg(test)]
    fn replace_init_snapshot(&mut self, tree: ClipTree) {
        self.edl.replace_init_snapshot(tree);
    }
}

fn clip_cache_key(clip: &Clip) -> Option<(MediaId, u64, u64)> {
    clip.source
        .as_ref()
        .map(|source| (source.media_id, source.offset, clip.len))
}

fn apply_clip_cache(tree: &ClipTree, peaked: &Clip) -> ClipTree {
    let peaked_key = clip_cache_key(peaked);
    tree.map_clips(|clip| {
        let same =
            clip.id == peaked.id || (peaked_key.is_some() && clip_cache_key(clip) == peaked_key);
        if !same {
            return clip.clone();
        }
        let mut clip = clip.clone();
        clip.cache = peaked.cache.clone();
        clip
    })
}

fn media_stats_match(media: &MediaRef, meta: &std::fs::Metadata) -> bool {
    if meta.len() != media.size_bytes {
        return false;
    }
    let Ok(modified) = meta.modified() else {
        return true;
    };
    let stored = media
        .modified
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let actual = modified
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    stored.as_secs() == actual.as_secs()
}

fn media_ref_from_probed(probed: ProbedFile) -> MediaRef {
    MediaRef {
        id: MediaId(0),
        path: probed.path,
        sample_rate: probed.sample_rate,
        channel_count: probed.channel_count,
        frame_count: probed.frame_count,
        bits_per_sample: probed.bits_per_sample,
        size_bytes: probed.size_bytes,
        modified: probed.modified,
        container_format: probed.container_format,
        codec: probed.codec,
        hash: None,
        samples: probed.samples,
    }
}

pub fn is_facomp_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("facomp"))
}

fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "composition.facomp".into());
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    let tmp = match dir {
        Some(dir) => dir.join(format!(".{file_name}.tmp")),
        None => PathBuf::from(format!(".{file_name}.tmp")),
    };
    std::fs::write(&tmp, contents).with_context(|| format!("failed to write {}", tmp.display()))?;
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    std::fs::rename(&tmp, path).with_context(|| {
        format!(
            "failed to replace {} with {}",
            path.display(),
            tmp.display()
        )
    })
}

pub struct FramesIter<'a> {
    composition: &'a Composition,
    pos: u64,
    buf: Vec<f32>,
}

impl Iterator for FramesIter<'_> {
    type Item = Vec<f32>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.composition.frames() {
            return None;
        }
        self.buf.fill(0.0);
        self.composition
            .read_interleaved(self.pos, 1, &mut self.buf)
            .ok()?;
        self.pos += 1;
        Some(self.buf.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::super::markers::MARKER_TYPE_BLUE;
    use super::*;

    fn sine_media(frames: usize, channels: usize, rate: u32) -> MediaRef {
        let samples = (0..channels)
            .map(|ch| {
                (0..frames)
                    .map(|i| {
                        let t = i as f32 / rate as f32;
                        (t * (440.0 + ch as f32 * 110.0) * std::f32::consts::TAU).sin() * 0.5
                    })
                    .collect()
            })
            .collect();
        MediaRef::from_memory(MediaId(0), rate, samples)
    }

    fn materialize(comp: &Composition) -> Vec<Vec<f32>> {
        let frames = comp.frames() as usize;
        let mut planes = vec![vec![0.0; frames]; comp.channel_count()];
        if frames == 0 {
            return planes;
        }
        let mut refs: Vec<&mut [f32]> = planes.iter_mut().map(|p| p.as_mut_slice()).collect();
        comp.read_planar(0, frames as u64, &mut refs).unwrap();
        planes
    }

    #[test]
    fn from_media_reads_match_source() {
        let media = sine_media(64, 2, 44100);
        let expected = media.samples.as_ref().unwrap().clone();
        let comp = Composition::from_media(media).unwrap();
        comp.assert_invariants();
        assert_eq!(comp.frames(), 64);
        let got = materialize(&comp);
        assert_eq!(got, *expected);
        let clip = comp.clip_at(0).unwrap().clip;
        assert!(!clip.cache.peaks.is_empty());
    }

    #[test]
    fn unaligned_delete_keeps_right_peak_cache() {
        let block = crate::audio::PEAK_BLOCK as u64;
        let mut comp = Composition::from_media(sine_media((block * 4) as usize, 1, 44100)).unwrap();
        assert!(!comp.needs_peak_build());
        comp.delete(block + 7, 5);
        assert!(
            !comp.needs_peak_build(),
            "delete should reuse suffix peak bins instead of rebuilding"
        );
        let right = comp
            .spans()
            .into_iter()
            .rev()
            .find(|span| span.clip.source.is_some())
            .unwrap();
        assert!(!right.clip.cache.is_missing_peaks());
        let (min, max) =
            comp.min_max_in_range(0, right.start as f64, (right.start + right.clip.len) as f64);
        assert!(max > min);
    }

    #[test]
    fn remove_shrinks_delete_preserves_length() {
        let mut comp = Composition::from_media(sine_media(20, 1, 48000)).unwrap();
        comp.remove(5, 5);
        comp.assert_invariants();
        assert_eq!(comp.frames(), 15);
        comp.undo();
        comp.delete(5, 5);
        comp.assert_invariants();
        assert_eq!(comp.frames(), 20);
        let samples = materialize(&comp);
        assert!(samples[0][5..10].iter().all(|&s| s == 0.0));
        assert_ne!(samples[0][4], 0.0);
        assert_eq!(comp.modified_ranges(), vec![(5, 10)]);
        assert_eq!(comp.ranges_for_edit(comp.current_edit()), vec![(5, 10)]);
    }

    #[test]
    fn cut_copy_paste_and_undo() {
        let mut comp = Composition::from_media(sine_media(16, 1, 44100)).unwrap();
        let original = materialize(&comp);
        comp.cut(4, 4);
        assert_eq!(comp.frames(), 12);
        comp.paste(0).unwrap();
        assert_eq!(comp.frames(), 16);
        let after_paste = materialize(&comp);
        assert_eq!(after_paste[0][..4], original[0][4..8]);
        assert!(comp.undo());
        assert_eq!(comp.frames(), 12);
        assert!(comp.redo());
        assert_eq!(comp.frames(), 16);
    }

    #[test]
    fn duplicate_trim_move_roll() {
        let mut comp = Composition::from_media(sine_media(10, 1, 44100)).unwrap();
        comp.duplicate(0, 3);
        assert_eq!(comp.frames(), 13);
        comp.trim(0, 6);
        assert_eq!(comp.frames(), 6);
        comp.move_range(0, 2, 6);
        assert_eq!(comp.frames(), 6);
        comp.assert_invariants();

        let mut rolled = Composition::from_media(sine_media(10, 1, 44100)).unwrap();
        rolled.trim(0, 8);
        rolled.roll(0, 1);
        rolled.assert_invariants();
        let clip = rolled.clip_at(0).unwrap();
        assert_eq!(clip.clip.source.as_ref().unwrap().offset, 1);
        rolled.roll(0, 100);
        assert_eq!(
            rolled
                .clip_at(0)
                .unwrap()
                .clip
                .source
                .as_ref()
                .unwrap()
                .offset,
            2
        );
    }

    #[test]
    fn jump_to_edit_restores_snapshot() {
        let mut comp = Composition::from_media(sine_media(8, 1, 44100)).unwrap();
        let init = comp.current_edit();
        comp.remove(0, 2);
        let after = comp.current_edit();
        comp.delete(0, 2);
        assert!(comp.jump_to_edit(init));
        assert_eq!(comp.frames(), 8);
        assert!(comp.jump_to_edit(after));
        assert_eq!(comp.frames(), 6);
    }

    #[test]
    fn undo_to_init_keeps_original_media() {
        let mut comp = Composition::from_media(sine_media(12, 1, 44100)).unwrap();
        let frames = comp.frames();
        let init = comp.current_edit();
        comp.remove(2, 4);
        assert_eq!(comp.frames(), 8);
        assert!(comp.undo());
        assert_eq!(comp.current_edit(), init);
        assert_eq!(comp.frames(), frames);
        assert!(comp.clip_at(0).is_some());
        assert!(!comp.clip_at(0).unwrap().clip.cache.peaks.is_empty());
    }

    #[test]
    fn new_composition_is_clean() {
        let comp = Composition::new(44100, 2);
        assert!(!comp.is_modified());
        let from_media = Composition::from_media(sine_media(12, 1, 44100)).unwrap();
        assert!(!from_media.is_modified());
    }

    #[test]
    fn edit_then_undo_to_init_is_clean() {
        let mut comp = Composition::from_media(sine_media(12, 1, 44100)).unwrap();
        assert!(!comp.is_modified());
        comp.remove(2, 4);
        assert!(comp.is_modified());
        assert!(comp.undo());
        assert!(!comp.is_modified());
    }

    #[test]
    fn load_with_history_is_clean_until_edited() {
        let mut live = Composition::from_media(sine_media(12, 1, 44100)).unwrap();
        live.remove(2, 2);
        assert!(live.edit_cursor() > 0);
        let json = live.to_json().unwrap();
        let mut restored = Composition::from_json(&json).unwrap();
        assert!(restored.edit_cursor() > 0);
        assert!(!restored.is_modified());
        restored.remove(0, 1);
        assert!(restored.is_modified());
        assert!(restored.undo());
        assert!(!restored.is_modified());
    }

    #[test]
    fn marker_add_and_remove_are_dirty() {
        let mut comp = Composition::from_media(sine_media(12, 1, 44100)).unwrap();
        let id = comp.add_marker(0, MARKER_TYPE_BLUE, None).unwrap();
        assert!(comp.is_modified());
        assert!(comp.remove_marker(id));
        assert!(!comp.is_modified());
        comp.add_marker(4, MARKER_TYPE_BLUE, None);
        assert!(comp.is_modified());
    }

    #[test]
    fn save_clears_dirty() {
        let mut live = Composition::from_media(sine_media(12, 1, 44100)).unwrap();
        live.remove(2, 2);
        live.add_marker(0, MARKER_TYPE_BLUE, None);
        assert!(live.is_modified());
        let path = std::env::temp_dir().join("snd-composition-dirty-save.facomp");
        live.save_to_path(&path).unwrap();
        assert!(!live.is_modified());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn empty_init_snapshot_rebuilds_from_media() {
        let mut comp = Composition::from_media(sine_media(16, 1, 44100)).unwrap();
        let frames = comp.frames();
        let init = comp.current_edit();
        comp.remove(0, 4);
        comp.replace_init_snapshot(ClipTree::empty());
        assert!(comp.jump_to_edit(init));
        assert_eq!(comp.frames(), frames);
        assert!(!comp.is_empty());
        assert!(comp.clip_at(0).is_some());
    }

    #[test]
    fn paste_rejects_rate_mismatch() {
        let mut comp = Composition::new(48000, 2);
        comp.clipboard = Clipboard {
            sample_rate: 44100,
            channel_count: 2,
            clips: vec![Clip::silence(ClipId(1), 4)],
        };
        let err = comp.paste(0).unwrap_err();
        assert!(err.to_string().contains("clipboard"));
    }

    #[test]
    fn project_json_round_trip_skips_pcm() {
        let mut comp = Composition::from_media(sine_media(12, 1, 44100)).unwrap();
        comp.remove(2, 2);
        let json = comp.to_json().unwrap();
        assert!(!json.contains("samples"));
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value["media"][0].get("samples").is_none());
        assert_eq!(value["kind"], "facomp");
        assert_eq!(value["format_version"], 3);
        let media = &value["media"][0];
        assert!(media.get("path").is_some());
        assert!(media.get("size_bytes").is_some());
        assert!(media.get("modified").is_some());
        assert_eq!(media["container_format"], "memory");
        assert_eq!(media["codec"], "pcm");
        let restored = Composition::from_json(&json).unwrap();
        assert_eq!(restored.frames(), comp.frames());
        assert_eq!(restored.current_edit().0, comp.current_edit().0);
        restored.assert_invariants();
    }

    #[test]
    fn project_json_rejects_wrong_kind_and_future_version() {
        let err = Composition::from_json(
            r#"{"kind":"other","format_version":1,"sample_rate":1,"channel_count":1,"media":[],"initial":{"type":"empty"},"edits":[],"edit_cursor":0}"#,
        )
        .err()
        .unwrap()
        .to_string();
        assert!(err.contains("kind"));
        let err = Composition::from_json(r#"{"kind":"facomp"}"#)
            .err()
            .unwrap()
            .to_string();
        assert!(
            err.contains("format_version")
                || err.contains("missing field")
                || err.contains("parse project JSON"),
            "{err}"
        );
        let err = Composition::from_json(
            r#"{"kind":"facomp","format_version":99,"sample_rate":1,"channel_count":1,"media":[],"initial":{"type":"empty"},"edits":[],"edit_cursor":0}"#,
        )
        .err()
        .unwrap()
        .to_string();
        assert!(err.contains("newer snd-review"));
    }

    #[test]
    fn suggested_facomp_name_appends_extension() {
        let media = sine_media(4, 1, 44100);
        let mut media = media;
        media.path = std::path::PathBuf::from("take.wav");
        let comp = Composition::from_media(media).unwrap();
        assert_eq!(comp.suggested_facomp_name(), "take.wav.facomp");
        assert_eq!(comp.display_name(), "take.wav");
    }

    #[test]
    fn frames_iter_matches_interleaved_read() {
        let comp = Composition::from_media(sine_media(5, 2, 44100)).unwrap();
        let mut all = vec![0.0; 10];
        comp.read_interleaved(0, 5, &mut all).unwrap();
        let collected: Vec<f32> = comp.frames_iter(0).flatten().collect();
        assert_eq!(collected, all);
    }

    #[test]
    fn load_from_wav_pages_off_disk() {
        use std::io::Write;
        fn write_sine_wav(path: &std::path::Path, channels: u16, frames: u32, sample_rate: u32) {
            let bits_per_sample: u16 = 16;
            let block_align = channels * bits_per_sample / 8;
            let byte_rate = sample_rate * u32::from(block_align);
            let data_len = frames * u32::from(block_align);
            let mut out = std::fs::File::create(path).unwrap();
            out.write_all(b"RIFF").unwrap();
            out.write_all(&(36 + data_len).to_le_bytes()).unwrap();
            out.write_all(b"WAVE").unwrap();
            out.write_all(b"fmt ").unwrap();
            out.write_all(&16u32.to_le_bytes()).unwrap();
            out.write_all(&1u16.to_le_bytes()).unwrap();
            out.write_all(&channels.to_le_bytes()).unwrap();
            out.write_all(&sample_rate.to_le_bytes()).unwrap();
            out.write_all(&byte_rate.to_le_bytes()).unwrap();
            out.write_all(&block_align.to_le_bytes()).unwrap();
            out.write_all(&bits_per_sample.to_le_bytes()).unwrap();
            out.write_all(b"data").unwrap();
            out.write_all(&data_len.to_le_bytes()).unwrap();
            for i in 0..frames {
                let t = i as f32 / sample_rate as f32;
                let sample = (t * 440.0 * std::f32::consts::TAU).sin();
                let pcm = (sample * 0.6 * i16::MAX as f32) as i16;
                for _ in 0..channels {
                    out.write_all(&pcm.to_le_bytes()).unwrap();
                }
            }
        }
        let dir = std::env::temp_dir();
        let path = dir.join("snd-composition-page-test.wav");
        write_sine_wav(&path, 1, 512, 44100);
        let mut comp = Composition::load_from_path(&path).unwrap();
        assert_eq!(comp.frames(), 512);
        assert!(comp.pool().first().unwrap().samples.is_none());
        let mut dest = vec![0.0; 8];
        comp.read_interleaved(10, 8, &mut dest).unwrap();
        assert!(dest.iter().any(|&s| s.abs() > 0.01));
        assert!(comp.needs_peak_build());
        let progress = crate::progress::ProgressHandle::new();
        let epoch = progress.begin("building peaks");
        let updates = comp
            .build_missing_peak_caches(Some(&progress), epoch)
            .unwrap();
        assert!(!updates.is_empty());
        assert_eq!(
            progress.snapshot().unwrap().message(),
            "building peaks 100%"
        );
        comp.apply_peak_caches(updates);
        assert!(!comp.needs_peak_build());
        progress.finish(epoch);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn shared_peak_build_fills_file_backed_caches() {
        use std::io::Write;
        fn write_sine_wav(path: &std::path::Path, channels: u16, frames: u32, sample_rate: u32) {
            let bits_per_sample: u16 = 16;
            let block_align = channels * bits_per_sample / 8;
            let byte_rate = sample_rate * u32::from(block_align);
            let data_len = frames * u32::from(block_align);
            let mut out = std::fs::File::create(path).unwrap();
            out.write_all(b"RIFF").unwrap();
            out.write_all(&(36 + data_len).to_le_bytes()).unwrap();
            out.write_all(b"WAVE").unwrap();
            out.write_all(b"fmt ").unwrap();
            out.write_all(&16u32.to_le_bytes()).unwrap();
            out.write_all(&1u16.to_le_bytes()).unwrap();
            out.write_all(&channels.to_le_bytes()).unwrap();
            out.write_all(&sample_rate.to_le_bytes()).unwrap();
            out.write_all(&byte_rate.to_le_bytes()).unwrap();
            out.write_all(&block_align.to_le_bytes()).unwrap();
            out.write_all(&bits_per_sample.to_le_bytes()).unwrap();
            out.write_all(b"data").unwrap();
            out.write_all(&data_len.to_le_bytes()).unwrap();
            for i in 0..frames {
                let t = i as f32 / sample_rate as f32;
                let sample = (t * 440.0 * std::f32::consts::TAU).sin();
                let pcm = (sample * 0.6 * i16::MAX as f32) as i16;
                for _ in 0..channels {
                    out.write_all(&pcm.to_le_bytes()).unwrap();
                }
            }
        }
        let path = std::env::temp_dir().join("snd-composition-shared-peak-test.wav");
        write_sine_wav(&path, 1, 512, 44100);
        let lock = std::sync::RwLock::new(Composition::load_from_path(&path).unwrap());
        assert!(lock.read().unwrap().needs_peak_build());
        let progress = crate::progress::ProgressHandle::new();
        let epoch = progress.begin("building peaks");
        let updates =
            Composition::build_missing_peak_caches_shared(&lock, Some(&progress), epoch).unwrap();
        assert!(!updates.is_empty());
        lock.write().unwrap().apply_peak_caches(updates);
        assert!(!lock.read().unwrap().needs_peak_build());
        progress.finish(epoch);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn facomp_save_and_reload_reprobes_media() {
        use std::io::Write;
        fn write_sine_wav(path: &std::path::Path, channels: u16, frames: u32, sample_rate: u32) {
            let bits_per_sample: u16 = 16;
            let block_align = channels * bits_per_sample / 8;
            let byte_rate = sample_rate * u32::from(block_align);
            let data_len = frames * u32::from(block_align);
            let mut out = std::fs::File::create(path).unwrap();
            out.write_all(b"RIFF").unwrap();
            out.write_all(&(36 + data_len).to_le_bytes()).unwrap();
            out.write_all(b"WAVE").unwrap();
            out.write_all(b"fmt ").unwrap();
            out.write_all(&16u32.to_le_bytes()).unwrap();
            out.write_all(&1u16.to_le_bytes()).unwrap();
            out.write_all(&channels.to_le_bytes()).unwrap();
            out.write_all(&sample_rate.to_le_bytes()).unwrap();
            out.write_all(&byte_rate.to_le_bytes()).unwrap();
            out.write_all(&block_align.to_le_bytes()).unwrap();
            out.write_all(&bits_per_sample.to_le_bytes()).unwrap();
            out.write_all(b"data").unwrap();
            out.write_all(&data_len.to_le_bytes()).unwrap();
            for i in 0..frames {
                let t = i as f32 / sample_rate as f32;
                let sample = (t * 440.0 * std::f32::consts::TAU).sin();
                let pcm = (sample * 0.6 * i16::MAX as f32) as i16;
                for _ in 0..channels {
                    out.write_all(&pcm.to_le_bytes()).unwrap();
                }
            }
        }
        let dir = std::env::temp_dir();
        let wav = dir.join("snd-composition-facomp-src.wav");
        let facomp = dir.join("snd-composition-facomp-src.wav.facomp");
        write_sine_wav(&wav, 1, 256, 44100);
        let mut live = Composition::load_from_path(&wav).unwrap();
        live.remove(10, 8);
        live.save_to_path(&facomp).unwrap();
        let json = std::fs::read_to_string(&facomp).unwrap();
        assert!(json.contains("\"kind\": \"facomp\""));
        assert!(!json.contains("samples"));
        let (restored, warnings) = Composition::load_facomp(&facomp).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(restored.frames(), live.frames());
        assert_eq!(restored.sample_rate(), 44100);
        let media = restored.pool().first().unwrap();
        assert_eq!(media.container_format, "wav");
        assert!(media.samples.is_none());
        let _ = std::fs::remove_file(wav);
        let _ = std::fs::remove_file(facomp);
    }

    #[test]
    fn facomp_reload_skips_audio_probe_when_stats_match() {
        let dummy = std::env::temp_dir().join("snd-facomp-not-audio.bin");
        std::fs::write(&dummy, b"not an audio file at all!!").unwrap();
        let meta = std::fs::metadata(&dummy).unwrap();
        let media = MediaRef {
            id: MediaId(1),
            path: dummy.clone(),
            sample_rate: 44100,
            channel_count: 1,
            frame_count: 1000,
            bits_per_sample: Some(16),
            size_bytes: meta.len(),
            modified: meta.modified().unwrap_or(std::time::UNIX_EPOCH),
            container_format: "wav".into(),
            codec: "pcm".into(),
            hash: None,
            samples: None,
        };
        let file = ProjectFile {
            sample_rate: 44100,
            channel_count: 1,
            media: vec![media],
            initial: InitialState::FromMedia { media_id: 1 },
            edits: Vec::new(),
            edit_cursor: 0,
            markers: Vec::new(),
            marker_types: Vec::new(),
            collections: Vec::new(),
        };
        let json = ProjectEnvelope::wrap(file).to_json().unwrap();
        let (comp, warnings) = Composition::from_json_reprobing(&json).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(comp.frames(), 1000);
        assert!(comp.pool().first().unwrap().samples.is_none());
        let _ = std::fs::remove_file(dummy);
    }

    #[test]
    fn remove_matches_oracle_slice() {
        let media = sine_media(24, 1, 44100);
        let expected_src = media.samples.as_ref().unwrap()[0].clone();
        let mut comp = Composition::from_media(media).unwrap();
        comp.remove(6, 5);
        comp.assert_invariants();
        let mut expected = expected_src.clone();
        expected.drain(6..11);
        assert_eq!(materialize(&comp)[0], expected);
    }

    #[test]
    fn copy_does_not_change_timeline() {
        let mut comp = Composition::from_media(sine_media(8, 1, 44100)).unwrap();
        let before = materialize(&comp);
        comp.copy(2, 3);
        assert_eq!(comp.frames(), 8);
        assert_eq!(materialize(&comp), before);
        assert_eq!(comp.clipboard().frames(), 3);
    }

    #[test]
    fn fade_in_scales_leading_frames() {
        let media = sine_media(8, 1, 44100);
        let original = media.samples.as_ref().unwrap()[0].clone();
        let mut comp = Composition::from_media(media).unwrap();
        let mut clip = (*comp.clip_at(0).unwrap().clip).clone();
        clip.fade_in = 4;
        let len = clip.len;
        comp.replace_range(0, len, vec![clip]);
        let got = materialize(&comp);
        assert_eq!(got[0][0], 0.0);
        assert!((got[0][2] - original[2] * 0.5).abs() < 1e-5);
        assert!((got[0][4] - original[4]).abs() < 1e-5);
    }

    #[test]
    fn json_replay_matches_live_snapshots() {
        let mut live = Composition::from_media(sine_media(20, 1, 44100)).unwrap();
        live.remove(2, 2);
        live.delete(0, 3);
        live.duplicate(0, 4);
        let json = live.to_json().unwrap();
        assert!(!json.contains("samples"));
        let restored = Composition::from_json(&json).unwrap();
        restored.assert_invariants();
        assert_eq!(restored.frames(), live.frames());
        assert_eq!(restored.current_edit().0, live.current_edit().0);
        let live_spans: Vec<_> = live
            .spans()
            .into_iter()
            .map(|s| (s.start, s.clip.len, s.clip.source.clone()))
            .collect();
        let restored_spans: Vec<_> = restored
            .spans()
            .into_iter()
            .map(|s| (s.start, s.clip.len, s.clip.source.clone()))
            .collect();
        assert_eq!(restored_spans, live_spans);
    }

    #[test]
    fn new_edit_drops_redo_tail() {
        let mut comp = Composition::from_media(sine_media(10, 1, 44100)).unwrap();
        comp.remove(0, 1);
        comp.remove(0, 1);
        assert!(comp.undo());
        assert!(comp.can_redo());
        comp.delete(0, 1);
        assert!(!comp.can_redo());
        assert_eq!(comp.frames(), 9);
    }

    #[test]
    fn zero_length_clips_are_dropped() {
        let mut comp = Composition::from_media(sine_media(6, 1, 44100)).unwrap();
        comp.replace_range(2, 2, vec![Clip::silence(ClipId(99), 0)]);
        comp.assert_invariants();
        assert_eq!(comp.frames(), 4);
        assert!(comp.spans().iter().all(|s| s.clip.len > 0));
    }

    #[test]
    fn markers_remap_through_cut_and_undo() {
        use super::super::markers::MARKER_TYPE_BLUE;

        let mut comp = Composition::from_media(sine_media(40, 1, 44100)).unwrap();
        comp.add_marker(5, MARKER_TYPE_BLUE, None).unwrap();
        comp.add_marker(15, MARKER_TYPE_BLUE, None).unwrap();
        comp.add_marker(30, MARKER_TYPE_BLUE, None).unwrap();
        comp.cut(10, 10);
        let frames: Vec<u64> = comp.markers().iter().map(|m| m.frame).collect();
        assert_eq!(frames, vec![5, 20]);
        assert!(comp.undo());
        let frames: Vec<u64> = comp.markers().iter().map(|m| m.frame).collect();
        assert_eq!(frames, vec![5, 30]);
    }

    #[test]
    fn project_json_v1_loads_without_markers() {
        let json = r#"{
            "kind":"facomp",
            "format_version":1,
            "sample_rate":44100,
            "channel_count":1,
            "media":[],
            "initial":{"type":"empty"},
            "edits":[],
            "edit_cursor":0
        }"#;
        let restored = Composition::from_json(json).unwrap();
        assert!(restored.markers().is_empty());
        assert_eq!(restored.sample_rate(), 44100);
    }

    #[test]
    fn project_json_round_trip_keeps_markers() {
        use super::super::markers::{MARKER_TYPE_BLUE, MARKER_TYPE_YELLOW};

        let mut comp = Composition::from_media(sine_media(12, 1, 44100)).unwrap();
        comp.add_marker(3, MARKER_TYPE_BLUE, None).unwrap();
        comp.add_marker(8, MARKER_TYPE_YELLOW, Some("cue".into()))
            .unwrap();
        let json = comp.to_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["format_version"], 3);
        assert_eq!(value["markers"].as_array().unwrap().len(), 2);
        assert!(value["markers"][0].get("color").is_none());
        assert!(value["marker_types"].as_array().unwrap().len() >= 3);
        let restored = Composition::from_json(&json).unwrap();
        let markers: Vec<_> = restored.markers().iter().cloned().collect();
        assert_eq!(markers.len(), 2);
        assert_eq!(markers[0].frame, 3);
        assert_eq!(markers[0].marker_type, MARKER_TYPE_BLUE);
        assert_eq!(markers[1].frame, 8);
        assert_eq!(markers[1].note.as_deref(), Some("cue"));
    }

    #[test]
    fn project_json_v2_loads_without_collections() {
        let json = r#"{
            "kind":"facomp",
            "format_version":2,
            "sample_rate":44100,
            "channel_count":1,
            "media":[],
            "initial":{"type":"empty"},
            "edits":[],
            "edit_cursor":0,
            "markers":[]
        }"#;
        let restored = Composition::from_json(json).unwrap();
        assert!(restored.collections().is_empty());
        assert_eq!(restored.marker_types().len(), 3);
        assert_eq!(restored.sample_rate(), 44100);
    }

    #[test]
    fn project_json_round_trip_keeps_collections_and_marker_types() {
        use crate::model::buffer::ChannelScope;

        let mut comp = Composition::from_media(sine_media(12, 1, 44100)).unwrap();
        assert!(comp.add_marker_type("Red", [1.0, 0.0, 0.0, 1.0]));
        comp.add_named_region("silent", 2, 5, ChannelScope::all(), Some("gap".into()))
            .unwrap();
        let json = comp.to_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["format_version"], 3);
        assert_eq!(value["collections"].as_array().unwrap().len(), 1);
        assert!(value["marker_types"]
            .as_array()
            .unwrap()
            .iter()
            .any(|ty| ty["name"] == "Red"));
        let restored = Composition::from_json(&json).unwrap();
        assert!(restored.marker_types().iter().any(|ty| ty.name == "Red"));
        let silent = restored.collection("silent").expect("silent");
        assert_eq!(silent.regions[0].label.as_deref(), Some("gap"));
        assert_eq!(silent.regions[0].start, 2);
        assert_eq!(silent.regions[0].end, 5);
    }

    #[test]
    fn project_json_v2_instance_color_becomes_marker_type() {
        let json = r#"{
            "kind":"facomp",
            "format_version":2,
            "sample_rate":44100,
            "channel_count":1,
            "media":[],
            "initial":{"type":"empty"},
            "edits":[],
            "edit_cursor":0,
            "markers":[
                {"id":1,"frame":10,"type":"Red","color":[1.0,0.0,0.0,1.0],"note":"hit"}
            ]
        }"#;
        let restored = Composition::from_json(json).unwrap();
        let marker = restored.markers().iter().next().expect("marker");
        assert_eq!(marker.frame, 10);
        assert_eq!(marker.marker_type, "Red");
        assert_eq!(marker.note.as_deref(), Some("hit"));
        assert_eq!(restored.resolved_marker_color("Red"), [1.0, 0.0, 0.0, 1.0]);
        assert!(restored
            .marker_types()
            .iter()
            .any(|ty| ty.name == "Red" && ty.color == [1.0, 0.0, 0.0, 1.0]));
        let saved: serde_json::Value = serde_json::from_str(&restored.to_json().unwrap()).unwrap();
        assert!(saved["markers"][0].get("color").is_none());
        assert!(saved["marker_types"]
            .as_array()
            .unwrap()
            .iter()
            .any(|ty| ty["name"] == "Red"));
    }
}
