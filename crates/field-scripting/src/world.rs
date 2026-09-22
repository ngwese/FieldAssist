// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Headless session / document store for scripting.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use field_audio_model::{
    Buffer, ChannelScope, MediaStore, RegionCollection, RegionId, SamplePosition,
    SELECTION_COLLECTION,
};
use field_composition::{is_facomp_path, Composition, MarkerId, MarkerType};
use field_session::{
    is_fasession_path, placeholder_media_descriptor, DocumentId, Session, SessionDocument,
};

/// Open document held by the headless world.
pub struct OpenDocument {
    /// Timeline / EDL.
    pub composition: Arc<RwLock<Composition>>,
    /// Scratch buffer (unused for most Lua ops).
    pub buffer: Arc<RwLock<Buffer>>,
    /// Primary selection collection.
    pub selection: RegionCollection,
    /// Named region collections.
    pub collections: HashMap<String, RegionCollection>,
    /// Display name.
    pub name: String,
    /// Source / project path when known.
    pub path: Option<PathBuf>,
    /// Playhead sample.
    pub position: Option<SamplePosition>,
    next_region_id: u64,
}

impl OpenDocument {
    /// Wrap a composition.
    pub fn new(composition: Composition, name: impl Into<String>, path: Option<PathBuf>) -> Self {
        Self {
            composition: Arc::new(RwLock::new(composition)),
            buffer: Arc::new(RwLock::new(Buffer::empty())),
            selection: RegionCollection::new(SELECTION_COLLECTION),
            collections: HashMap::new(),
            name: name.into(),
            path,
            position: Some(SamplePosition {
                sample: 0,
                channels: ChannelScope::all(),
            }),
            next_region_id: 1,
        }
    }

    /// Wrap existing shared arcs (e.g. from a DesktopBackend's BufferDocument).
    ///
    /// Selection is empty; callers should set `selection`, `position`, and
    /// `collections` from the source document before running callbacks.
    pub fn from_shared(
        composition: Arc<RwLock<Composition>>,
        buffer: Arc<RwLock<Buffer>>,
        name: impl Into<String>,
        path: Option<PathBuf>,
        next_region_id: u64,
    ) -> Self {
        Self {
            composition,
            buffer,
            selection: RegionCollection::new(SELECTION_COLLECTION),
            collections: HashMap::new(),
            name: name.into(),
            path,
            position: Some(SamplePosition {
                sample: 0,
                channels: ChannelScope::all(),
            }),
            next_region_id: next_region_id.max(1),
        }
    }

    /// Current next-region-id counter (used by DesktopBackend to sync back).
    pub fn next_region_id(&self) -> u64 {
        self.next_region_id
    }

    /// Timeline length in samples.
    pub fn frames(&self) -> usize {
        self.composition.read().unwrap().frames() as usize
    }

    /// Sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.composition.read().unwrap().sample_rate()
    }

    /// Select a half-open sample range.
    pub fn select_range(&mut self, start: usize, stop: usize, channels: ChannelScope) {
        self.selection.clear();
        let (start, end) = if stop >= start {
            (start, stop.saturating_sub(1))
        } else {
            (stop, start.saturating_sub(1))
        };
        let id = RegionId(self.next_region_id);
        self.next_region_id += 1;
        self.selection.regions.push(field_audio_model::Region {
            id,
            start,
            end,
            channels,
            label: None,
        });
    }

    /// Select the whole timeline.
    pub fn select_all(&mut self) {
        let frames = self.frames();
        if frames == 0 {
            self.clear_selection();
            return;
        }
        self.select_range(0, frames, ChannelScope::all());
    }

    /// Clear the selection.
    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    /// Ensure a named collection exists.
    pub fn ensure_named_collection(&mut self, name: &str) {
        if name == SELECTION_COLLECTION {
            return;
        }
        self.collections
            .entry(name.to_string())
            .or_insert_with(|| RegionCollection::new(name.to_string()));
    }

    /// Named collection names (excluding selection).
    pub fn collection_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.collections.keys().cloned().collect();
        names.sort();
        names
    }

    /// Add a labeled region.
    pub fn add_labeled_region(
        &mut self,
        start: usize,
        stop: usize,
        channels: ChannelScope,
        label: Option<String>,
        collection: &str,
    ) -> RegionId {
        let (start, end) = if stop >= start {
            (start, stop.saturating_sub(1).max(start))
        } else {
            (stop, start.saturating_sub(1).max(stop))
        };
        let id = RegionId(self.next_region_id);
        self.next_region_id += 1;
        let region = field_audio_model::Region {
            id,
            start,
            end,
            channels,
            label,
        };
        if collection == SELECTION_COLLECTION {
            self.selection.regions.push(region);
        } else {
            self.ensure_named_collection(collection);
            self.collections
                .get_mut(collection)
                .unwrap()
                .regions
                .push(region);
        }
        id
    }

    /// Remove a region from selection or named collections.
    pub fn remove_region(&mut self, id: RegionId) -> bool {
        if self.selection.remove(id) {
            return true;
        }
        for collection in self.collections.values_mut() {
            if collection.remove(id) {
                return true;
            }
        }
        false
    }

    /// Marker type list.
    pub fn marker_types(&self) -> Vec<MarkerType> {
        self.composition.read().unwrap().marker_types().to_vec()
    }

    /// Add a marker.
    pub fn add_marker(
        &mut self,
        frame: usize,
        marker_type: &str,
        note: Option<String>,
    ) -> Option<MarkerId> {
        self.composition
            .write()
            .unwrap()
            .add_marker(frame as u64, marker_type, note)
    }

    /// Add a marker type.
    pub fn add_marker_type(&mut self, name: &str, color: [f32; 4]) -> bool {
        self.composition
            .write()
            .unwrap()
            .add_marker_type(name, color)
    }

    /// Remove a marker type.
    pub fn remove_marker_type(&mut self, name: &str) -> bool {
        self.composition.write().unwrap().remove_marker_type(name)
    }

    /// Remove a marker by id.
    pub fn remove_marker(&mut self, id: MarkerId) -> bool {
        self.composition.write().unwrap().remove_marker(id)
    }

    /// Remove markers at a frame.
    pub fn remove_marker_at(&mut self, sample: usize) -> bool {
        self.composition
            .write()
            .unwrap()
            .remove_marker_at(sample as u64)
    }

    /// Remove markers at a frame of a given type.
    pub fn remove_marker_at_type(&mut self, sample: usize, marker_type: &str) -> bool {
        self.composition
            .write()
            .unwrap()
            .remove_marker_at_type(sample as u64, marker_type)
    }

    /// Remove all markers of a type.
    pub fn remove_marker_by_type(&mut self, marker_type: &str) -> usize {
        self.composition
            .write()
            .unwrap()
            .remove_marker_by_type(marker_type)
    }

    /// Set playhead.
    pub fn set_position(&mut self, sample: usize, channels: ChannelScope) {
        self.position = Some(SamplePosition { sample, channels });
    }

    /// Selection spans as `(start, len)`.
    pub fn selection_spans(&self) -> Vec<(u64, u64)> {
        self.selection.edit_spans()
    }

    /// Copy a named collection into the selection collection.
    pub fn adopt_collection_as_selection(&mut self, name: &str) {
        self.selection.clear();
        if name == SELECTION_COLLECTION {
            return;
        }
        if let Some(collection) = self.collections.get(name) {
            for region in &collection.regions {
                self.selection.regions.push(region.clone());
            }
        }
    }

    /// Find a region in the selection or named collections.
    pub fn find_region(&self, id: RegionId) -> Option<(String, field_audio_model::Region)> {
        if let Some(region) = self.selection.regions.iter().find(|r| r.id == id) {
            return Some((SELECTION_COLLECTION.into(), region.clone()));
        }
        for (name, collection) in &self.collections {
            if let Some(region) = collection.regions.iter().find(|r| r.id == id) {
                return Some((name.clone(), region.clone()));
            }
        }
        None
    }

    /// Undo last edit.
    pub fn edit_undo(&mut self) -> bool {
        self.composition.write().unwrap().undo()
    }

    /// Redo.
    pub fn edit_redo(&mut self) -> bool {
        self.composition.write().unwrap().redo()
    }

    /// Copy selection to clipboard.
    pub fn edit_copy(&mut self) {
        let spans = self.selection_spans();
        if !spans.is_empty() {
            self.composition.write().unwrap().copy_ranges(&spans);
        }
    }

    /// Cut selection.
    pub fn edit_cut(&mut self) {
        let spans = self.selection_spans();
        self.edit_copy();
        for (start, len) in spans.into_iter().rev() {
            self.composition.write().unwrap().cut(start, len);
        }
        self.clear_selection();
    }

    /// Paste at playhead / selection start.
    pub fn edit_paste(&mut self) {
        let at = self
            .selection_spans()
            .first()
            .map(|(s, _)| *s)
            .or_else(|| self.position.as_ref().map(|p| p.sample as u64))
            .unwrap_or(0);
        let _ = self.composition.write().unwrap().paste(at);
    }

    /// Clear samples in selection (silence).
    pub fn edit_clear(&mut self) {
        // Approximate: cut then leave gap closed — composition clear if available.
        let spans = self.selection_spans();
        for (start, len) in spans.into_iter().rev() {
            self.composition.write().unwrap().cut(start, len);
        }
        self.clear_selection();
    }

    /// Remove selection (same as clear for headless).
    pub fn edit_remove(&mut self) {
        self.edit_clear();
    }

    /// Duplicate selection in place after itself.
    pub fn edit_duplicate(&mut self) {
        let spans = self.selection_spans();
        if spans.is_empty() {
            return;
        }
        self.edit_copy();
        if let Some((start, len)) = spans.last() {
            let _ = self.composition.write().unwrap().paste(start + len);
        }
    }

    /// Trim timeline to selection.
    pub fn edit_trim(&mut self) {
        let spans = self.selection_spans();
        let Some((start, len)) = spans.first().copied() else {
            return;
        };
        let frames = self.frames() as u64;
        if start + len < frames {
            self.composition
                .write()
                .unwrap()
                .cut(start + len, frames - (start + len));
        }
        if start > 0 {
            self.composition.write().unwrap().cut(0, start);
        }
        self.clear_selection();
    }
}

/// Process-local scripting world.
pub struct HeadlessWorld {
    /// Active session.
    pub session: Session,
    /// Open documents.
    pub docs: HashMap<DocumentId, OpenDocument>,
    /// Shared media store.
    pub media_store: Arc<Mutex<MediaStore>>,
    next_id: u128,
}

impl HeadlessWorld {
    /// Empty world with a new session.
    pub fn new() -> Self {
        Self {
            session: Session::new(),
            docs: HashMap::new(),
            media_store: Arc::new(Mutex::new(MediaStore::in_memory())),
            next_id: 0,
        }
    }

    fn alloc_id(&mut self) -> DocumentId {
        self.next_id += 1;
        DocumentId::from_u128(self.next_id)
    }

    /// Open audio or `.facomp` into the session.
    pub fn open_path(&mut self, path: &Path) -> anyhow::Result<DocumentId> {
        if is_fasession_path(path) {
            anyhow::bail!("use field.session.open for .fasession files");
        }
        let (composition, _warnings) = if is_facomp_path(path) {
            Composition::load_facomp(path)?
        } else {
            (Composition::from_media_path(path, None)?, Vec::new())
        };
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".into());
        Ok(self.push_composition(composition, name, Some(path.to_path_buf())))
    }

    /// Insert a composition as a session document.
    pub fn push_composition(
        &mut self,
        composition: Composition,
        name: impl Into<String>,
        path: Option<PathBuf>,
    ) -> DocumentId {
        let id = self.alloc_id();
        {
            let mut store = self.media_store.lock().unwrap();
            for media in composition.pool().iter() {
                store.intern(media.clone());
            }
        }
        let comp_id = composition.id();
        let doc = OpenDocument::new(composition, name, path.clone());
        let session_doc = match path.as_ref() {
            Some(p) if is_facomp_path(p) => {
                SessionDocument::new_composition(id, p.clone(), comp_id)
            }
            Some(p) => {
                let descriptor = doc
                    .composition
                    .read()
                    .unwrap()
                    .pool()
                    .first()
                    .map(|media| media.to_descriptor())
                    .unwrap_or_else(|| placeholder_media_descriptor(p));
                SessionDocument::new_media(id, p.clone(), comp_id, descriptor)
            }
            None => SessionDocument::new_composition(id, PathBuf::new(), comp_id),
        };
        self.docs.insert(id, doc);
        self.session.insert(session_doc);
        id
    }

    /// Active document id.
    pub fn active(&self) -> Option<DocumentId> {
        self.session.active()
    }

    /// Set active document.
    pub fn set_active(&mut self, id: DocumentId) -> bool {
        self.session.focus(id).is_some() || self.session.active() == Some(id)
    }

    /// Close a document.
    pub fn close(&mut self, id: DocumentId) {
        self.docs.remove(&id);
        self.session.close_document(id);
    }

    /// Replace document media/project from a path.
    pub fn replace(&mut self, id: DocumentId, path: &Path) -> anyhow::Result<DocumentId> {
        self.close(id);
        self.open_path(path)
    }

    /// Probe and intern media without opening a document.
    pub fn add_media(&mut self, path: &Path) -> anyhow::Result<field_audio_model::MediaId> {
        let probed = field_audio_io::probe_file(path)?;
        let media = field_composition::media_ref_from_probed(probed);
        let (id, _) = self.media_store.lock().unwrap().intern(media);
        Ok(id)
    }
}

impl Default for HeadlessWorld {
    fn default() -> Self {
        Self::new()
    }
}

/// Persistable string map type used by session / document properties.
#[allow(dead_code)]
pub type PropMap = BTreeMap<String, String>;
