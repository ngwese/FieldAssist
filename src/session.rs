// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

/// Stable identity for an open composition in the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DocumentId(pub u64);

impl DocumentId {
    pub fn to_tree_id(self) -> String {
        format!("doc:{}", self.0)
    }

    pub fn from_tree_id(id: &str) -> Option<Self> {
        let rest = id.strip_prefix("doc:")?;
        rest.parse().ok().map(Self)
    }
}

/// Session metadata for one open composition. View entities live in AppView.
#[derive(Clone, Debug)]
pub struct OpenDocument {
    pub id: DocumentId,
    pub source_path: Option<PathBuf>,
    pub project_path: Option<PathBuf>,
    pub tab_open: bool,
    /// Pinned tabs stay in the bar; transient tabs may be replaced.
    pub tab_pinned: bool,
}

/// Ordered list of open compositions and which one is active.
#[derive(Clone, Debug, Default)]
pub struct DocumentSession {
    next_id: u64,
    documents: Vec<OpenDocument>,
    active: Option<DocumentId>,
}

impl DocumentSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn documents(&self) -> &[OpenDocument] {
        &self.documents
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

    pub fn get(&self, id: DocumentId) -> Option<&OpenDocument> {
        self.documents.iter().find(|doc| doc.id == id)
    }

    pub fn get_mut(&mut self, id: DocumentId) -> Option<&mut OpenDocument> {
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
        self.next_id += 1;
        let id = DocumentId(self.next_id);
        self.documents.push(OpenDocument {
            id,
            source_path,
            project_path: None,
            tab_open: true,
            tab_pinned: false,
        });
        self.active = Some(id);
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
        previous
    }

    pub fn ensure_tab(&mut self, id: DocumentId) -> bool {
        let Some(doc) = self.get_mut(id) else {
            return false;
        };
        if doc.tab_open {
            return false;
        }
        doc.tab_open = true;
        true
    }

    pub fn close_tab(&mut self, id: DocumentId) -> bool {
        let Some(doc) = self.get_mut(id) else {
            return false;
        };
        if !doc.tab_open {
            return false;
        }
        doc.tab_open = false;
        doc.tab_pinned = false;
        true
    }

    pub fn set_tab_open(&mut self, id: DocumentId, open: bool) {
        if let Some(doc) = self.get_mut(id) {
            doc.tab_open = open;
            if !open {
                doc.tab_pinned = false;
            }
        }
    }

    /// Pin an open tab. Returns false if the document has no tab.
    pub fn pin_tab(&mut self, id: DocumentId) -> bool {
        let Some(doc) = self.get_mut(id) else {
            return false;
        };
        if !doc.tab_open || doc.tab_pinned {
            return false;
        }
        doc.tab_pinned = true;
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
        true
    }
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

    #[test]
    fn open_two_paths_lists_both() {
        let mut session = DocumentSession::new();
        session.push(Some(path("a.wav")));
        session.push(Some(path("b.wav")));
        assert_eq!(session.len(), 2);
        assert_eq!(session.tab_open_count(), 2);
    }

    #[test]
    fn closing_one_tab_keeps_both_documents() {
        let mut session = DocumentSession::new();
        let a = session.push(Some(path("a.wav")));
        session.push(Some(path("b.wav")));
        assert!(session.close_tab(a));
        assert_eq!(session.len(), 2);
        assert_eq!(session.tab_open_count(), 1);
        assert!(!session.get(a).unwrap().tab_open);
    }

    #[test]
    fn ensure_tab_restores_a_closed_tab() {
        let mut session = DocumentSession::new();
        let a = session.push(Some(path("a.wav")));
        session.push(Some(path("b.wav")));
        session.close_tab(a);
        assert!(session.ensure_tab(a));
        assert!(session.get(a).unwrap().tab_open);
        assert_eq!(session.tab_open_count(), 2);
    }

    #[test]
    fn close_from_explorer_drops_document_and_tab() {
        let mut session = DocumentSession::new();
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
        let mut session = DocumentSession::new();
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
        let mut session = DocumentSession::new();
        let a = session.push(Some(path("a.wav")));
        session.get_mut(a).unwrap().project_path = Some(path("a.wav.facomp"));
        assert_eq!(session.find_by_path(Path::new("a.wav.facomp")), Some(a));
    }

    #[test]
    fn tree_id_round_trips() {
        let id = DocumentId(42);
        assert_eq!(DocumentId::from_tree_id(&id.to_tree_id()), Some(id));
        assert_eq!(DocumentId::from_tree_id("nope"), None);
    }

    #[test]
    fn new_tabs_are_transient() {
        let mut session = DocumentSession::new();
        let a = session.push(Some(path("a.wav")));
        assert!(!session.tab_pinned(a));
        assert!(session.get(a).unwrap().tab_open);
    }

    #[test]
    fn pin_then_close_clears_pin() {
        let mut session = DocumentSession::new();
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
        let mut session = DocumentSession::new();
        let a = session.push(Some(path("a.wav")));
        let b = session.push(Some(path("b.wav")));
        let c = session.push(Some(path("c.wav")));
        session.pin_tab(a);
        session.close_tab(b);
        assert_eq!(session.first_transient_in(&[a, b, c]), Some(c));
    }

    #[test]
    fn closing_saved_tabs_leaves_dirty_tabs_open() {
        let mut session = DocumentSession::new();
        let saved = session.push(Some(path("a.wav")));
        let dirty = session.push(Some(path("b.wav")));
        session.pin_tab(dirty);
        assert!(session.close_tab(saved));
        assert!(!session.get(saved).unwrap().tab_open);
        assert!(session.get(dirty).unwrap().tab_open);
        assert!(session.tab_pinned(dirty));
        assert_eq!(session.tab_open_count(), 1);
    }
}
