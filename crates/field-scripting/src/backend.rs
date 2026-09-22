// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Host backend seam: session/document/media ops behind a trait.
//!
//! *World* types hold state ([`crate::world::HeadlessWorld`]).
//! *Backend* types ([`HeadlessBackend`], and FieldAssist's `DesktopBackend`)
//! implement [`ScriptBackend`] so the shared Lua `field.*` surface can run
//! against different stores without depending on GPUI or desktop plumbing.
//!
//! ## Session routing
//!
//! Many session methods accept `which: Option<SessionId>`:
//! - `None` → the focused (world) session that `field.session.focused()`
//!   addresses.
//! - `Some(id)` → a *detached* session, loaded via `field.session.open()`.
//!   Detached sessions live in [`HeadlessBackend::detached_sessions`]; they
//!   hold session metadata but do **not** pre-open their documents (Phase 2
//!   will wire full document hydration for detached sessions).

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use field_audio_model::{MediaId, MediaRef, MediaStore};
use field_composition::Composition;
use field_session::{is_fasession_path, DocumentId, Session, SessionDocument, SessionId};

use crate::world::{HeadlessWorld, OpenDocument};

// ── Trait ────────────────────────────────────────────────────────────────────

/// Object-safe backend for `field.session` / composition / media ops.
///
/// All session methods accept `which: Option<SessionId>` (`None` = shared
/// world session, `Some(id)` = detached session loaded via
/// `open_detached_session`).  Document and media ops always address the shared
/// world pool.
pub trait ScriptBackend {
    // ── Session identity & metadata ─────────────────────────────────────────

    /// Session id string.
    fn session_id(&self, which: Option<SessionId>) -> String;

    /// Compatibility convenience for the shared session.
    fn shared_session_id(&self) -> String {
        self.session_id(None)
    }

    /// Compatibility convenience for a detached session.
    fn detached_session_id(&self, id: SessionId) -> Option<String> {
        Some(self.session_id(Some(id)))
    }

    /// Session filesystem path, if any.
    fn session_path(&self, which: Option<SessionId>) -> Option<String>;

    /// Compatibility convenience for the shared session.
    fn shared_session_path(&self) -> Option<String> {
        self.session_path(None)
    }

    /// Compatibility convenience for a detached session.
    fn detached_session_path(&self, id: SessionId) -> Option<String> {
        self.session_path(Some(id))
    }

    /// Persisted workflow name on the session.
    fn session_workflow(&self, which: Option<SessionId>) -> Option<String>;

    /// Compatibility convenience for the shared session.
    fn shared_session_workflow(&self) -> Option<String> {
        self.session_workflow(None)
    }

    /// Compatibility convenience for a detached session.
    fn detached_session_workflow(&self, id: SessionId) -> Option<String> {
        self.session_workflow(Some(id))
    }

    /// Set persisted workflow name.
    fn set_session_workflow(
        &mut self,
        which: Option<SessionId>,
        workflow: Option<String>,
    ) -> mlua::Result<()>;

    /// Compatibility convenience for the shared session.
    fn set_shared_session_workflow(&mut self, workflow: Option<String>) -> mlua::Result<()> {
        self.set_session_workflow(None, workflow)
    }

    /// `capture_ui` flag.
    fn session_capture_ui(&self, which: Option<SessionId>) -> bool;

    /// Compatibility convenience for the shared session.
    fn shared_session_capture_ui(&self) -> bool {
        self.session_capture_ui(None)
    }

    /// Set `capture_ui`.
    fn set_session_capture_ui(
        &mut self,
        which: Option<SessionId>,
        capture_ui: bool,
    ) -> mlua::Result<()>;

    /// Compatibility convenience for the shared session.
    fn set_shared_session_capture_ui(&mut self, capture_ui: bool) -> mlua::Result<()> {
        self.set_session_capture_ui(None, capture_ui)
    }

    /// Session property map.
    fn session_properties(&self, which: Option<SessionId>) -> BTreeMap<String, String>;

    /// Compatibility convenience for the shared session.
    fn shared_session_properties(&self) -> BTreeMap<String, String> {
        self.session_properties(None)
    }

    /// Compatibility convenience for a detached session.
    fn detached_session_properties(&self, id: SessionId) -> Option<BTreeMap<String, String>> {
        Some(self.session_properties(Some(id)))
    }

    /// Replace session property map.
    fn set_session_properties(
        &mut self,
        which: Option<SessionId>,
        properties: BTreeMap<String, String>,
    ) -> mlua::Result<()>;

    /// Compatibility convenience for the shared session.
    fn set_shared_session_properties(
        &mut self,
        properties: BTreeMap<String, String>,
    ) -> mlua::Result<()> {
        self.set_session_properties(None, properties)
    }

    /// Compatibility convenience for a detached session.
    fn set_detached_session_properties(
        &mut self,
        id: SessionId,
        properties: BTreeMap<String, String>,
    ) -> mlua::Result<()> {
        self.set_session_properties(Some(id), properties)
    }

    /// Active document on the session.
    fn session_active_document(&self, which: Option<SessionId>) -> Option<DocumentId>;

    /// Compatibility convenience for the shared session.
    fn shared_active_document(&self) -> Option<DocumentId> {
        self.session_active_document(None)
    }

    /// All document ids in the session.
    fn session_documents(&self, which: Option<SessionId>) -> Vec<DocumentId>;

    /// Compatibility convenience for the shared session.
    fn shared_session_documents(&self) -> Vec<DocumentId> {
        self.session_documents(None)
    }

    /// Compatibility convenience for a detached session.
    fn detached_session_documents(&self, id: SessionId) -> Option<Vec<DocumentId>> {
        Some(self.session_documents(Some(id)))
    }

    /// Count documents in a named group.
    fn session_group_count(&self, which: Option<SessionId>, group: &str) -> usize;

    /// Compatibility convenience for the shared session.
    fn shared_session_group_count(&self, group: &str) -> usize {
        self.session_group_count(None, group)
    }

    /// Group names in order.
    fn session_groups(&self, which: Option<SessionId>) -> Vec<String>;

    /// Compatibility convenience for the shared session.
    fn shared_session_groups(&self) -> Vec<String> {
        self.session_groups(None)
    }

    /// Replace group list.
    fn set_session_groups(
        &mut self,
        which: Option<SessionId>,
        groups: Vec<String>,
    ) -> mlua::Result<()>;

    /// Compatibility convenience for the shared session.
    fn set_shared_session_groups(&mut self, groups: Vec<String>) -> mlua::Result<()> {
        self.set_session_groups(None, groups)
    }

    /// Add a group.
    fn add_session_group(&mut self, which: Option<SessionId>, name: String) -> mlua::Result<()>;

    /// Compatibility convenience for the shared session.
    fn add_shared_session_group(&mut self, name: String) -> mlua::Result<()> {
        self.add_session_group(None, name)
    }

    /// Rename a group.
    fn rename_session_group(
        &mut self,
        which: Option<SessionId>,
        old: String,
        new: String,
    ) -> mlua::Result<()>;

    /// Compatibility convenience for the shared session.
    fn rename_shared_session_group(&mut self, old: String, new: String) -> mlua::Result<()> {
        self.rename_session_group(None, old, new)
    }

    /// Delete a group.
    fn delete_session_group(&mut self, which: Option<SessionId>, name: String) -> mlua::Result<()>;

    /// Compatibility convenience for the shared session.
    fn delete_shared_session_group(&mut self, name: String) -> mlua::Result<()> {
        self.delete_session_group(None, name)
    }

    /// Move a group to a 0-based index.
    fn move_session_group(
        &mut self,
        which: Option<SessionId>,
        name: String,
        index: usize,
    ) -> mlua::Result<()>;

    /// Compatibility convenience for the shared session.
    fn move_shared_session_group(&mut self, name: String, index: usize) -> mlua::Result<()> {
        self.move_session_group(None, name, index)
    }

    /// Move a document to a 0-based index.
    fn move_session_document(
        &mut self,
        which: Option<SessionId>,
        id: DocumentId,
        index: usize,
    ) -> mlua::Result<()>;

    /// Compatibility convenience for the shared session.
    fn move_shared_session_document(&mut self, id: DocumentId, index: usize) -> mlua::Result<()> {
        self.move_session_document(None, id, index)
    }

    /// Make a document the active one on the session.
    ///
    /// Returns `true` when focus changed and the caller should fire
    /// `composition_selected` hooks **after** releasing the backend borrow
    /// (hooks often re-enter the host). Detached focus never emits.
    ///
    /// For detached sessions this is a metadata-only update; the document need
    /// not be open in the world.
    fn set_session_active_document(
        &mut self,
        which: Option<SessionId>,
        id: DocumentId,
    ) -> mlua::Result<bool>;

    /// Compatibility convenience for the shared session.
    fn set_shared_active_document(&mut self, id: DocumentId) -> mlua::Result<bool> {
        self.set_session_active_document(None, id)
    }

    // ── World-session-only ops ──────────────────────────────────────────────

    /// Open a media or `.facomp` path into the **shared** world session.
    fn open_path(&mut self, path: &Path) -> mlua::Result<DocumentId>;

    /// Reset the **shared** world session and clear all open documents.
    fn reset_session(&mut self) -> mlua::Result<()>;

    /// Load a `.fasession` file into a new detached slot; return its id.
    ///
    /// The shared world session is **not** replaced.  Documents are **not**
    /// pre-hydrated in Phase 1; that is left for Phase 2.
    fn open_detached_session(&mut self, path: &Path) -> mlua::Result<SessionId>;

    /// Create an empty detached session.
    fn new_detached_session(&mut self) -> SessionId;

    /// Load a `.fasession` file into the **shared** world session, hydrating
    /// documents (batch / `field-batch` style).
    fn load_shared_session_file(&mut self, path: &Path) -> mlua::Result<()>;

    /// Save session.  Headless returns an error; desktop overrides this.
    fn save_session(&mut self, which: Option<SessionId>, path: Option<PathBuf>)
        -> mlua::Result<()>;

    /// Close a detached session loaded via `open_detached_session`.
    ///
    /// Returns `Err` if `id` is not a loaded detached session.
    fn close_detached_session(&mut self, id: SessionId) -> mlua::Result<()> {
        let _ = id;
        Err(mlua::Error::runtime(
            "session.close is not available in this backend",
        ))
    }

    // ── Document metadata (shared-world session docs) ───────────────────────

    /// Document group tag.
    fn document_group(&self, id: DocumentId) -> mlua::Result<Option<String>>;

    /// Set document group.
    fn set_document_group(&mut self, id: DocumentId, group: Option<String>) -> mlua::Result<()>;

    /// Document state tag.
    fn document_state(&self, id: DocumentId) -> mlua::Result<Option<String>>;

    /// Set document state.
    fn set_document_state(&mut self, id: DocumentId, state: Option<String>) -> mlua::Result<()>;

    /// Document property map.
    fn document_properties(&self, id: DocumentId) -> mlua::Result<BTreeMap<String, String>>;

    /// Set document property map.
    fn set_document_properties(
        &mut self,
        id: DocumentId,
        properties: BTreeMap<String, String>,
    ) -> mlua::Result<()>;

    /// Human-readable display name for a document.
    fn display_name(&self, id: DocumentId) -> Option<String>;

    /// Set display name.
    fn set_display_name(&mut self, id: DocumentId, name: String) -> mlua::Result<()>;

    /// Filesystem path for the document.
    fn document_path(&self, id: DocumentId) -> Option<PathBuf>;

    /// Inner composition UUID string.
    fn composition_id_string(&self, id: DocumentId) -> mlua::Result<String>;

    /// Close and remove a document.
    fn close_composition(&mut self, id: DocumentId) -> mlua::Result<()>;

    /// Replace document from a path and return the new document id.
    fn replace_composition(&mut self, id: DocumentId, path: &Path) -> mlua::Result<DocumentId>;

    // ── Open-document content access ────────────────────────────────────────

    /// Borrow an open document for read-only operations.
    ///
    /// The callback receives `&OpenDocument`; it must set `result` via a
    /// captured variable then return `Ok(())`.  The function returns the same
    /// `mlua::Result<()>` the callback returns.
    ///
    /// Headless: resolves from `HeadlessWorld.docs`.
    /// Desktop: should map the shared `BufferDocument` to an `OpenDocument`
    /// view, or return `Err(runtime("composition is not open"))`.
    fn with_open_document(
        &self,
        id: DocumentId,
        f: &mut dyn FnMut(&OpenDocument) -> mlua::Result<()>,
    ) -> mlua::Result<()>;

    /// Mutable borrow of an open document.
    fn with_open_document_mut(
        &mut self,
        id: DocumentId,
        f: &mut dyn FnMut(&mut OpenDocument) -> mlua::Result<()>,
    ) -> mlua::Result<()>;

    // ── Channel-layout application ──────────────────────────────────────────

    /// Apply a channel layout to a document's composition.
    ///
    /// `name` is set as the layout name on the composition, `labels` are
    /// applied per-channel, and `default_chain` is set as the monitor chain
    /// when the composition does not already have one.
    fn apply_document_channel_layout(
        &mut self,
        id: DocumentId,
        name: Option<&str>,
        labels: BTreeMap<usize, String>,
        default_chain: Option<String>,
    ) -> mlua::Result<()>;

    // ── Media pool ──────────────────────────────────────────────────────────

    /// Probe and intern a media file; return its id.
    fn add_media(&mut self, path: &Path) -> mlua::Result<MediaId>;

    /// Remove a media entry from the pool.
    ///
    /// Returns `Err` if the media is still referenced by open compositions
    /// (DesktopBackend) or is otherwise unremovable.
    fn remove_media(&mut self, id: MediaId) -> mlua::Result<()>;

    /// All media ids in the pool.
    fn list_media(&self) -> Vec<MediaId>;

    /// Look up media metadata.
    fn get_media(&self, id: MediaId) -> mlua::Result<MediaRef>;

    /// Optional handle to the underlying `Arc<Mutex<MediaStore>>` (advanced
    /// callers, e.g. for batch SRC checks).  Default: `None`.
    fn media_store(&self) -> Option<Arc<Mutex<MediaStore>>> {
        None
    }

    /// Notify a host-specific workflow toolbar renderer.
    fn toolbar_changed(&mut self) -> mlua::Result<()> {
        Ok(())
    }

    // ── Desktop chrome stubs ────────────────────────────────────────────────
    //
    // Headless returns `Ok(None)` / `Ok(vec![])` / `Err(unsupported)`.
    // DesktopBackend (Phase 2) overrides all of these.

    /// Parent composition id, if this document is a child clip.
    fn composition_parent(&self, _id: DocumentId) -> mlua::Result<Option<DocumentId>> {
        Ok(None)
    }

    /// Child composition ids, if this document is a parent clip.
    fn composition_children(&self, _id: DocumentId) -> mlua::Result<Vec<DocumentId>> {
        Ok(vec![])
    }

    /// Audio codec name as reported by the desktop format layer.
    fn composition_codec(&self, _id: DocumentId) -> mlua::Result<Option<String>> {
        Ok(None)
    }

    /// Active channel-layout name on the composition, if any.
    fn composition_channel_layout(&self, _id: DocumentId) -> mlua::Result<Option<String>> {
        Ok(None)
    }

    /// Active monitor chain name, if any.
    fn composition_monitor_chain(&self, _id: DocumentId) -> mlua::Result<Option<String>> {
        Ok(None)
    }

    /// Set monitor chain name.
    fn set_composition_monitor_chain(
        &mut self,
        _id: DocumentId,
        _chain: Option<String>,
    ) -> mlua::Result<()> {
        Err(mlua::Error::runtime(
            "monitor_chain is not available in the headless backend",
        ))
    }

    /// Playback channel override list, or `None` for all-channels.
    fn composition_playback_channels(&self, _id: DocumentId) -> mlua::Result<Option<Vec<usize>>> {
        Ok(None)
    }

    /// Set playback channel override.
    fn set_composition_playback_channels(
        &mut self,
        _id: DocumentId,
        _channels: Option<Vec<usize>>,
    ) -> mlua::Result<()> {
        Err(mlua::Error::runtime(
            "playback_channels is not available in the headless backend",
        ))
    }
}

// ── BackendHandle type alias ─────────────────────────────────────────────────

/// Shared reference-counted backend handle stored in [`crate::host::HostInner`].
pub type BackendHandle = Rc<RefCell<dyn ScriptBackend>>;

// ── HeadlessBackend ──────────────────────────────────────────────────────────

/// [`ScriptBackend`] backed by [`HeadlessWorld`].
///
/// `detached_sessions` stores sessions loaded with `field.session.open`;
/// the shared world session lives in `world.session`.  In Phase 1 detached
/// sessions hold metadata only (no open documents); Phase 2 will add
/// per-detached document hydration.
pub struct HeadlessBackend {
    /// Shared world — session, open documents, media pool.
    pub(crate) world: Rc<RefCell<HeadlessWorld>>,
    /// Sessions loaded by `field.session.open` (keyed by their `SessionId`).
    /// These do **not** replace the shared world session.
    pub(crate) detached_sessions: HashMap<SessionId, Session>,
}

impl HeadlessBackend {
    /// Create an empty backend.
    pub fn new() -> Self {
        Self {
            world: Rc::new(RefCell::new(HeadlessWorld::new())),
            detached_sessions: HashMap::new(),
        }
    }

    /// Wrap an existing world (useful in tests).
    pub fn from_world(world: HeadlessWorld) -> Self {
        Self {
            world: Rc::new(RefCell::new(world)),
            detached_sessions: HashMap::new(),
        }
    }

    /// Wrap a shared world handle.
    pub fn from_world_rc(world: Rc<RefCell<HeadlessWorld>>) -> Self {
        Self {
            world,
            detached_sessions: HashMap::new(),
        }
    }

    /// Access the shared world `Rc`.
    pub fn world_rc(&self) -> Rc<RefCell<HeadlessWorld>> {
        self.world.clone()
    }

    /// Borrow the session indicated by `which`.
    fn session_ref(&self, which: Option<SessionId>) -> SessionBorrow<'_> {
        match which {
            None => SessionBorrow::World(self.world.borrow()),
            Some(id) => {
                if self.detached_sessions.contains_key(&id) {
                    SessionBorrow::Detached(self.detached_sessions.get(&id).unwrap())
                } else {
                    // Unknown id — fall back to world session.
                    SessionBorrow::World(self.world.borrow())
                }
            }
        }
    }
}

impl Default for HeadlessBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// Temporary borrow discriminant — avoids holding two borrows at once.
enum SessionBorrow<'a> {
    World(std::cell::Ref<'a, HeadlessWorld>),
    Detached(&'a Session),
}

impl SessionBorrow<'_> {
    fn session(&self) -> &Session {
        match self {
            SessionBorrow::World(w) => &w.session,
            SessionBorrow::Detached(s) => s,
        }
    }
}

// ── ScriptBackend impl ───────────────────────────────────────────────────────

impl ScriptBackend for HeadlessBackend {
    // ── session identity & metadata ─────────────────────────────────────────

    fn session_id(&self, which: Option<SessionId>) -> String {
        match which {
            Some(id) => id.to_string(),
            None => self.session_ref(None).session().id().to_string(),
        }
    }

    fn session_path(&self, which: Option<SessionId>) -> Option<String> {
        self.session_ref(which)
            .session()
            .path()
            .map(|p| p.display().to_string())
    }

    fn session_workflow(&self, which: Option<SessionId>) -> Option<String> {
        self.session_ref(which)
            .session()
            .workflow()
            .map(str::to_string)
    }

    fn set_session_workflow(
        &mut self,
        which: Option<SessionId>,
        workflow: Option<String>,
    ) -> mlua::Result<()> {
        match which {
            None => self.world.borrow_mut().session.set_workflow(workflow),
            Some(id) => {
                if let Some(s) = self.detached_sessions.get_mut(&id) {
                    s.set_workflow(workflow);
                }
            }
        }
        Ok(())
    }

    fn session_capture_ui(&self, which: Option<SessionId>) -> bool {
        self.session_ref(which).session().capture_ui()
    }

    fn set_session_capture_ui(
        &mut self,
        which: Option<SessionId>,
        capture_ui: bool,
    ) -> mlua::Result<()> {
        match which {
            None => self.world.borrow_mut().session.set_capture_ui(capture_ui),
            Some(id) => {
                if let Some(s) = self.detached_sessions.get_mut(&id) {
                    s.set_capture_ui(capture_ui);
                }
            }
        }
        Ok(())
    }

    fn session_properties(&self, which: Option<SessionId>) -> BTreeMap<String, String> {
        self.session_ref(which).session().properties().clone()
    }

    fn set_session_properties(
        &mut self,
        which: Option<SessionId>,
        properties: BTreeMap<String, String>,
    ) -> mlua::Result<()> {
        match which {
            None => self.world.borrow_mut().session.set_properties(properties),
            Some(id) => {
                if let Some(s) = self.detached_sessions.get_mut(&id) {
                    s.set_properties(properties);
                }
            }
        }
        Ok(())
    }

    fn session_active_document(&self, which: Option<SessionId>) -> Option<DocumentId> {
        self.session_ref(which).session().active()
    }

    fn session_documents(&self, which: Option<SessionId>) -> Vec<DocumentId> {
        self.session_ref(which)
            .session()
            .documents()
            .iter()
            .map(|d| d.id)
            .collect()
    }

    fn session_group_count(&self, which: Option<SessionId>, group: &str) -> usize {
        self.session_ref(which).session().group_count(group)
    }

    fn session_groups(&self, which: Option<SessionId>) -> Vec<String> {
        self.session_ref(which).session().groups().to_vec()
    }

    fn set_session_groups(
        &mut self,
        which: Option<SessionId>,
        groups: Vec<String>,
    ) -> mlua::Result<()> {
        match which {
            None => {
                self.world.borrow_mut().session.set_groups(groups);
            }
            Some(id) => {
                if let Some(s) = self.detached_sessions.get_mut(&id) {
                    s.set_groups(groups);
                }
            }
        }
        Ok(())
    }

    fn add_session_group(&mut self, which: Option<SessionId>, name: String) -> mlua::Result<()> {
        match which {
            None => {
                self.world.borrow_mut().session.add_group(name);
            }
            Some(id) => {
                if let Some(s) = self.detached_sessions.get_mut(&id) {
                    s.add_group(name);
                }
            }
        }
        Ok(())
    }

    fn rename_session_group(
        &mut self,
        which: Option<SessionId>,
        old: String,
        new: String,
    ) -> mlua::Result<()> {
        match which {
            None => {
                self.world.borrow_mut().session.rename_group(&old, &new);
            }
            Some(id) => {
                if let Some(s) = self.detached_sessions.get_mut(&id) {
                    s.rename_group(&old, &new);
                }
            }
        }
        Ok(())
    }

    fn delete_session_group(&mut self, which: Option<SessionId>, name: String) -> mlua::Result<()> {
        match which {
            None => {
                self.world.borrow_mut().session.delete_group(&name);
            }
            Some(id) => {
                if let Some(s) = self.detached_sessions.get_mut(&id) {
                    s.delete_group(&name);
                }
            }
        }
        Ok(())
    }

    fn move_session_group(
        &mut self,
        which: Option<SessionId>,
        name: String,
        index: usize,
    ) -> mlua::Result<()> {
        match which {
            None => {
                self.world.borrow_mut().session.move_group(&name, index);
            }
            Some(id) => {
                if let Some(s) = self.detached_sessions.get_mut(&id) {
                    s.move_group(&name, index);
                }
            }
        }
        Ok(())
    }

    fn move_session_document(
        &mut self,
        which: Option<SessionId>,
        id: DocumentId,
        index: usize,
    ) -> mlua::Result<()> {
        match which {
            None => {
                self.world.borrow_mut().session.move_document(id, index);
            }
            Some(sid) => {
                if let Some(s) = self.detached_sessions.get_mut(&sid) {
                    s.move_document(id, index);
                }
            }
        }
        Ok(())
    }

    fn set_session_active_document(
        &mut self,
        which: Option<SessionId>,
        id: DocumentId,
    ) -> mlua::Result<bool> {
        match which {
            None => {
                let mut world = self.world.borrow_mut();
                if world.docs.get(&id).is_none() {
                    return Err(mlua::Error::runtime("composition is not open"));
                }
                Ok(world.session.focus(id).is_some())
            }
            Some(sid) => {
                // Detached: just record the focus; documents are not open in world.
                if let Some(s) = self.detached_sessions.get_mut(&sid) {
                    let _ = s.focus(id);
                }
                Ok(false)
            }
        }
    }

    // ── world-session-only ops ──────────────────────────────────────────────

    fn open_path(&mut self, path: &Path) -> mlua::Result<DocumentId> {
        if is_fasession_path(path) {
            return Err(mlua::Error::runtime(
                "use field.session.open for .fasession files",
            ));
        }
        let mut world = self.world.borrow_mut();
        if let Some(id) = world.session.find_by_path(path) {
            world.set_active(id);
            return Ok(id);
        }
        world
            .open_path(path)
            .map_err(|e| mlua::Error::runtime(e.to_string()))
    }

    fn reset_session(&mut self) -> mlua::Result<()> {
        let mut world = self.world.borrow_mut();
        world.docs.clear();
        world.session = Session::new();
        Ok(())
    }

    fn open_detached_session(&mut self, path: &Path) -> mlua::Result<SessionId> {
        if !is_fasession_path(path) {
            return Err(mlua::Error::runtime(
                "field.session.open expects a .fasession file",
            ));
        }
        let json =
            std::fs::read_to_string(path).map_err(|e| mlua::Error::runtime(e.to_string()))?;
        let loaded = Session::from_json(&json, Some(path))
            .map_err(|e| mlua::Error::runtime(e.to_string()))?;
        let id = loaded.session.id();
        self.detached_sessions.insert(id, loaded.session);
        Ok(id)
    }

    fn new_detached_session(&mut self) -> SessionId {
        let session = Session::new();
        let id = session.id();
        self.detached_sessions.insert(id, session);
        id
    }

    fn load_shared_session_file(&mut self, path: &Path) -> mlua::Result<()> {
        if !is_fasession_path(path) {
            return Err(mlua::Error::runtime(
                "field.session.open expects a .fasession file",
            ));
        }
        let json =
            std::fs::read_to_string(path).map_err(|e| mlua::Error::runtime(e.to_string()))?;
        let loaded = Session::from_json(&json, Some(path))
            .map_err(|e| mlua::Error::runtime(e.to_string()))?;
        let mut world = self.world.borrow_mut();
        world.docs.clear();
        world.session = loaded.session;
        world.session.set_path(Some(path.to_path_buf()));
        hydrate_session_documents(&mut world)?;
        Ok(())
    }

    fn save_session(
        &mut self,
        _which: Option<SessionId>,
        _path: Option<PathBuf>,
    ) -> mlua::Result<()> {
        Err(mlua::Error::runtime(
            "session save is not implemented in the headless backend",
        ))
    }

    // ── close detached session ──────────────────────────────────────────────

    fn close_detached_session(&mut self, id: SessionId) -> mlua::Result<()> {
        if self.detached_sessions.remove(&id).is_none() {
            return Err(mlua::Error::runtime("session is not loaded"));
        }
        Ok(())
    }

    // ── document metadata ───────────────────────────────────────────────────

    fn document_group(&self, id: DocumentId) -> mlua::Result<Option<String>> {
        Ok(self
            .world
            .borrow()
            .session
            .get(id)
            .and_then(|d| d.group.clone()))
    }

    fn set_document_group(&mut self, id: DocumentId, group: Option<String>) -> mlua::Result<()> {
        // Mirror FieldAssist: Session::set_document_group also registers the
        // name via ensure_group so explorers / group_count stay consistent.
        let mut world = self.world.borrow_mut();
        if world.session.set_document_group(id, group) {
            Ok(())
        } else {
            Err(mlua::Error::runtime("composition is not open"))
        }
    }

    fn document_state(&self, id: DocumentId) -> mlua::Result<Option<String>> {
        Ok(self
            .world
            .borrow()
            .session
            .get(id)
            .and_then(|d| d.state.clone()))
    }

    fn set_document_state(&mut self, id: DocumentId, state: Option<String>) -> mlua::Result<()> {
        let mut world = self.world.borrow_mut();
        let doc = world
            .session
            .get_mut(id)
            .ok_or_else(|| mlua::Error::runtime("composition is not open"))?;
        doc.state = state;
        world.session.mark_dirty();
        Ok(())
    }

    fn document_properties(&self, id: DocumentId) -> mlua::Result<BTreeMap<String, String>> {
        Ok(self
            .world
            .borrow()
            .session
            .get(id)
            .map(|d| d.properties.clone())
            .unwrap_or_default())
    }

    fn set_document_properties(
        &mut self,
        id: DocumentId,
        properties: BTreeMap<String, String>,
    ) -> mlua::Result<()> {
        let mut world = self.world.borrow_mut();
        let doc = world
            .session
            .get_mut(id)
            .ok_or_else(|| mlua::Error::runtime("composition is not open"))?;
        doc.properties = properties;
        world.session.mark_dirty();
        Ok(())
    }

    fn display_name(&self, id: DocumentId) -> Option<String> {
        let world = self.world.borrow();
        world.docs.get(&id).map(|d| d.name.clone()).or_else(|| {
            world.session.get(id).and_then(|d| {
                d.name.clone().or_else(|| {
                    d.file_path()
                        .and_then(|p| p.file_stem())
                        .map(|s| s.to_string_lossy().into_owned())
                })
            })
        })
    }

    fn set_display_name(&mut self, id: DocumentId, name: String) -> mlua::Result<()> {
        let mut world = self.world.borrow_mut();
        if let Some(doc) = world.docs.get_mut(&id) {
            doc.name = name.clone();
        }
        if let Some(doc) = world.session.get_mut(id) {
            doc.name = Some(name);
            world.session.mark_dirty();
        }
        Ok(())
    }

    fn document_path(&self, id: DocumentId) -> Option<PathBuf> {
        let world = self.world.borrow();
        world
            .docs
            .get(&id)
            .and_then(|d| d.path.clone())
            .or_else(|| {
                world
                    .session
                    .get(id)
                    .and_then(|d| d.file_path().map(Path::to_path_buf))
            })
    }

    fn composition_id_string(&self, id: DocumentId) -> mlua::Result<String> {
        let world = self.world.borrow();
        let doc = world
            .docs
            .get(&id)
            .ok_or_else(|| mlua::Error::runtime("composition is not open"))?;
        let composition_id = doc.composition.read().unwrap().id().to_string();
        Ok(composition_id)
    }

    fn close_composition(&mut self, id: DocumentId) -> mlua::Result<()> {
        self.world.borrow_mut().close(id);
        Ok(())
    }

    fn replace_composition(&mut self, id: DocumentId, path: &Path) -> mlua::Result<DocumentId> {
        self.world
            .borrow_mut()
            .replace(id, path)
            .map_err(|e| mlua::Error::runtime(e.to_string()))
    }

    // ── open-document content access ────────────────────────────────────────

    fn with_open_document(
        &self,
        id: DocumentId,
        f: &mut dyn FnMut(&OpenDocument) -> mlua::Result<()>,
    ) -> mlua::Result<()> {
        let world = self.world.borrow();
        let doc = world
            .docs
            .get(&id)
            .ok_or_else(|| mlua::Error::runtime("composition is not open"))?;
        f(doc)
    }

    fn with_open_document_mut(
        &mut self,
        id: DocumentId,
        f: &mut dyn FnMut(&mut OpenDocument) -> mlua::Result<()>,
    ) -> mlua::Result<()> {
        let mut world = self.world.borrow_mut();
        let doc = world
            .docs
            .get_mut(&id)
            .ok_or_else(|| mlua::Error::runtime("composition is not open"))?;
        f(doc)
    }

    // ── channel-layout application ──────────────────────────────────────────

    fn apply_document_channel_layout(
        &mut self,
        id: DocumentId,
        name: Option<&str>,
        labels: BTreeMap<usize, String>,
        default_chain: Option<String>,
    ) -> mlua::Result<()> {
        let name_owned = name.map(str::to_string);
        let mut world = self.world.borrow_mut();
        let doc = world
            .docs
            .get_mut(&id)
            .ok_or_else(|| mlua::Error::runtime("composition is not open"))?;
        let mut composition = doc.composition.write().unwrap();
        composition.apply_channel_layout(name_owned, labels);
        if composition.monitor_chain().is_none() {
            if let Some(chain) = default_chain {
                composition.set_monitor_chain(Some(chain));
            }
        }
        Ok(())
    }

    // ── media pool ──────────────────────────────────────────────────────────

    fn add_media(&mut self, path: &Path) -> mlua::Result<MediaId> {
        self.world
            .borrow_mut()
            .add_media(path)
            .map_err(|e| mlua::Error::runtime(e.to_string()))
    }

    fn remove_media(&mut self, id: MediaId) -> mlua::Result<()> {
        self.world
            .borrow_mut()
            .media_store
            .lock()
            .unwrap()
            .remove(id);
        Ok(())
    }

    fn list_media(&self) -> Vec<MediaId> {
        self.world
            .borrow()
            .media_store
            .lock()
            .unwrap()
            .pool()
            .iter()
            .map(|m| m.id)
            .collect()
    }

    fn get_media(&self, id: MediaId) -> mlua::Result<MediaRef> {
        self.world
            .borrow()
            .media_store
            .lock()
            .unwrap()
            .pool()
            .iter()
            .find(|media| media.id == id)
            .cloned()
            .ok_or_else(|| mlua::Error::runtime(format!("unknown media id: {id}")))
    }

    fn media_store(&self) -> Option<Arc<Mutex<MediaStore>>> {
        Some(self.world.borrow().media_store.clone())
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Hydrate open documents from the session manifest.
///
/// For each session document whose path exists on disk, load its composition
/// into `world.docs`.  Documents that cannot be loaded get a placeholder empty
/// composition.
pub(crate) fn hydrate_session_documents(world: &mut HeadlessWorld) -> mlua::Result<()> {
    let docs: Vec<SessionDocument> = world.session.documents().to_vec();
    for session_doc in docs {
        let name = session_doc
            .name
            .clone()
            .or_else(|| {
                session_doc
                    .file_path()
                    .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            })
            .unwrap_or_else(|| session_doc.id.to_string());
        if world.docs.contains_key(&session_doc.id) {
            continue;
        }
        if let Some(path) = session_doc.file_path() {
            if path.exists() && !is_fasession_path(path) {
                let loaded = Composition::load_from_path_with_warnings(path).map(|(c, _)| c);
                if let Ok(composition) = loaded {
                    world.docs.insert(
                        session_doc.id,
                        OpenDocument::new(composition, name, Some(path.to_path_buf())),
                    );
                    continue;
                }
            }
        }
        let composition = Composition::new(48_000, 2);
        world
            .docs
            .insert(session_doc.id, OpenDocument::new(composition, name, None));
    }
    Ok(())
}
