// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::file_url::{encode_file_url, resolve_file_url};

pub const FASESSION_KIND: &str = "fasession";
pub const FASESSION_FORMAT_VERSION: u32 = 1;

/// Stable identity for an open composition in the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DocumentId(pub Uuid);

impl DocumentId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_u128(n: u128) -> Self {
        Self(Uuid::from_u128(n))
    }

    pub fn to_tree_id(self) -> String {
        format!("doc:{}", self.0)
    }

    pub fn from_tree_id(id: &str) -> Option<Self> {
        let rest = id.strip_prefix("doc:")?;
        rest.parse().ok().map(Self)
    }
}

impl fmt::Display for DocumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Identity of a session, stable across save/load.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_u128(n: u128) -> Self {
        Self(Uuid::from_u128(n))
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Session metadata for one open composition. View entities live in AppView.
#[derive(Clone, Debug)]
pub struct SessionDocument {
    pub id: DocumentId,
    pub source_path: Option<PathBuf>,
    pub project_path: Option<PathBuf>,
    pub tab_open: bool,
    /// Pinned tabs stay in the bar; transient tabs may be replaced.
    pub tab_pinned: bool,
    pub group: Option<String>,
    pub state: Option<String>,
    pub properties: BTreeMap<String, String>,
}

impl SessionDocument {
    pub fn new(id: DocumentId, source_path: Option<PathBuf>) -> Self {
        Self {
            id,
            source_path,
            project_path: None,
            tab_open: true,
            tab_pinned: false,
            group: None,
            state: None,
            properties: BTreeMap::new(),
        }
    }

    pub fn file_path(&self) -> Option<&Path> {
        self.project_path.as_deref().or(self.source_path.as_deref())
    }
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
    documents: Vec<SessionDocument>,
    active: Option<DocumentId>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            id: SessionId::new(),
            path: None,
            workflow: None,
            properties: BTreeMap::new(),
            capture_ui: true,
            dirty: false,
            documents: Vec::new(),
            active: None,
        }
    }

    pub fn id(&self) -> SessionId {
        self.id
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn set_path(&mut self, path: Option<PathBuf>) {
        if self.path != path {
            self.path = path;
            self.mark_dirty();
        }
    }

    pub fn workflow(&self) -> Option<&str> {
        self.workflow.as_deref()
    }

    pub fn set_workflow(&mut self, workflow: Option<String>) {
        let workflow = workflow.filter(|name| !name.is_empty());
        if self.workflow != workflow {
            self.workflow = workflow;
            self.mark_dirty();
        }
    }

    pub fn properties(&self) -> &BTreeMap<String, String> {
        &self.properties
    }

    pub fn set_properties(&mut self, properties: BTreeMap<String, String>) {
        if self.properties != properties {
            self.properties = properties;
            self.mark_dirty();
        }
    }

    pub fn capture_ui(&self) -> bool {
        self.capture_ui
    }

    pub fn set_capture_ui(&mut self, capture_ui: bool) {
        if self.capture_ui != capture_ui {
            self.capture_ui = capture_ui;
            self.mark_dirty();
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    pub fn documents(&self) -> &[SessionDocument] {
        &self.documents
    }

    pub fn take_documents(&mut self) -> Vec<SessionDocument> {
        std::mem::take(&mut self.documents)
    }

    pub fn len(&self) -> usize {
        self.documents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }

    pub fn tab_open_count(&self) -> usize {
        self.documents.iter().filter(|doc| doc.tab_open).count()
    }

    pub fn active(&self) -> Option<DocumentId> {
        self.active
    }

    pub fn get(&self, id: DocumentId) -> Option<&SessionDocument> {
        self.documents.iter().find(|doc| doc.id == id)
    }

    pub fn get_mut(&mut self, id: DocumentId) -> Option<&mut SessionDocument> {
        self.documents.iter_mut().find(|doc| doc.id == id)
    }

    pub fn find_by_path(&self, path: &Path) -> Option<DocumentId> {
        self.documents.iter().find_map(|doc| {
            let matches_source = doc
                .source_path
                .as_deref()
                .is_some_and(|source| paths_equivalent(source, path));
            let matches_project = doc
                .project_path
                .as_deref()
                .is_some_and(|project| paths_equivalent(project, path));
            (matches_source || matches_project).then_some(doc.id)
        })
    }

    /// Insert a document, make it active, and open a center tab.
    pub fn push(&mut self, source_path: Option<PathBuf>) -> DocumentId {
        self.insert(SessionDocument::new(DocumentId::new(), source_path))
    }

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

    pub fn set_project_path(&mut self, id: DocumentId, path: PathBuf) {
        let changed = match self.get_mut(id) {
            Some(doc) if doc.project_path.as_ref() != Some(&path) => {
                doc.project_path = Some(path);
                true
            }
            _ => false,
        };
        if changed {
            self.mark_dirty();
        }
    }

    pub fn set_document_group(&mut self, id: DocumentId, group: Option<String>) -> bool {
        let group = group.filter(|name| !name.is_empty());
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

    pub fn to_json(&self, name_of: impl Fn(DocumentId) -> String) -> Result<String> {
        self.to_envelope(name_of, None)?.to_json()
    }

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
        for doc in &self.documents {
            let Some(path) = doc.file_path() else {
                bail!("cannot save session: {} has no file URL", name_of(doc.id));
            };
            documents.push(SessionDocumentFile {
                id: doc.id,
                url: encode_file_url(path, base),
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
            documents,
            ui: None,
        })
    }

    pub fn from_json(json: &str, session_path: Option<&Path>) -> Result<LoadedSession> {
        let envelope = SessionEnvelope::from_json(json)?;
        let base = session_path.and_then(|path| path.parent());
        let mut documents = Vec::with_capacity(envelope.documents.len());
        for file_doc in envelope.documents {
            let path = resolve_file_url(&file_doc.url, base)
                .with_context(|| format!("invalid document URL {}", file_doc.url))?;
            let (source_path, project_path) = if is_fasession_path(&path) {
                bail!("session documents cannot be nested session files");
            } else if super::composition::is_facomp_path(&path) {
                (None, Some(path))
            } else {
                (Some(path), None)
            };
            documents.push(SessionDocument {
                id: file_doc.id,
                source_path,
                project_path,
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
        Ok(LoadedSession {
            session: Session {
                id: envelope.id,
                path: session_path.map(Path::to_path_buf),
                workflow: envelope.workflow,
                properties: envelope.properties,
                capture_ui: envelope.capture_ui,
                dirty: false,
                documents,
                active,
            },
            ui: envelope.ui,
        })
    }

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
pub struct LoadedSession {
    pub session: Session,
    pub ui: Option<SessionUi>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionUi {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<SessionWindowUi>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docks: Option<SessionDocksUi>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionWindowUi {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionDocksUi {
    pub explorer: bool,
    pub detail: bool,
    pub script: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionEnvelope {
    pub kind: String,
    pub format_version: u32,
    pub id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<DocumentId>,
    #[serde(default = "default_true")]
    pub capture_ui: bool,
    pub documents: Vec<SessionDocumentFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<SessionUi>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionDocumentFile {
    pub id: DocumentId,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, String>,
    #[serde(default = "default_true")]
    pub tab_open: bool,
    #[serde(default)]
    pub tab_pinned: bool,
}

impl SessionEnvelope {
    fn from_json(json: &str) -> Result<Self> {
        let envelope: Self = serde_json::from_str(json).context("parse session JSON")?;
        if envelope.kind != FASESSION_KIND {
            bail!("not a FieldAssist session (kind {:?})", envelope.kind);
        }
        match envelope.format_version {
            1 => Ok(envelope),
            0 => bail!("missing or invalid format_version"),
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

pub fn is_fasession_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("fasession"))
}

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
        session.get_mut(a).unwrap().project_path = Some(path("a.wav.facomp"));
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
        session.insert(SessionDocument {
            id,
            source_path: Some(wav.clone()),
            project_path: None,
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
        assert_eq!(doc.source_path.as_deref(), Some(wav.as_path()));
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
        assert_eq!(id.to_string(), Uuid::from_u128(99).to_string());
    }

    #[test]
    fn is_fasession_detects_extension() {
        assert!(is_fasession_path(Path::new("batch.fasession")));
        assert!(is_fasession_path(Path::new("BATCH.FASESSION")));
        assert!(!is_fasession_path(Path::new("take.wav")));
    }

    #[test]
    fn from_json_rejects_unknown_kind() {
        let err = Session::from_json(
            r#"{"kind":"other","format_version":1,"id":"00000000-0000-0000-0000-000000000001","documents":[]}"#,
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("kind"), "{err}");
    }
}
