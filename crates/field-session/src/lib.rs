// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

#![deny(missing_docs)]

//! # field-session
//!
//! Session document lists and `.fasession` I/O. Path classification for
//! nested `.facomp` entries uses a local helper (no dependency on
//! `field-composition`).
//!
//! ```
//! use field_session::{is_fasession_path, Session};
//! use std::path::Path;
//!
//! assert!(is_fasession_path(Path::new("batch.fasession")));
//! let session = Session::new();
//! assert!(session.is_empty());
//! ```

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use field_audio_model::{MediaDescriptor, MediaId};
use field_core::{deserialize_prefixed_uuid, serialize_prefixed_uuid, CompositionId, Location};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

/// Session file kind marker written into `.fasession` envelopes.
pub const FASESSION_KIND: &str = "fasession";
/// Current on-disk format version for `.fasession` files.
pub const FASESSION_FORMAT_VERSION: u32 = 2;

/// True when `path` looks like a `.facomp` project (extension only).
fn is_facomp_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("facomp"))
}

/// Stable identity for an open composition in the session (`doc:<uuid>`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocumentId(pub Uuid);

impl DocumentId {
    /// `new`.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// `from_u128`.
    pub fn from_u128(n: u128) -> Self {
        Self(Uuid::from_u128(n))
    }

    /// `to_tree_id`.
    pub fn to_tree_id(self) -> String {
        format!("doc:{}", self.0)
    }

    /// `from_tree_id`.
    pub fn from_tree_id(id: &str) -> Option<Self> {
        let rest = id.strip_prefix("doc:")?;
        rest.parse().ok().map(Self)
    }
}

impl fmt::Display for DocumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "doc:{}", self.0)
    }
}

impl fmt::Debug for DocumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DocumentId({self})")
    }
}

impl Serialize for DocumentId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serialize_prefixed_uuid("doc", &self.0, serializer)
    }
}

impl<'de> Deserialize<'de> for DocumentId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let uuid = deserialize_prefixed_uuid("doc", deserializer)?;
        Ok(Self(uuid))
    }
}

/// Identity of a session, stable across save/load (`session:<uuid>`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(pub Uuid);

impl SessionId {
    /// `new`.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// `from_u128`.
    pub fn from_u128(n: u128) -> Self {
        Self(Uuid::from_u128(n))
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "session:{}", self.0)
    }
}

impl fmt::Debug for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SessionId({self})")
    }
}

impl Serialize for SessionId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serialize_prefixed_uuid("session", &self.0, serializer)
    }
}

impl<'de> Deserialize<'de> for SessionId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let uuid = deserialize_prefixed_uuid("session", deserializer)?;
        Ok(Self(uuid))
    }
}

/// What a session document tab points at on disk.
#[derive(Clone, Debug)]
pub enum SessionDocumentTarget {
    /// Audio source file bound to a composition identity.
    Media {
        /// Stable media identity.
        media_id: MediaId,
        /// Composition this tab edits.
        composition_id: CompositionId,
        /// Resolved source path for the audio file.
        source_path: PathBuf,
        /// Last known descriptor for session persistence (updated on open/reload).
        descriptor: MediaDescriptor,
    },
    /// Standalone `.facomp` project.
    Composition {
        /// Composition identity for this project.
        composition_id: CompositionId,
        /// Path to the `.facomp` file.
        project_path: PathBuf,
    },
}

/// Session metadata for one open composition. View entities live in AppView.
#[derive(Clone, Debug)]
pub struct SessionDocument {
    /// id.
    pub id: DocumentId,
    /// Open target (media file or composition project).
    pub target: SessionDocumentTarget,
    /// Optional display name override. When unset, defaults to the media
    /// basename or the `.facomp` file stem/basename.
    pub name: Option<String>,
    /// tab_open.
    pub tab_open: bool,
    /// Pinned tabs stay in the bar; transient tabs may be replaced.
    pub tab_pinned: bool,
    /// group.
    pub group: Option<String>,
    /// state.
    pub state: Option<String>,
    /// properties.
    pub properties: BTreeMap<String, String>,
}

impl SessionDocument {
    /// Open a media-backed document (legacy helper; prefer [`Self::new_media`]).
    pub fn new(id: DocumentId, source_path: Option<PathBuf>) -> Self {
        let path = source_path.unwrap_or_default();
        if is_facomp_path(&path) {
            Self::new_composition(id, path, CompositionId::new())
        } else {
            let descriptor = placeholder_descriptor(&path);
            Self::new_media(id, path, CompositionId::new(), descriptor)
        }
    }

    /// Open a media-backed tab.
    pub fn new_media(
        id: DocumentId,
        source_path: PathBuf,
        composition_id: CompositionId,
        descriptor: MediaDescriptor,
    ) -> Self {
        let media_id = descriptor.id;
        Self {
            id,
            target: SessionDocumentTarget::Media {
                media_id,
                composition_id,
                source_path,
                descriptor,
            },
            name: None,
            tab_open: true,
            tab_pinned: false,
            group: None,
            state: None,
            properties: BTreeMap::new(),
        }
    }

    /// Open a `.facomp` project tab.
    pub fn new_composition(
        id: DocumentId,
        project_path: PathBuf,
        composition_id: CompositionId,
    ) -> Self {
        Self {
            id,
            target: SessionDocumentTarget::Composition {
                composition_id,
                project_path,
            },
            name: None,
            tab_open: true,
            tab_pinned: false,
            group: None,
            state: None,
            properties: BTreeMap::new(),
        }
    }

    /// Primary on-disk path for save (project wins when both exist historically).
    pub fn file_path(&self) -> Option<&Path> {
        match &self.target {
            SessionDocumentTarget::Composition { project_path, .. }
                if !project_path.as_os_str().is_empty() =>
            {
                Some(project_path.as_path())
            }
            SessionDocumentTarget::Media { source_path, .. }
                if !source_path.as_os_str().is_empty() =>
            {
                Some(source_path.as_path())
            }
            _ => None,
        }
    }

    /// Explicit display-name override, if any.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Set or clear the display-name override (empty clears).
    pub fn set_name(&mut self, name: impl AsRef<str>) {
        let trimmed = name.as_ref().trim();
        self.name = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }

    /// Display name: explicit [`Self::name`], else media basename / composition
    /// file basename.
    pub fn effective_name(&self) -> String {
        if let Some(name) = self.name.as_ref().filter(|n| !n.is_empty()) {
            return name.clone();
        }
        match &self.target {
            SessionDocumentTarget::Media {
                descriptor,
                source_path,
                ..
            } => {
                if !descriptor.basename.is_empty() {
                    descriptor.basename.clone()
                } else {
                    basename_of(source_path)
                }
            }
            SessionDocumentTarget::Composition { project_path, .. } => basename_of(project_path),
        }
    }

    /// Composition identity for this tab.
    pub fn composition_id(&self) -> CompositionId {
        match self.target {
            SessionDocumentTarget::Media { composition_id, .. }
            | SessionDocumentTarget::Composition { composition_id, .. } => composition_id,
        }
    }

    /// True when the tab is backed by a media source file.
    pub fn is_media(&self) -> bool {
        matches!(self.target, SessionDocumentTarget::Media { .. })
    }

    /// Stable media id when this is a media-backed document.
    pub fn media_id(&self) -> Option<MediaId> {
        match self.target {
            SessionDocumentTarget::Media { media_id, .. } => Some(media_id),
            SessionDocumentTarget::Composition { .. } => None,
        }
    }

    /// True when the tab is a `.facomp` project.
    pub fn is_composition(&self) -> bool {
        matches!(self.target, SessionDocumentTarget::Composition { .. })
    }

    /// Resolved audio source path when this is a media tab.
    pub fn source_path(&self) -> Option<&Path> {
        match &self.target {
            SessionDocumentTarget::Media { source_path, .. } => Some(source_path.as_path()),
            _ => None,
        }
    }

    /// Project path when this is a composition tab.
    pub fn project_path(&self) -> Option<&Path> {
        match &self.target {
            SessionDocumentTarget::Composition { project_path, .. } => Some(project_path.as_path()),
            _ => None,
        }
    }

    /// Update the media source path (media tabs only).
    pub fn set_source_path(&mut self, path: PathBuf) {
        if let SessionDocumentTarget::Media {
            source_path,
            descriptor,
            ..
        } = &mut self.target
        {
            *source_path = path.clone();
            if descriptor.url.as_str().is_empty() || !descriptor.url.as_str().contains("://") {
                descriptor.url = Location::from_path(&path);
            }
            descriptor.basename = basename_of(&path);
        }
    }

    /// Set or upgrade to a project path (converts media tabs to composition).
    pub fn set_project_path(&mut self, path: PathBuf) {
        match &mut self.target {
            SessionDocumentTarget::Composition { project_path, .. } => *project_path = path,
            SessionDocumentTarget::Media { composition_id, .. } => {
                let composition_id = *composition_id;
                self.target = SessionDocumentTarget::Composition {
                    composition_id,
                    project_path: path,
                };
            }
        }
    }

    /// Replace the composition id (media or project tab).
    pub fn set_composition_id(&mut self, composition_id: CompositionId) {
        match &mut self.target {
            SessionDocumentTarget::Media {
                composition_id: id, ..
            }
            | SessionDocumentTarget::Composition {
                composition_id: id, ..
            } => *id = composition_id,
        }
    }

    /// Media descriptor when this is a media tab.
    pub fn media_descriptor(&self) -> Option<&MediaDescriptor> {
        match &self.target {
            SessionDocumentTarget::Media { descriptor, .. } => Some(descriptor),
            _ => None,
        }
    }

    /// Update the recorded media descriptor (media tabs only).
    pub fn set_media_descriptor(&mut self, descriptor: MediaDescriptor) {
        if let SessionDocumentTarget::Media {
            media_id,
            descriptor: stored,
            ..
        } = &mut self.target
        {
            *media_id = descriptor.id;
            *stored = descriptor;
        }
    }
}

/// Build a placeholder [`MediaDescriptor`] from a path (tests / untitled opens).
pub fn placeholder_media_descriptor(path: &Path) -> MediaDescriptor {
    placeholder_descriptor(path)
}

fn placeholder_descriptor(path: &Path) -> MediaDescriptor {
    use std::time::UNIX_EPOCH;
    let basename = basename_of(path);
    MediaDescriptor {
        id: MediaId::from_bytes([0u8; 32]),
        url: if path.as_os_str().is_empty() {
            Location::from("")
        } else {
            Location::from_path(path)
        },
        basename,
        sample_rate: 44_100,
        channel_count: 2,
        frame_count: 0,
        bits_per_sample: Some(16),
        size_bytes: 0,
        modified: UNIX_EPOCH,
        container_format: "unknown".into(),
        codec: "unknown".into(),
    }
    .with_computed_id()
}

fn basename_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

/// Ordered list of open compositions and which one is active.
#[derive(Clone, Debug)]
pub struct Session {
    id: SessionId,
    path: Option<PathBuf>,
    workflow: Option<String>,
    properties: BTreeMap<String, String>,
    capture_ui: bool,
    dirty: bool,
    /// Ordered named explorer groups (may be empty of documents).
    groups: Vec<String>,
    documents: Vec<SessionDocument>,
    active: Option<DocumentId>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// `new`.
    pub fn new() -> Self {
        Self {
            id: SessionId::new(),
            path: None,
            workflow: None,
            properties: BTreeMap::new(),
            capture_ui: true,
            dirty: false,
            groups: Vec::new(),
            documents: Vec::new(),
            active: None,
        }
    }

    /// `id`.
    pub fn id(&self) -> SessionId {
        self.id
    }

    /// `path`.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// `set_path`.
    pub fn set_path(&mut self, path: Option<PathBuf>) {
        if self.path != path {
            self.path = path;
            self.mark_dirty();
        }
    }

    /// `workflow`.
    pub fn workflow(&self) -> Option<&str> {
        self.workflow.as_deref()
    }

    /// `suggested_fasession_name`.
    pub fn suggested_fasession_name(&self) -> String {
        let stem = self
            .workflow()
            .filter(|name| !name.is_empty())
            .unwrap_or("session");
        format!("{stem}.fasession")
    }

    /// `set_workflow`.
    pub fn set_workflow(&mut self, workflow: Option<String>) {
        let workflow = workflow.filter(|name| !name.is_empty());
        if self.workflow != workflow {
            self.workflow = workflow;
            self.mark_dirty();
        }
    }

    /// `properties`.
    pub fn properties(&self) -> &BTreeMap<String, String> {
        &self.properties
    }

    /// `set_properties`.
    pub fn set_properties(&mut self, properties: BTreeMap<String, String>) {
        if self.properties != properties {
            self.properties = properties;
            self.mark_dirty();
        }
    }

    /// `capture_ui`.
    pub fn capture_ui(&self) -> bool {
        self.capture_ui
    }

    /// `set_capture_ui`.
    pub fn set_capture_ui(&mut self, capture_ui: bool) {
        if self.capture_ui != capture_ui {
            self.capture_ui = capture_ui;
            self.mark_dirty();
        }
    }

    /// `is_dirty`.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Untitled sessions exist from launch until the user saves or loads one.
    /// Those should not prompt on quit or replace, even if modified.
    pub fn should_prompt_save(&self) -> bool {
        self.dirty && self.path.is_some()
    }

    /// `mark_dirty`.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// `mark_clean`.
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// `documents`.
    pub fn documents(&self) -> &[SessionDocument] {
        &self.documents
    }

    /// `take_documents`.
    pub fn take_documents(&mut self) -> Vec<SessionDocument> {
        std::mem::take(&mut self.documents)
    }

    /// `len`.
    pub fn len(&self) -> usize {
        self.documents.len()
    }

    /// `is_empty`.
    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }

    /// `tab_open_count`.
    pub fn tab_open_count(&self) -> usize {
        self.documents.iter().filter(|doc| doc.tab_open).count()
    }

    /// `group_count`.
    pub fn group_count(&self, group: &str) -> usize {
        self.documents
            .iter()
            .filter(|doc| doc.group.as_deref() == Some(group))
            .count()
    }

    /// `active`.
    pub fn active(&self) -> Option<DocumentId> {
        self.active
    }

    /// `get`.
    pub fn get(&self, id: DocumentId) -> Option<&SessionDocument> {
        self.documents.iter().find(|doc| doc.id == id)
    }

    /// `get_mut`.
    pub fn get_mut(&mut self, id: DocumentId) -> Option<&mut SessionDocument> {
        self.documents.iter_mut().find(|doc| doc.id == id)
    }

    /// `find_by_path`.
    pub fn find_by_path(&self, path: &Path) -> Option<DocumentId> {
        self.documents.iter().find_map(|doc| {
            let matches_source = doc
                .source_path()
                .is_some_and(|source| paths_equivalent(source, path));
            let matches_project = doc
                .project_path()
                .is_some_and(|project| paths_equivalent(project, path));
            (matches_source || matches_project).then_some(doc.id)
        })
    }

    /// Insert a document, make it active, and open a center tab.
    pub fn push(&mut self, source_path: Option<PathBuf>) -> DocumentId {
        let path = source_path.unwrap_or_default();
        if is_facomp_path(&path) {
            self.insert(SessionDocument::new_composition(
                DocumentId::new(),
                path,
                CompositionId::new(),
            ))
        } else {
            self.insert(SessionDocument::new(DocumentId::new(), Some(path)))
        }
    }

    /// Insert an unsaved composition document that already has a stable id
    /// (break-out children). The path is filled in on first save.
    pub fn push_untitled_composition(&mut self, composition_id: CompositionId) -> DocumentId {
        self.insert(SessionDocument::new_composition(
            DocumentId::new(),
            PathBuf::new(),
            composition_id,
        ))
    }

    /// `insert`.
    pub fn insert(&mut self, doc: SessionDocument) -> DocumentId {
        let id = doc.id;
        self.documents.push(doc);
        self.active = Some(id);
        self.mark_dirty();
        id
    }

    /// Set the active document. Returns the previous id when it changed.
    pub fn focus(&mut self, id: DocumentId) -> Option<DocumentId> {
        if self.get(id).is_none() {
            return None;
        }
        let previous = self.active;
        if previous == Some(id) {
            return None;
        }
        self.active = Some(id);
        self.mark_dirty();
        previous
    }

    /// `ensure_tab`.
    pub fn ensure_tab(&mut self, id: DocumentId) -> bool {
        let changed = match self.get_mut(id) {
            Some(doc) if !doc.tab_open => {
                doc.tab_open = true;
                true
            }
            _ => false,
        };
        if changed {
            self.mark_dirty();
        }
        changed
    }

    /// `close_tab`.
    pub fn close_tab(&mut self, id: DocumentId) -> bool {
        let changed = match self.get_mut(id) {
            Some(doc) if doc.tab_open => {
                doc.tab_open = false;
                doc.tab_pinned = false;
                true
            }
            _ => false,
        };
        if changed {
            self.mark_dirty();
        }
        changed
    }

    /// `set_tab_open`.
    pub fn set_tab_open(&mut self, id: DocumentId, open: bool) {
        let changed = match self.get_mut(id) {
            Some(doc) if doc.tab_open != open => {
                doc.tab_open = open;
                if !open {
                    doc.tab_pinned = false;
                }
                true
            }
            _ => false,
        };
        if changed {
            self.mark_dirty();
        }
    }

    /// Pin an open tab. Returns false if the document has no tab.
    pub fn pin_tab(&mut self, id: DocumentId) -> bool {
        let changed = match self.get_mut(id) {
            Some(doc) if doc.tab_open && !doc.tab_pinned => {
                doc.tab_pinned = true;
                true
            }
            _ => false,
        };
        if changed {
            self.mark_dirty();
        }
        changed
    }

    /// `set_project_path`.
    pub fn set_project_path(&mut self, id: DocumentId, path: PathBuf) {
        let changed = match self.get_mut(id) {
            Some(doc) if doc.project_path() != Some(path.as_path()) => {
                doc.set_project_path(path);
                true
            }
            _ => false,
        };
        if changed {
            self.mark_dirty();
        }
    }

    /// Record the composition identity for a session document.
    pub fn set_composition_id(&mut self, id: DocumentId, composition_id: CompositionId) {
        if let Some(doc) = self.get_mut(id) {
            if doc.composition_id() != composition_id {
                doc.set_composition_id(composition_id);
                self.mark_dirty();
            }
        }
    }

    /// Update the media descriptor for a media-backed document.
    pub fn set_media_descriptor(&mut self, id: DocumentId, descriptor: MediaDescriptor) {
        if let Some(doc) = self.get_mut(id) {
            if doc.is_media() {
                doc.set_media_descriptor(descriptor);
                self.mark_dirty();
            }
        }
    }

    /// Point an existing document at a new audio or `.facomp` file.
    pub fn replace_document_path(&mut self, id: DocumentId, path: PathBuf) -> bool {
        let Some(doc) = self.get_mut(id) else {
            return false;
        };
        if is_facomp_path(&path) {
            doc.set_project_path(path);
        } else if doc.is_media() {
            doc.set_source_path(path);
        } else {
            doc.set_project_path(path);
        }
        self.mark_dirty();
        true
    }

    /// `set_document_group`.
    pub fn set_document_group(&mut self, id: DocumentId, group: Option<String>) -> bool {
        let group = group.filter(|name| !name.is_empty());
        if let Some(name) = group.as_ref() {
            self.ensure_group(name);
        }
        let changed = match self.get_mut(id) {
            Some(doc) if doc.group != group => {
                doc.group = group;
                true
            }
            Some(_) => return true,
            None => return false,
        };
        if changed {
            self.mark_dirty();
        }
        true
    }

    /// Ordered named groups in the session registry (may be empty of members).
    pub fn groups(&self) -> &[String] {
        &self.groups
    }

    /// Ensure `name` appears in the group registry (append if missing).
    pub fn ensure_group(&mut self, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        if self.groups.iter().any(|g| g == name) {
            return false;
        }
        self.groups.push(name.to_string());
        self.mark_dirty();
        true
    }

    /// Append a named group. No-op for empty or duplicate names. Returns true
    /// when the registry changed.
    pub fn add_group(&mut self, name: impl AsRef<str>) -> bool {
        self.ensure_group(name.as_ref())
    }

    /// Insert a named group after `after` (or at the end when `after` is None /
    /// unknown). No-op for empty or duplicate names.
    pub fn add_group_after(&mut self, name: impl AsRef<str>, after: Option<&str>) -> bool {
        let name = name.as_ref().trim();
        if name.is_empty() || self.groups.iter().any(|g| g == name) {
            return false;
        }
        let index = after
            .and_then(|a| self.groups.iter().position(|g| g == a))
            .map(|i| i + 1)
            .unwrap_or(self.groups.len());
        self.groups.insert(index, name.to_string());
        self.mark_dirty();
        true
    }

    /// Rename a registry group and all member documents. Returns false if
    /// `old` is missing or `new` is empty / already used.
    pub fn rename_group(&mut self, old: &str, new: impl AsRef<str>) -> bool {
        let new = new.as_ref().trim();
        if new.is_empty() {
            return false;
        }
        let Some(index) = self.groups.iter().position(|g| g == old) else {
            return false;
        };
        if old == new {
            return true;
        }
        if self.groups.iter().any(|g| g == new) {
            return false;
        }
        self.groups[index] = new.to_string();
        for doc in &mut self.documents {
            if doc.group.as_deref() == Some(old) {
                doc.group = Some(new.to_string());
            }
        }
        self.mark_dirty();
        true
    }

    /// Remove a registry group and clear membership on documents.
    pub fn delete_group(&mut self, name: &str) -> bool {
        let Some(index) = self.groups.iter().position(|g| g == name) else {
            return false;
        };
        self.groups.remove(index);
        for doc in &mut self.documents {
            if doc.group.as_deref() == Some(name) {
                doc.group = None;
            }
        }
        self.mark_dirty();
        true
    }

    /// Move a named group to `index` within the registry (0-based, clamped).
    pub fn move_group(&mut self, name: &str, index: usize) -> bool {
        let Some(from) = self.groups.iter().position(|g| g == name) else {
            return false;
        };
        let group = self.groups.remove(from);
        let to = index.min(self.groups.len());
        if to == from {
            self.groups.insert(to, group);
            return true;
        }
        self.groups.insert(to, group);
        self.mark_dirty();
        true
    }

    /// Replace the group registry. Documents whose group is not in `names`
    /// become ungrouped. Empty names are skipped; duplicates keep first.
    pub fn set_groups(&mut self, names: Vec<String>) -> bool {
        let mut groups = Vec::new();
        for name in names {
            let name = name.trim();
            if name.is_empty() || groups.iter().any(|g| g == name) {
                continue;
            }
            groups.push(name.to_string());
        }
        let mut docs_changed = false;
        for doc in &mut self.documents {
            if let Some(group) = doc.group.as_ref() {
                if !groups.iter().any(|g| g == group) {
                    doc.group = None;
                    docs_changed = true;
                }
            }
        }
        if self.groups == groups && !docs_changed {
            return true;
        }
        self.groups = groups;
        self.mark_dirty();
        true
    }

    /// Move `id` to `index` in the full document list (0-based, clamped to
    /// `0..=len` after removal). Does not change `group`. Returns false if
    /// the document is missing. Marks dirty only when order changes.
    pub fn move_document(&mut self, id: DocumentId, index: usize) -> bool {
        let Some(from) = self.documents.iter().position(|doc| doc.id == id) else {
            return false;
        };
        let doc = self.documents.remove(from);
        let to = index.min(self.documents.len());
        if to == from {
            self.documents.insert(to, doc);
            return true;
        }
        self.documents.insert(to, doc);
        self.mark_dirty();
        true
    }

    /// Set `group` and insert `id` so it becomes the `index_in_group`-th
    /// member of that group (0-based; values past the end append). Empty
    /// group names become ungrouped (`None`). Marks dirty when group or
    /// order changes. Returns false if the document is missing.
    pub fn place_document(
        &mut self,
        id: DocumentId,
        group: Option<String>,
        index_in_group: usize,
    ) -> bool {
        let group = group.filter(|name| !name.is_empty());
        let Some(from) = self.documents.iter().position(|doc| doc.id == id) else {
            return false;
        };
        if let Some(name) = group.as_ref() {
            self.ensure_group(name);
        }
        let mut doc = self.documents.remove(from);
        let group_changed = doc.group != group;
        doc.group = group.clone();

        let members: Vec<usize> = self
            .documents
            .iter()
            .enumerate()
            .filter(|(_, d)| d.group == group)
            .map(|(ix, _)| ix)
            .collect();
        let to = if index_in_group < members.len() {
            members[index_in_group]
        } else if let Some(&last) = members.last() {
            last + 1
        } else {
            self.documents.len()
        };

        if !group_changed && to == from {
            self.documents.insert(to, doc);
            return true;
        }
        self.documents.insert(to, doc);
        self.mark_dirty();
        true
    }

    /// `set_document_state`.
    pub fn set_document_state(&mut self, id: DocumentId, state: Option<String>) -> bool {
        let state = state.filter(|name| !name.is_empty());
        let changed = match self.get_mut(id) {
            Some(doc) if doc.state != state => {
                doc.state = state;
                true
            }
            Some(_) => return true,
            None => return false,
        };
        if changed {
            self.mark_dirty();
        }
        true
    }

    /// `set_document_properties`.
    pub fn set_document_properties(
        &mut self,
        id: DocumentId,
        properties: BTreeMap<String, String>,
    ) -> bool {
        let changed = match self.get_mut(id) {
            Some(doc) if doc.properties != properties => {
                doc.properties = properties;
                true
            }
            Some(_) => return true,
            None => return false,
        };
        if changed {
            self.mark_dirty();
        }
        true
    }

    /// `tab_pinned`.
    pub fn tab_pinned(&self, id: DocumentId) -> bool {
        self.get(id)
            .is_some_and(|doc| doc.tab_open && doc.tab_pinned)
    }

    /// First open unpinned tab in `order` (typically center tab-bar order).
    pub fn first_transient_in(&self, order: &[DocumentId]) -> Option<DocumentId> {
        order.iter().copied().find(|&id| {
            self.get(id)
                .is_some_and(|doc| doc.tab_open && !doc.tab_pinned)
        })
    }

    /// `open_tab_ids`.
    pub fn open_tab_ids(&self) -> Vec<DocumentId> {
        self.documents
            .iter()
            .filter(|doc| doc.tab_open)
            .map(|doc| doc.id)
            .collect()
    }

    /// Drop the document from the session. If it was active, focus another.
    pub fn close_document(&mut self, id: DocumentId) -> bool {
        let Some(ix) = self.documents.iter().position(|doc| doc.id == id) else {
            return false;
        };
        self.documents.remove(ix);
        if self.active == Some(id) {
            self.active = self
                .documents
                .get(ix)
                .or_else(|| self.documents.last())
                .map(|doc| doc.id);
        }
        self.mark_dirty();
        true
    }

    /// `to_json`.
    pub fn to_json(&self, name_of: impl Fn(DocumentId) -> String) -> Result<String> {
        self.to_envelope(name_of, None)?.to_json()
    }

    /// `to_json_at`.
    pub fn to_json_at(
        &self,
        session_path: &Path,
        name_of: impl Fn(DocumentId) -> String,
        ui: Option<SessionUi>,
    ) -> Result<String> {
        let base = session_path.parent();
        let mut envelope = self.to_envelope(name_of, base)?;
        if self.capture_ui {
            envelope.ui = ui;
        } else {
            envelope.ui = None;
        }
        envelope.to_json()
    }

    fn to_envelope(
        &self,
        name_of: impl Fn(DocumentId) -> String,
        base: Option<&Path>,
    ) -> Result<SessionEnvelope> {
        let mut documents = Vec::with_capacity(self.documents.len());
        let mut media = Vec::new();
        let mut seen_media = HashSet::new();
        for doc in &self.documents {
            let Some(_path) = doc.file_path() else {
                bail!("cannot save session: {} has no file URL", name_of(doc.id));
            };
            let target = match &doc.target {
                SessionDocumentTarget::Media {
                    media_id,
                    composition_id,
                    source_path,
                    descriptor,
                } => {
                    let mut descriptor = descriptor.clone();
                    // Keep document media_id aligned with the descriptor we persist.
                    let media_id = if *media_id == descriptor.id {
                        *media_id
                    } else {
                        descriptor.id
                    };
                    descriptor.url = match base {
                        Some(base) => Location::from_path_relative_to(source_path, base),
                        None => Location::from_path(source_path),
                    };
                    if seen_media.insert(media_id) {
                        media.push(descriptor.clone());
                    }
                    SessionDocumentTargetFile::Media {
                        media_id,
                        composition_id: *composition_id,
                    }
                }
                SessionDocumentTarget::Composition {
                    composition_id,
                    project_path,
                } => SessionDocumentTargetFile::Composition {
                    composition_id: *composition_id,
                    url: match base {
                        Some(base) => Location::from_path_relative_to(project_path, base),
                        None => Location::from_path(project_path),
                    }
                    .to_string(),
                },
            };
            let name = doc.name.clone().filter(|n| !n.is_empty());
            documents.push(SessionDocumentFile {
                id: doc.id,
                name,
                target,
                group: doc.group.clone(),
                state: doc.state.clone(),
                properties: doc.properties.clone(),
                tab_open: doc.tab_open,
                tab_pinned: doc.tab_pinned,
            });
        }
        Ok(SessionEnvelope {
            kind: FASESSION_KIND.into(),
            format_version: FASESSION_FORMAT_VERSION,
            id: self.id,
            workflow: self.workflow.clone(),
            properties: self.properties.clone(),
            active: self.active,
            capture_ui: self.capture_ui,
            groups: self.groups.clone(),
            media,
            documents,
            ui: None,
        })
    }

    /// `from_json`.
    pub fn from_json(json: &str, session_path: Option<&Path>) -> Result<LoadedSession> {
        let envelope = SessionEnvelope::from_json(json)?;
        let base = session_path.and_then(|path| path.parent());
        let media_by_id: HashMap<MediaId, MediaDescriptor> = envelope
            .media
            .into_iter()
            .map(|descriptor| (descriptor.id, descriptor))
            .collect();
        let mut documents = Vec::with_capacity(envelope.documents.len());
        for file_doc in envelope.documents {
            let name = file_doc.name.filter(|n| !n.is_empty());
            let target = match file_doc.target {
                SessionDocumentTargetFile::Media {
                    media_id,
                    composition_id,
                } => {
                    let mut descriptor = media_by_id.get(&media_id).cloned().ok_or_else(|| {
                        anyhow::anyhow!("missing media descriptor for {}", media_id)
                    })?;
                    let path = descriptor.url.resolve_against_path(base).with_context(|| {
                        format!("invalid media URL {} for {}", descriptor.url, media_id)
                    })?;
                    if is_fasession_path(&path) {
                        bail!("session documents cannot be nested session files");
                    }
                    if is_facomp_path(&path) {
                        bail!("media document URL must not be a .facomp file");
                    }
                    if descriptor.basename.is_empty() {
                        descriptor.basename = basename_of(&path);
                    }
                    SessionDocumentTarget::Media {
                        media_id,
                        composition_id,
                        source_path: path,
                        descriptor,
                    }
                }
                SessionDocumentTargetFile::Composition {
                    composition_id,
                    url,
                } => {
                    let path = Location::parse(&url)?
                        .resolve_against_path(base)
                        .with_context(|| format!("invalid document URL {url}"))?;
                    if is_fasession_path(&path) {
                        bail!("session documents cannot be nested session files");
                    }
                    if !is_facomp_path(&path) {
                        bail!("composition document URL must be a .facomp file");
                    }
                    SessionDocumentTarget::Composition {
                        composition_id,
                        project_path: path,
                    }
                }
            };
            documents.push(SessionDocument {
                id: file_doc.id,
                target,
                name,
                tab_open: file_doc.tab_open,
                tab_pinned: file_doc.tab_pinned,
                group: file_doc.group,
                state: file_doc.state,
                properties: file_doc.properties,
            });
        }
        let active = envelope
            .active
            .filter(|id| documents.iter().any(|doc| doc.id == *id))
            .or_else(|| documents.first().map(|doc| doc.id));
        let mut groups = envelope.groups;
        for doc in &documents {
            if let Some(name) = doc.group.as_ref() {
                if !name.is_empty() && !groups.iter().any(|g| g == name) {
                    groups.push(name.clone());
                }
            }
        }
        Ok(LoadedSession {
            session: Session {
                id: envelope.id,
                path: session_path.map(Path::to_path_buf),
                workflow: envelope.workflow,
                properties: envelope.properties,
                capture_ui: envelope.capture_ui,
                dirty: false,
                groups,
                documents,
                active,
            },
            ui: envelope.ui,
        })
    }

    /// `save_to_path`.
    pub fn save_to_path(
        &mut self,
        path: &Path,
        name_of: impl Fn(DocumentId) -> String,
        ui: Option<SessionUi>,
    ) -> Result<()> {
        let json = self.to_json_at(path, name_of, ui)?;
        write_atomic(path, &json)?;
        self.path = Some(path.to_path_buf());
        self.mark_clean();
        Ok(())
    }
}

fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "session.fasession".into());
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
    })?;
    Ok(())
}

#[derive(Debug)]
/// LoadedSession.
pub struct LoadedSession {
    /// session.
    pub session: Session,
    /// ui.
    pub ui: Option<SessionUi>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// SessionUi.
pub struct SessionUi {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// window.
    pub window: Option<SessionWindowUi>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// docks.
    pub docks: Option<SessionDocksUi>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// SessionWindowUi.
pub struct SessionWindowUi {
    /// x.
    pub x: f32,
    /// y.
    pub y: f32,
    /// width.
    pub width: f32,
    /// height.
    pub height: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// SessionDocksUi.
pub struct SessionDocksUi {
    /// explorer.
    pub explorer: bool,
    /// detail.
    pub detail: bool,
    /// script.
    pub script: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionEnvelope {
    /// kind.
    pub kind: String,
    /// format_version.
    pub format_version: u32,
    /// id.
    pub id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// workflow.
    pub workflow: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    /// properties.
    pub properties: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// active.
    pub active: Option<DocumentId>,
    #[serde(default = "default_true")]
    /// capture_ui.
    pub capture_ui: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Ordered named explorer groups.
    pub groups: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Media descriptors for open media-backed documents.
    pub media: Vec<MediaDescriptor>,
    /// documents.
    pub documents: Vec<SessionDocumentFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// ui.
    pub ui: Option<SessionUi>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionDocumentFile {
    /// id.
    pub id: DocumentId,
    /// Optional display-name override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Tagged open target.
    pub target: SessionDocumentTargetFile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// group.
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// state.
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    /// properties.
    pub properties: BTreeMap<String, String>,
    #[serde(default = "default_true")]
    /// tab_open.
    pub tab_open: bool,
    #[serde(default)]
    /// tab_pinned.
    pub tab_pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SessionDocumentTargetFile {
    /// Media-backed tab (URL lives on the session `media` descriptor).
    Media {
        /// media_id.
        media_id: MediaId,
        /// composition_id.
        composition_id: CompositionId,
    },
    /// `.facomp` project tab.
    Composition {
        /// composition_id.
        composition_id: CompositionId,
        /// url.
        url: String,
    },
}

impl SessionEnvelope {
    fn from_json(json: &str) -> Result<Self> {
        let envelope: Self = serde_json::from_str(json).context("parse session JSON")?;
        if envelope.kind != FASESSION_KIND {
            bail!("not a FieldAssist session (kind {:?})", envelope.kind);
        }
        match envelope.format_version {
            2 => Ok(envelope),
            0 => bail!("missing or invalid format_version"),
            1 => bail!("unsupported format_version 1 (upgrade to format_version 2)"),
            n if n > FASESSION_FORMAT_VERSION => {
                bail!("this file requires a newer FieldAssist (format_version {n})")
            }
            n => bail!("unsupported format_version {n}"),
        }
    }

    fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("serialize session")
    }
}

/// `is_fasession_path`.
pub fn is_fasession_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("fasession"))
}

/// `paths_equivalent`.
pub fn paths_equivalent(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: &str) -> PathBuf {
        PathBuf::from(name)
    }

    fn named(_id: DocumentId) -> String {
        "doc".into()
    }

    #[test]
    fn open_two_paths_lists_both() {
        let mut session = Session::new();
        session.push(Some(path("a.wav")));
        session.push(Some(path("b.wav")));
        assert_eq!(session.len(), 2);
        assert_eq!(session.tab_open_count(), 2);
    }

    #[test]
    fn closing_one_tab_keeps_both_documents() {
        let mut session = Session::new();
        let a = session.push(Some(path("a.wav")));
        session.push(Some(path("b.wav")));
        assert!(session.close_tab(a));
        assert_eq!(session.len(), 2);
        assert_eq!(session.tab_open_count(), 1);
        assert!(!session.get(a).unwrap().tab_open);
    }

    #[test]
    fn ensure_tab_restores_a_closed_tab() {
        let mut session = Session::new();
        let a = session.push(Some(path("a.wav")));
        session.push(Some(path("b.wav")));
        session.close_tab(a);
        assert!(session.ensure_tab(a));
        assert!(session.get(a).unwrap().tab_open);
        assert_eq!(session.tab_open_count(), 2);
    }

    #[test]
    fn close_from_explorer_drops_document_and_tab() {
        let mut session = Session::new();
        let a = session.push(Some(path("a.wav")));
        let b = session.push(Some(path("b.wav")));
        assert!(session.close_document(a));
        assert_eq!(session.len(), 1);
        assert_eq!(session.tab_open_count(), 1);
        assert_eq!(session.active(), Some(b));
        assert!(session.get(a).is_none());
    }

    #[test]
    fn reopen_same_path_reuses_id() {
        let mut session = Session::new();
        let a = session.push(Some(path("a.wav")));
        session.push(Some(path("b.wav")));
        let found = session.find_by_path(Path::new("a.wav")).unwrap();
        assert_eq!(found, a);
        session.focus(found);
        session.ensure_tab(found);
        assert_eq!(session.len(), 2);
        assert_eq!(session.active(), Some(a));
    }

    #[test]
    fn find_by_path_matches_project_path() {
        let mut session = Session::new();
        let a = session.push(Some(path("a.wav")));
        session
            .get_mut(a)
            .unwrap()
            .set_project_path(path("a.wav.facomp"));
        assert_eq!(session.find_by_path(Path::new("a.wav.facomp")), Some(a));
    }

    #[test]
    fn tree_id_round_trips() {
        let id = DocumentId::from_u128(42);
        assert_eq!(DocumentId::from_tree_id(&id.to_tree_id()), Some(id));
        assert_eq!(DocumentId::from_tree_id("nope"), None);
    }

    #[test]
    fn new_tabs_are_transient() {
        let mut session = Session::new();
        let a = session.push(Some(path("a.wav")));
        assert!(!session.tab_pinned(a));
        assert!(session.get(a).unwrap().tab_open);
    }

    #[test]
    fn pin_then_close_clears_pin() {
        let mut session = Session::new();
        let a = session.push(Some(path("a.wav")));
        assert!(session.pin_tab(a));
        assert!(session.tab_pinned(a));
        assert!(session.close_tab(a));
        assert!(!session.tab_pinned(a));
        session.ensure_tab(a);
        assert!(!session.tab_pinned(a));
    }

    #[test]
    fn first_transient_skips_pinned_and_closed() {
        let mut session = Session::new();
        let a = session.push(Some(path("a.wav")));
        let b = session.push(Some(path("b.wav")));
        let c = session.push(Some(path("c.wav")));
        session.pin_tab(a);
        session.close_tab(b);
        assert_eq!(session.first_transient_in(&[a, b, c]), Some(c));
    }

    #[test]
    fn closing_saved_tabs_leaves_dirty_tabs_open() {
        let mut session = Session::new();
        let saved = session.push(Some(path("a.wav")));
        let dirty = session.push(Some(path("b.wav")));
        session.pin_tab(dirty);
        assert!(session.close_tab(saved));
        assert!(!session.get(saved).unwrap().tab_open);
        assert!(session.get(dirty).unwrap().tab_open);
        assert!(session.tab_pinned(dirty));
        assert_eq!(session.tab_open_count(), 1);
    }

    #[test]
    fn json_round_trip_uses_relative_url() {
        let dir = std::env::temp_dir().join("fa-session-json");
        let _ = std::fs::create_dir_all(&dir);
        let wav = dir.join("takes").join("a.wav");
        let _ = std::fs::create_dir_all(wav.parent().unwrap());
        std::fs::write(&wav, b"wav").unwrap();
        let session_path = dir.join("batch.fasession");

        let mut session = Session::new();
        session.id = SessionId::from_u128(1);
        let id = DocumentId::from_u128(2);
        let composition_id = CompositionId::from_u128(3);
        let descriptor = placeholder_descriptor(&wav);
        session.insert(SessionDocument {
            id,
            target: SessionDocumentTarget::Media {
                media_id: descriptor.id,
                composition_id,
                source_path: wav.clone(),
                descriptor,
            },
            name: None,
            tab_open: true,
            tab_pinned: true,
            group: Some("day1".into()),
            state: Some("reviewed".into()),
            properties: BTreeMap::from([("reviewer".into(), "greg".into())]),
        });
        session.set_workflow(Some("review".into()));
        session.set_capture_ui(false);
        let json = session.to_json_at(&session_path, named, None).unwrap();
        assert!(json.contains("takes/a.wav"), "{json}");
        assert!(!json.contains("\"ui\""), "{json}");

        let loaded = Session::from_json(&json, Some(&session_path)).unwrap();
        assert_eq!(loaded.session.id(), SessionId::from_u128(1));
        assert_eq!(loaded.session.workflow(), Some("review"));
        let doc = loaded.session.get(id).unwrap();
        assert_eq!(doc.group.as_deref(), Some("day1"));
        assert_eq!(doc.state.as_deref(), Some("reviewed"));
        assert_eq!(
            doc.properties.get("reviewer").map(String::as_str),
            Some("greg")
        );
        assert!(doc.tab_pinned);
        assert_eq!(doc.source_path(), Some(wav.as_path()));
        let _ = std::fs::remove_file(&wav);
        let _ = std::fs::remove_file(&session_path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_fails_without_document_path() {
        let mut session = Session::new();
        session.push(None);
        let err = session.to_json(named).unwrap_err().to_string();
        assert!(err.contains("no file URL"), "{err}");
    }

    #[test]
    fn uuid_stays_stable_across_json() {
        let id = DocumentId::from_u128(99);
        let json = serde_json::to_string(&id).unwrap();
        let parsed: DocumentId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, id);
        assert_eq!(id.to_string(), "doc:00000000-0000-0000-0000-000000000063");
    }

    #[test]
    fn session_id_serializes_with_prefix() {
        let id = SessionId::from_u128(7);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"session:00000000-0000-0000-0000-000000000007\"");
    }

    #[test]
    fn format_version_1_is_rejected() {
        let err = Session::from_json(
            r#"{"kind":"fasession","format_version":1,"id":"session:00000000-0000-0000-0000-000000000001","documents":[]}"#,
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("format_version 1"), "{err}");
    }

    #[test]
    fn media_only_session_round_trips_three_files() {
        let dir = std::env::temp_dir().join("fa-session-media-only-rt");
        let _ = std::fs::create_dir_all(&dir);
        let paths = [
            dir.join("one.wav"),
            dir.join("two.wav"),
            dir.join("three.wav"),
        ];
        for (i, path) in paths.iter().enumerate() {
            std::fs::write(path, format!("wav{i}")).unwrap();
        }
        let session_path = dir.join("batch.fasession");

        let mut session = Session::new();
        let mut expected = Vec::new();
        for (i, path) in paths.iter().enumerate() {
            let doc_id = DocumentId::from_u128(100 + i as u128);
            let comp_id = CompositionId::from_u128(200 + i as u128);
            let mut descriptor = placeholder_descriptor(path);
            descriptor.frame_count = 1000 + i as u64;
            descriptor.size_bytes = 10 + i as u64;
            descriptor.sample_rate = 48_000;
            descriptor = descriptor.with_computed_id();
            expected.push((doc_id, comp_id, path.clone(), descriptor.id));
            session.insert(SessionDocument::new_media(
                doc_id,
                path.clone(),
                comp_id,
                descriptor,
            ));
        }

        let json = session.to_json_at(&session_path, named, None).unwrap();
        assert!(json.contains("\"format_version\": 2"), "{json}");
        // Media URLs live only on the top-level media descriptors.
        assert!(json.contains("\"url\": \"one.wav\""), "{json}");
        assert!(json.contains("\"url\": \"two.wav\""), "{json}");
        assert!(json.contains("\"url\": \"three.wav\""), "{json}");
        assert!(json.contains("\"media\""), "{json}");
        // Documents targeting media must not repeat url.
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        for doc in parsed["documents"].as_array().unwrap() {
            let target = &doc["target"];
            if target["type"] == "media" {
                assert!(target.get("url").is_none(), "{doc}");
                assert!(doc.get("name").is_none() || doc["name"].is_null(), "{doc}");
            }
        }

        let mut loaded = Session::from_json(&json, Some(&session_path)).unwrap();
        assert_eq!(loaded.session.documents().len(), 3);
        for (doc_id, comp_id, path, media_id) in &expected {
            let doc = loaded.session.get(*doc_id).expect("document present");
            assert!(doc.is_media(), "{doc_id}");
            assert_eq!(doc.composition_id(), *comp_id);
            assert_eq!(doc.source_path(), Some(path.as_path()));
            assert!(
                path.is_file(),
                "resolved media path missing: {}",
                path.display()
            );
            let descriptor = doc.media_descriptor().expect("descriptor");
            assert_eq!(descriptor.id, *media_id);
            // Descriptor URL stays relative; source_path is absolute beside session.
            assert_eq!(
                descriptor.url.as_str(),
                path.file_name().unwrap().to_string_lossy().as_ref()
            );
            assert_eq!(doc.name(), None);
            assert_eq!(
                doc.effective_name(),
                path.file_name().unwrap().to_string_lossy()
            );
        }

        // Explicit name override round-trips; omission still defaults to basename.
        loaded
            .session
            .get_mut(expected[0].0)
            .unwrap()
            .set_name("Take One");
        let json_named = loaded
            .session
            .to_json_at(&session_path, named, None)
            .unwrap();
        assert!(
            json_named.contains("\"name\": \"Take One\""),
            "{json_named}"
        );
        let loaded_named = Session::from_json(&json_named, Some(&session_path)).unwrap();
        assert_eq!(
            loaded_named
                .session
                .get(expected[0].0)
                .unwrap()
                .effective_name(),
            "Take One"
        );
        assert_eq!(
            loaded_named
                .session
                .get(expected[1].0)
                .unwrap()
                .effective_name(),
            "two.wav"
        );

        // Re-serialize from the loaded session and load again (true persistence loop).
        let json2 = loaded_named
            .session
            .to_json_at(&session_path, named, None)
            .unwrap();
        let loaded2 = Session::from_json(&json2, Some(&session_path)).unwrap();
        assert_eq!(loaded2.session.documents().len(), 3);
        for (doc_id, comp_id, path, media_id) in &expected {
            let doc = loaded2.session.get(*doc_id).unwrap();
            assert_eq!(doc.composition_id(), *comp_id);
            assert_eq!(doc.source_path(), Some(path.as_path()));
            assert_eq!(doc.media_descriptor().unwrap().id, *media_id);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn format_version_2_round_trips_media_and_composition() {
        let dir = std::env::temp_dir().join("fa-session-v2");
        let _ = std::fs::create_dir_all(&dir);
        let wav = dir.join("clip.wav");
        std::fs::write(&wav, b"wav").unwrap();
        let facomp = dir.join("edit.facomp");
        std::fs::write(&facomp, b"proj").unwrap();
        let session_path = dir.join("mix.fasession");

        let media_comp = CompositionId::from_u128(10);
        let project_comp = CompositionId::from_u128(11);
        let media_id = DocumentId::from_u128(20);
        let project_id = DocumentId::from_u128(21);
        let descriptor = placeholder_descriptor(&wav);

        let mut session = Session::new();
        session.insert(SessionDocument::new_media(
            media_id,
            wav.clone(),
            media_comp,
            descriptor,
        ));
        session.insert(SessionDocument::new_composition(
            project_id,
            facomp.clone(),
            project_comp,
        ));

        let json = session.to_json_at(&session_path, named, None).unwrap();
        assert!(json.contains("\"format_version\": 2"), "{json}");
        assert!(json.contains("\"type\": \"media\""), "{json}");
        assert!(json.contains("\"type\": \"composition\""), "{json}");
        assert!(json.contains("\"media\""), "{json}");

        let loaded = Session::from_json(&json, Some(&session_path)).unwrap();
        let media_doc = loaded.session.get(media_id).unwrap();
        assert!(media_doc.is_media());
        assert_eq!(media_doc.composition_id(), media_comp);
        assert_eq!(media_doc.source_path(), Some(wav.as_path()));

        let project_doc = loaded.session.get(project_id).unwrap();
        assert!(project_doc.is_composition());
        assert_eq!(project_doc.composition_id(), project_comp);
        assert_eq!(project_doc.project_path(), Some(facomp.as_path()));
        assert_eq!(project_doc.effective_name(), "edit.facomp");
        assert_eq!(media_doc.effective_name(), "clip.wav");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn media_document_composition_id_stable_across_session_reload() {
        // Ephemeral media tabs mint a composition id that children may parent
        // to; session save/load must keep that id so lineage still matches.
        let dir = std::env::temp_dir().join("fa-session-stable-comp-id");
        let _ = std::fs::create_dir_all(&dir);
        let wav = dir.join("parent.wav");
        std::fs::write(&wav, b"RIFF").unwrap();
        let session_path = dir.join("batch.fasession");

        let recorded = CompositionId::from_u128(0xAABB_CCDD);
        let mut descriptor = placeholder_descriptor(&wav);
        descriptor.frame_count = 100;
        descriptor.size_bytes = 4;
        descriptor = descriptor.with_computed_id();

        let mut session = Session::new();
        let parent_doc = DocumentId::from_u128(1);
        session.insert(SessionDocument::new_media(
            parent_doc,
            wav.clone(),
            recorded,
            descriptor,
        ));
        let child_doc = DocumentId::from_u128(2);
        let child_comp = CompositionId::from_u128(0x1122);
        let facomp = dir.join("child.facomp");
        std::fs::write(&facomp, b"{}").unwrap();
        session.insert(SessionDocument::new_composition(
            child_doc,
            facomp.clone(),
            child_comp,
        ));

        let json = session.to_json_at(&session_path, named, None).unwrap();
        let loaded = Session::from_json(&json, Some(&session_path)).unwrap();
        assert_eq!(
            loaded.session.get(parent_doc).unwrap().composition_id(),
            recorded
        );
        assert_eq!(
            loaded.session.get(child_doc).unwrap().composition_id(),
            child_comp
        );
        // Child lineage targets the recorded parent composition id.
        assert_ne!(recorded, child_comp);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn untitled_composition_keeps_recorded_id_when_path_is_assigned() {
        let mut session = Session::new();
        let composition_id = CompositionId::from_u128(0xCAFE);
        let id = session.push_untitled_composition(composition_id);
        let doc = session.get(id).unwrap();
        assert!(doc.is_composition());
        assert_eq!(doc.composition_id(), composition_id);
        assert!(doc.file_path().is_none());

        session.set_project_path(id, path("1-take.facomp"));
        let doc = session.get(id).unwrap();
        assert!(doc.is_composition());
        assert_eq!(doc.composition_id(), composition_id);
        assert_eq!(doc.project_path(), Some(Path::new("1-take.facomp")));
    }

    #[test]
    fn untitled_media_converted_to_composition_keeps_placeholder_id() {
        let mut session = Session::new();
        let id = session.push(None);
        let placeholder = session.get(id).unwrap().composition_id();
        assert!(session.get(id).unwrap().is_media());
        session.set_project_path(id, path("child.facomp"));
        let doc = session.get(id).unwrap();
        assert!(doc.is_composition());
        assert_eq!(doc.composition_id(), placeholder);
    }

    #[test]
    fn untitled_dirty_session_does_not_prompt_save() {
        let mut session = Session::new();
        assert!(!session.should_prompt_save());
        session.push(Some(path("a.wav")));
        assert!(session.is_dirty());
        assert!(!session.should_prompt_save());
    }

    #[test]
    fn saved_dirty_session_prompts_save() {
        let dir = std::env::temp_dir().join("fa-session-prompt");
        let _ = std::fs::create_dir_all(&dir);
        let wav = dir.join("a.wav");
        std::fs::write(&wav, b"wav").unwrap();
        let session_path = dir.join("batch.fasession");

        let mut session = Session::new();
        session.push(Some(wav));
        session.save_to_path(&session_path, named, None).unwrap();
        assert!(!session.should_prompt_save());
        session.push(Some(path("b.wav")));
        assert!(session.should_prompt_save());

        let _ = std::fs::remove_file(&session_path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn suggested_fasession_name_uses_workflow_or_session() {
        let mut session = Session::new();
        assert_eq!(session.suggested_fasession_name(), "session.fasession");
        session.set_workflow(Some("review".into()));
        assert_eq!(session.suggested_fasession_name(), "review.fasession");
        session.set_workflow(Some(String::new()));
        assert_eq!(session.suggested_fasession_name(), "session.fasession");
    }

    #[test]
    fn is_fasession_detects_extension() {
        assert!(is_fasession_path(Path::new("batch.fasession")));
        assert!(is_fasession_path(Path::new("BATCH.FASESSION")));
        assert!(!is_fasession_path(Path::new("take.wav")));
    }

    fn ids(session: &Session) -> Vec<DocumentId> {
        session.documents().iter().map(|doc| doc.id).collect()
    }

    #[test]
    fn move_document_reorders_full_list() {
        let mut session = Session::new();
        let a = session.push(Some(path("a.wav")));
        let b = session.push(Some(path("b.wav")));
        let c = session.push(Some(path("c.wav")));
        session.mark_clean();
        assert!(session.move_document(c, 0));
        assert_eq!(ids(&session), vec![c, a, b]);
        assert!(session.is_dirty());
        session.mark_clean();
        assert!(session.move_document(c, 0));
        assert!(!session.is_dirty());
        assert_eq!(ids(&session), vec![c, a, b]);
    }

    #[test]
    fn move_document_clamps_and_rejects_missing() {
        let mut session = Session::new();
        let a = session.push(Some(path("a.wav")));
        let b = session.push(Some(path("b.wav")));
        assert!(session.move_document(a, 99));
        assert_eq!(ids(&session), vec![b, a]);
        assert!(!session.move_document(DocumentId::from_u128(999), 0));
    }

    #[test]
    fn place_document_moves_across_groups_without_scrambling() {
        let mut session = Session::new();
        let a = session.push(Some(path("a.wav")));
        let b = session.push(Some(path("b.wav")));
        let c = session.push(Some(path("c.wav")));
        let d = session.push(Some(path("d.wav")));
        session.set_document_group(a, Some("todo".into()));
        session.set_document_group(b, Some("todo".into()));
        session.set_document_group(c, Some("drop".into()));
        session.set_document_group(d, Some("drop".into()));
        session.mark_clean();

        assert!(session.place_document(d, Some("todo".into()), 1));
        assert_eq!(ids(&session), vec![a, d, b, c]);
        assert_eq!(session.get(d).unwrap().group.as_deref(), Some("todo"));
        assert!(session.is_dirty());

        session.mark_clean();
        assert!(session.place_document(a, Some("drop".into()), 0));
        assert_eq!(ids(&session), vec![d, b, a, c]);
        assert_eq!(session.get(a).unwrap().group.as_deref(), Some("drop"));
        assert_eq!(session.get(c).unwrap().group.as_deref(), Some("drop"));
    }

    #[test]
    fn place_document_appends_and_prepends_in_group() {
        let mut session = Session::new();
        let a = session.push(Some(path("a.wav")));
        let b = session.push(Some(path("b.wav")));
        let c = session.push(Some(path("c.wav")));
        session.set_document_group(a, Some("todo".into()));
        session.set_document_group(b, Some("todo".into()));

        assert!(session.place_document(c, Some("todo".into()), 99));
        assert_eq!(ids(&session), vec![a, b, c]);
        assert!(session.place_document(c, Some("todo".into()), 0));
        assert_eq!(ids(&session), vec![c, a, b]);
        // Empty target group inserts at the end of the full list.
        assert!(session.place_document(b, None, 0));
        assert_eq!(ids(&session), vec![c, a, b]);
        assert!(session.get(b).unwrap().group.is_none());
    }

    #[test]
    fn place_document_reorders_within_group_to_end() {
        let mut session = Session::new();
        let a = session.push(Some(path("a.wav")));
        let b = session.push(Some(path("b.wav")));
        let c = session.push(Some(path("c.wav")));
        session.mark_clean();

        // Same pipeline as explorer: visual gap after last row (drop_before =
        // len) adjusted for removing the dragged row.
        let drop_before = 3;
        let from = 0;
        let index = if drop_before > from {
            drop_before - 1
        } else {
            drop_before
        };
        assert_eq!(index, 2);
        assert!(session.place_document(a, None, index));
        assert_eq!(ids(&session), vec![b, c, a]);
        assert!(session.is_dirty());

        session.mark_clean();
        assert!(session.place_document(b, None, 2));
        assert_eq!(ids(&session), vec![c, a, b]);
    }

    #[test]
    fn place_document_moves_second_to_last_to_end() {
        let mut session = Session::new();
        let a = session.push(Some(path("a.wav")));
        let b = session.push(Some(path("b.wav")));
        let c = session.push(Some(path("c.wav")));
        // Gap before last (index 1 after remove adjust for b at 1) is a no-op;
        // index 2 must append.
        assert!(session.place_document(b, None, 1));
        assert_eq!(ids(&session), vec![a, b, c]);
        assert!(session.place_document(b, None, 2));
        assert_eq!(ids(&session), vec![a, c, b]);
    }

    #[test]
    fn groups_registry_round_trips_and_reorders() {
        let mut session = Session::new();
        let a = session.push(Some(path("a.facomp")));
        let b = session.push(Some(path("b.facomp")));
        assert!(session.add_group("todo"));
        assert!(session.add_group("done"));
        assert!(session.set_document_group(a, Some("todo".into())));
        assert!(session.set_document_group(b, Some("done".into())));
        assert!(session.add_group("empty"));
        assert_eq!(session.groups(), &["todo", "done", "empty"]);
        assert!(session.move_group("empty", 0));
        assert_eq!(session.groups(), &["empty", "todo", "done"]);
        assert!(session.rename_group("todo", "review"));
        assert_eq!(session.get(a).unwrap().group.as_deref(), Some("review"));
        let json = session.to_json(named).unwrap();
        assert!(json.contains("\"groups\""));
        let loaded = Session::from_json(&json, None).unwrap().session;
        assert_eq!(loaded.groups(), &["empty", "review", "done"]);
        assert!(loaded.get(a).unwrap().group.as_deref() == Some("review"));
    }

    #[test]
    fn delete_group_ungroups_members() {
        let mut session = Session::new();
        let a = session.push(Some(path("a.facomp")));
        session.add_group("todo");
        session.set_document_group(a, Some("todo".into()));
        assert!(session.delete_group("todo"));
        assert!(session.groups().is_empty());
        assert_eq!(session.get(a).unwrap().group, None);
    }

    #[test]
    fn from_json_rejects_unknown_kind() {
        let err = Session::from_json(
            r#"{"kind":"other","format_version":2,"id":"session:00000000-0000-0000-0000-000000000001","documents":[]}"#,
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("kind"), "{err}");
    }
}
