// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! FieldAssist adapter for the shared scripting backend.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use field_audio_model::{MediaId, MediaRef, MediaStore};
use field_scripting::{OpenDocument, ScriptBackend};
use field_session::{is_fasession_path, DocumentId, Session, SessionDocument, SessionId};

use crate::model::{is_facomp_path, BufferDocument, Composition};

use super::access;
use super::host::TestWorld;

/// Inner state of DesktopBackend, shared via Rc.
struct DesktopInner {
    test: Option<Rc<RefCell<TestWorld>>>,
    detached_sessions: HashMap<SessionId, Session>,
}

/// Script backend backed by the FieldAssist desktop model.
///
/// Cloning produces a second handle to the *same* inner state so both
/// `field_scripting::ScriptHost` (which owns the backend via trait object) and
/// the FA Lua app-data slot can share session/test state.
#[derive(Clone)]
pub struct DesktopBackend {
    inner: Rc<RefCell<DesktopInner>>,
}

/// Lua app-data handle for FA-specific app.*/theme.* bindings.
#[allow(dead_code)]
pub(crate) type DesktopBackendHandle = DesktopBackend;

pub(crate) fn install_lua_backend(lua: &mlua::Lua, backend: DesktopBackend) {
    lua.set_app_data(backend);
}

pub(crate) fn backend_from_lua(lua: &mlua::Lua) -> mlua::Result<DesktopBackend> {
    lua.app_data_ref::<DesktopBackend>()
        .map(|handle| handle.clone())
        .ok_or_else(|| mlua::Error::runtime("desktop backend is not bound"))
}

impl DesktopBackend {
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(DesktopInner {
                test: None,
                detached_sessions: HashMap::new(),
            })),
        }
    }

    #[cfg(test)]
    pub fn for_test(test: Rc<RefCell<TestWorld>>) -> Self {
        Self {
            inner: Rc::new(RefCell::new(DesktopInner {
                test: Some(test),
                detached_sessions: HashMap::new(),
            })),
        }
    }

    fn test(&self) -> Option<Rc<RefCell<TestWorld>>> {
        self.inner.borrow().test.clone()
    }

    fn with_session<R>(&self, f: impl FnOnce(&Session) -> R) -> Option<R> {
        if let Some(test) = self.test() {
            return Some(f(&test.borrow().session));
        }
        access::with_view(|view, _, _| f(view.session())).ok()
    }

    fn with_session_mut<R>(
        &mut self,
        f: impl FnOnce(&mut Session) -> mlua::Result<R>,
    ) -> mlua::Result<R> {
        if let Some(test) = self.test() {
            return f(&mut test.borrow_mut().session);
        }
        access::with_view(|view, _, _| f(view.session_mut())).map_err(mlua::Error::runtime)?
    }

    fn with_session_kind<R>(&self, which: Option<SessionId>, f: impl FnOnce(&Session) -> R) -> R {
        match which {
            Some(id) if self.inner.borrow().detached_sessions.contains_key(&id) => {
                let session = self.inner.borrow().detached_sessions[&id].clone();
                f(&session)
            }
            _ => self
                .with_session(f)
                .expect("desktop scripting requires an app view"),
        }
    }

    fn with_session_kind_mut<R>(
        &mut self,
        which: Option<SessionId>,
        f: impl FnOnce(&mut Session) -> mlua::Result<R>,
    ) -> mlua::Result<R> {
        if let Some(id) = which {
            let has_id = self.inner.borrow().detached_sessions.contains_key(&id);
            if has_id {
                let mut session = self
                    .inner
                    .borrow_mut()
                    .detached_sessions
                    .remove(&id)
                    .unwrap();
                let result = f(&mut session);
                self.inner
                    .borrow_mut()
                    .detached_sessions
                    .insert(id, session);
                return result;
            }
        }
        self.with_session_mut(f)
    }

    fn detached_document(&self, id: DocumentId) -> Option<SessionDocument> {
        for session in self.inner.borrow().detached_sessions.values() {
            if let Some(doc) = session.get(id) {
                return Some(doc.clone());
            }
        }
        None
    }

    fn open_from_buffer(doc: &BufferDocument, name: String, path: Option<PathBuf>) -> OpenDocument {
        let mut open = OpenDocument::from_shared(
            doc.composition.clone(),
            doc.buffer.clone(),
            name,
            path,
            doc.next_region_id(),
        );
        open.selection = doc.selection.clone();
        open.position = doc.current_position.clone();
        open.collections = doc
            .composition
            .read()
            .unwrap()
            .collections()
            .iter()
            .map(|collection| (collection.name.clone(), collection.clone()))
            .collect();
        open
    }

    fn apply_open_buffer(doc: &mut BufferDocument, open: &OpenDocument) {
        doc.selection = open.selection.clone();
        doc.current_position = open.position.clone();
        doc.set_next_region_id(open.next_region_id());
        let mut composition = doc.composition.write().unwrap();
        for collection in open.collections.values() {
            if let Some(target) = composition.ensure_collection(&collection.name) {
                *target = collection.clone();
            }
        }
    }

    fn with_buffer<R>(
        &self,
        id: DocumentId,
        f: impl FnOnce(&BufferDocument) -> mlua::Result<R>,
    ) -> mlua::Result<R> {
        if let Some(test) = self.test() {
            let world = test.borrow();
            return f(world
                .docs
                .get(&id)
                .ok_or_else(|| mlua::Error::runtime("composition is not open"))?);
        }
        access::with_view(|view, _, cx| view.script_with_document(id, cx, |doc| f(doc)))
            .map_err(mlua::Error::runtime)?
    }

    fn with_buffer_mut<R>(
        &mut self,
        id: DocumentId,
        f: impl FnOnce(&mut BufferDocument) -> mlua::Result<R>,
    ) -> mlua::Result<R> {
        if let Some(test) = self.test() {
            let mut world = test.borrow_mut();
            return f(world
                .docs
                .get_mut(&id)
                .ok_or_else(|| mlua::Error::runtime("composition is not open"))?);
        }
        access::with_view(|view, _, cx| view.script_with_document(id, cx, f))
            .map_err(mlua::Error::runtime)?
    }

    fn refresh(&self) {
        if self.test().is_none() {
            let _ = access::with_view(|view, _, cx| view.refresh_explorer(cx));
        }
    }

    pub(crate) fn output_device(&self) -> Option<String> {
        if let Some(test) = self.test() {
            return test.borrow().output_device.clone();
        }
        access::with_view(|view, _, _| view.output_device().map(str::to_owned))
            .ok()
            .flatten()
    }

    pub(crate) fn output_devices(&self) -> Vec<String> {
        if let Some(test) = self.test() {
            return test.borrow().output_devices.clone();
        }
        crate::playback::list_output_devices()
            .map(|devices| devices.into_iter().map(|info| info.name).collect())
            .unwrap_or_default()
    }

    pub(crate) fn set_output_device(&mut self, spec: Option<&str>) -> mlua::Result<()> {
        if let Some(test) = self.test() {
            test.borrow_mut().output_device = spec.map(str::to_owned);
            return Ok(());
        }
        access::with_view(|view, window, cx| view.set_output_device(spec, window, cx))
            .map_err(mlua::Error::runtime)?
            .map_err(mlua::Error::runtime)
    }

    pub(crate) fn command(&mut self, id: &str) -> Result<(), String> {
        crate::commands::validate_command_id(id)?;
        if let Some(test) = self.test() {
            let mut world = test.borrow_mut();
            match id {
                "transport.loop" => world.looping = !world.looping,
                "transport.preview" => world.preview = !world.preview,
                "view.show-explorer" => world.explorer = true,
                "view.hide-explorer" => world.explorer = false,
                "view.toggle-explorer" => world.explorer = !world.explorer,
                "view.show-detail" => world.detail = true,
                "view.hide-detail" => world.detail = false,
                "view.toggle-detail" => world.detail = !world.detail,
                "view.show-script" => world.script = true,
                "view.hide-script" => world.script = false,
                "view.toggle-script" => world.script = !world.script,
                _ => {}
            }
            return Ok(());
        }
        access::with_view(|view, window, cx| view.invoke_command(id, window, cx))?
    }

    pub(crate) fn looping(&self) -> bool {
        self.test()
            .map(|t| t.borrow().looping)
            .unwrap_or_else(|| access::with_view(|v, _, _| v.playback_looping()).unwrap_or(false))
    }
    pub(crate) fn preview(&self) -> bool {
        self.test()
            .map(|t| t.borrow().preview)
            .unwrap_or_else(|| access::with_view(|v, _, _| v.preview_enabled()).unwrap_or(false))
    }
    pub(crate) fn explorer(&self) -> bool {
        self.test().map(|t| t.borrow().explorer).unwrap_or_else(|| {
            access::with_view(|v, _, cx| v.explorer_dock_open(cx)).unwrap_or(false)
        })
    }

    pub(crate) fn theme_name(&self) -> String {
        self.test()
            .map(|t| t.borrow().theme_name.clone())
            .unwrap_or_else(|| {
                super::theme::live_theme_name().unwrap_or_else(|_| "Default Dark".into())
            })
    }
    pub(crate) fn theme_mode(&self) -> String {
        self.test()
            .map(|t| t.borrow().theme_mode.clone())
            .unwrap_or_else(|| super::theme::live_theme_mode().unwrap_or_else(|_| "dark".into()))
    }
    pub(crate) fn theme_names(&self) -> Vec<String> {
        self.test()
            .map(|t| t.borrow().themes.clone())
            .unwrap_or_else(|| super::theme::live_theme_names().unwrap_or_default())
    }
    pub(crate) fn set_theme_name(&mut self, name: &str) -> mlua::Result<()> {
        if let Some(test) = self.test() {
            let mut w = test.borrow_mut();
            if !w.themes.iter().any(|n| n == name) {
                return Err(mlua::Error::runtime(format!("unknown theme {name:?}")));
            }
            w.theme_name = name.into();
            w.theme_mode = if name.to_ascii_lowercase().contains("light") {
                "light".into()
            } else {
                "dark".into()
            };
            return Ok(());
        }
        super::theme::apply_theme_name(name).map_err(mlua::Error::runtime)
    }
    pub(crate) fn set_theme_mode(&mut self, mode: &str) -> mlua::Result<()> {
        let mode = super::theme::parse_theme_mode(mode).map_err(mlua::Error::runtime)?;
        if let Some(test) = self.test() {
            let mut w = test.borrow_mut();
            w.theme_mode = mode.name().into();
            let light = mode.name() == "light";
            if let Some(name) = w
                .themes
                .iter()
                .find(|n| n.to_ascii_lowercase().contains("light") == light)
                .cloned()
            {
                w.theme_name = name
            };
            return Ok(());
        }
        super::theme::apply_theme_mode(mode).map_err(mlua::Error::runtime)
    }
}

impl Default for DesktopBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptBackend for DesktopBackend {
    fn session_id(&self, which: Option<SessionId>) -> String {
        match which {
            Some(id) => id.to_string(),
            None => self
                .with_session(|s| s.id().to_string())
                .expect("desktop scripting requires an app view"),
        }
    }
    fn session_path(&self, which: Option<SessionId>) -> Option<String> {
        self.with_session_kind(which, |s| s.path().map(|p| p.display().to_string()))
    }
    fn session_workflow(&self, which: Option<SessionId>) -> Option<String> {
        self.with_session_kind(which, |s| s.workflow().map(str::to_owned))
    }
    fn set_session_workflow(
        &mut self,
        which: Option<SessionId>,
        workflow: Option<String>,
    ) -> mlua::Result<()> {
        self.with_session_kind_mut(which, |s| {
            s.set_workflow(workflow);
            Ok(())
        })
    }
    fn session_capture_ui(&self, which: Option<SessionId>) -> bool {
        self.with_session_kind(which, Session::capture_ui)
    }
    fn set_session_capture_ui(
        &mut self,
        which: Option<SessionId>,
        value: bool,
    ) -> mlua::Result<()> {
        self.with_session_kind_mut(which, |s| {
            s.set_capture_ui(value);
            Ok(())
        })
    }
    fn session_properties(&self, which: Option<SessionId>) -> BTreeMap<String, String> {
        self.with_session_kind(which, |s| s.properties().clone())
    }
    fn set_session_properties(
        &mut self,
        which: Option<SessionId>,
        properties: BTreeMap<String, String>,
    ) -> mlua::Result<()> {
        self.with_session_kind_mut(which, |s| {
            s.set_properties(properties);
            Ok(())
        })
    }
    fn session_active_document(&self, which: Option<SessionId>) -> Option<DocumentId> {
        self.with_session_kind(which, Session::active)
    }
    fn session_documents(&self, which: Option<SessionId>) -> Vec<DocumentId> {
        self.with_session_kind(which, |s| s.documents().iter().map(|d| d.id).collect())
    }
    fn session_group_count(&self, which: Option<SessionId>, group: &str) -> usize {
        self.with_session_kind(which, |s| s.group_count(group))
    }
    fn session_groups(&self, which: Option<SessionId>) -> Vec<String> {
        self.with_session_kind(which, |s| s.groups().to_vec())
    }
    fn set_session_groups(
        &mut self,
        which: Option<SessionId>,
        groups: Vec<String>,
    ) -> mlua::Result<()> {
        self.with_session_kind_mut(which, |s| {
            s.set_groups(groups);
            Ok(())
        })?;
        self.refresh();
        Ok(())
    }
    fn add_session_group(&mut self, which: Option<SessionId>, name: String) -> mlua::Result<()> {
        self.with_session_kind_mut(which, |s| {
            s.add_group(name);
            Ok(())
        })?;
        self.refresh();
        Ok(())
    }
    fn rename_session_group(
        &mut self,
        which: Option<SessionId>,
        old: String,
        new: String,
    ) -> mlua::Result<()> {
        self.with_session_kind_mut(which, |s| {
            if s.rename_group(&old, new) {
                Ok(())
            } else {
                Err(mlua::Error::runtime("could not rename group"))
            }
        })?;
        self.refresh();
        Ok(())
    }
    fn delete_session_group(&mut self, which: Option<SessionId>, name: String) -> mlua::Result<()> {
        self.with_session_kind_mut(which, |s| {
            if s.delete_group(&name) {
                Ok(())
            } else {
                Err(mlua::Error::runtime("group not found"))
            }
        })?;
        self.refresh();
        Ok(())
    }
    fn move_session_group(
        &mut self,
        which: Option<SessionId>,
        name: String,
        index: usize,
    ) -> mlua::Result<()> {
        self.with_session_kind_mut(which, |s| {
            if s.move_group(&name, index) {
                Ok(())
            } else {
                Err(mlua::Error::runtime("group not found"))
            }
        })?;
        self.refresh();
        Ok(())
    }
    fn move_session_document(
        &mut self,
        which: Option<SessionId>,
        id: DocumentId,
        index: usize,
    ) -> mlua::Result<()> {
        self.with_session_kind_mut(which, |s| {
            if s.move_document(id, index) {
                Ok(())
            } else {
                Err(mlua::Error::runtime("composition is not in this session"))
            }
        })?;
        self.refresh();
        Ok(())
    }
    fn set_session_active_document(
        &mut self,
        which: Option<SessionId>,
        id: DocumentId,
    ) -> mlua::Result<bool> {
        if which.is_some() {
            self.with_session_kind_mut(which, |s| {
                s.focus(id);
                Ok(())
            })?;
            return Ok(false);
        }
        if let Some(test) = self.test() {
            let mut world = test.borrow_mut();
            if world.session.get(id).is_none() {
                return Err(mlua::Error::runtime("composition is not open"));
            }
            // `focus` returns None when already active; that is still success.
            let changed = world.session.focus(id).is_some();
            world.active = Some(id);
            return Ok(changed);
        }
        // Live desktop: activate without firing hooks under the backend
        // borrow; HostHandle emits composition_selected after release.
        let changed =
            access::with_view(|view, window, cx| view.script_activate_document(id, window, cx))
                .map_err(mlua::Error::runtime)?
                .map_err(mlua::Error::runtime)?;
        Ok(changed)
    }

    fn open_path(&mut self, path: &Path) -> mlua::Result<DocumentId> {
        if is_fasession_path(path) {
            return Err(mlua::Error::runtime(
                "use field.session.open for .fasession files",
            ));
        }
        if let Some(test) = self.test() {
            return open_in_test(&test, path);
        }
        access::with_view(|view, window, cx| view.script_open(path.to_path_buf(), window, cx))
            .map_err(mlua::Error::runtime)?
            .map_err(mlua::Error::runtime)
    }
    fn reset_session(&mut self) -> mlua::Result<()> {
        if let Some(test) = self.test() {
            let mut w = test.borrow_mut();
            w.docs.clear();
            w.session = Session::new();
            w.active = None;
            return Ok(());
        }
        Err(mlua::Error::runtime(
            "reset_session is not available in FieldAssist",
        ))
    }
    fn open_detached_session(&mut self, path: &Path) -> mlua::Result<SessionId> {
        let json =
            std::fs::read_to_string(path).map_err(|e| mlua::Error::runtime(e.to_string()))?;
        let loaded = Session::from_json(&json, Some(path))
            .map_err(|e| mlua::Error::runtime(e.to_string()))?;
        let id = loaded.session.id();
        self.inner
            .borrow_mut()
            .detached_sessions
            .insert(id, loaded.session);
        Ok(id)
    }
    fn new_detached_session(&mut self) -> SessionId {
        let s = Session::new();
        let id = s.id();
        self.inner.borrow_mut().detached_sessions.insert(id, s);
        id
    }
    fn load_shared_session_file(&mut self, _path: &Path) -> mlua::Result<()> {
        Err(mlua::Error::runtime(
            "load_shared_session_file is not available in FieldAssist",
        ))
    }
    fn save_session(
        &mut self,
        which: Option<SessionId>,
        path: Option<PathBuf>,
    ) -> mlua::Result<()> {
        if which.is_some() {
            return Err(mlua::Error::runtime(
                "cannot save detached sessions in FieldAssist",
            ));
        }
        if let Some(test) = self.test() {
            let mut world = test.borrow_mut();
            let dest = path
                .or_else(|| world.session.path().map(Path::to_path_buf))
                .ok_or_else(|| mlua::Error::runtime("session has no path; call save_as"))?;
            let names = world.names.clone();
            world
                .session
                .save_to_path(
                    &dest,
                    |id| names.get(&id).cloned().unwrap_or_else(|| id.to_string()),
                    None,
                )
                .map_err(|e| mlua::Error::runtime(e.to_string()))?;
            return Ok(());
        }
        access::with_view(|view, window, cx| match path {
            Some(path) => view.script_save_session_to(path, window, cx),
            None => view.script_save_session(window, cx),
        })
        .map_err(mlua::Error::runtime)?
        .map_err(mlua::Error::runtime)
    }
    fn close_detached_session(&mut self, id: SessionId) -> mlua::Result<()> {
        self.inner
            .borrow_mut()
            .detached_sessions
            .remove(&id)
            .map(|_| ())
            .ok_or_else(|| mlua::Error::runtime("session is not loaded"))
    }

    fn document_group(&self, id: DocumentId) -> mlua::Result<Option<String>> {
        Ok(self
            .with_session(|s| s.get(id).and_then(|d| d.group.clone()))
            .flatten())
    }
    fn set_document_group(&mut self, id: DocumentId, group: Option<String>) -> mlua::Result<()> {
        // Use Session::set_document_group so the group registry is updated
        // (ensure_group). Assigning `doc.group` alone leaves compositions
        // invisible in the explorer, which only lists registered groups.
        self.with_session_mut(|s| {
            if s.set_document_group(id, group) {
                Ok(())
            } else {
                Err(mlua::Error::runtime("composition is not open"))
            }
        })?;
        self.refresh();
        Ok(())
    }
    fn document_state(&self, id: DocumentId) -> mlua::Result<Option<String>> {
        Ok(self
            .with_session(|s| s.get(id).and_then(|d| d.state.clone()))
            .flatten())
    }
    fn set_document_state(&mut self, id: DocumentId, state: Option<String>) -> mlua::Result<()> {
        self.with_session_mut(|s| {
            let d = s
                .get_mut(id)
                .ok_or_else(|| mlua::Error::runtime("composition is not open"))?;
            d.state = state;
            s.mark_dirty();
            Ok(())
        })
    }
    fn document_properties(&self, id: DocumentId) -> mlua::Result<BTreeMap<String, String>> {
        Ok(self
            .with_session(|s| s.get(id).map(|d| d.properties.clone()).unwrap_or_default())
            .unwrap_or_default())
    }
    fn set_document_properties(
        &mut self,
        id: DocumentId,
        properties: BTreeMap<String, String>,
    ) -> mlua::Result<()> {
        self.with_session_mut(|s| {
            let d = s
                .get_mut(id)
                .ok_or_else(|| mlua::Error::runtime("composition is not open"))?;
            d.properties = properties;
            s.mark_dirty();
            Ok(())
        })
    }
    fn display_name(&self, id: DocumentId) -> Option<String> {
        if let Some(test) = self.test() {
            let world = test.borrow();
            if let Some(name) = world.names.get(&id).cloned() {
                return Some(name);
            }
            if let Some(name) = world.session.get(id).and_then(|doc| {
                doc.name.clone().or_else(|| {
                    doc.file_path()
                        .and_then(|path| path.file_stem())
                        .map(|stem| stem.to_string_lossy().into_owned())
                })
            }) {
                return Some(name);
            }
        } else if let Ok(Some(name)) = access::with_view(|v, _, cx| v.script_display_name(id, cx)) {
            return Some(name);
        }
        self.detached_document(id).and_then(|doc| {
            doc.name.clone().or_else(|| {
                doc.file_path()
                    .and_then(|path| path.file_stem())
                    .map(|stem| stem.to_string_lossy().into_owned())
            })
        })
    }
    fn document_path(&self, id: DocumentId) -> Option<PathBuf> {
        if let Some(test) = self.test() {
            let world = test.borrow();
            if let Some(path) = world.paths.get(&id).cloned().flatten() {
                return Some(path);
            }
            if let Some(path) = world
                .session
                .get(id)
                .and_then(|doc| doc.file_path().map(Path::to_path_buf))
            {
                return Some(path);
            }
        } else if let Ok(Some(path)) = access::with_view(|v, _, _| v.script_path(id)) {
            return Some(path);
        }
        self.detached_document(id)
            .and_then(|doc| doc.file_path().map(Path::to_path_buf))
    }
    fn set_display_name(&mut self, id: DocumentId, name: String) -> mlua::Result<()> {
        if let Some(test) = self.test() {
            test.borrow_mut().names.insert(id, name);
            return Ok(());
        }
        access::with_view(|v, w, cx| v.script_set_display_name(id, name, w, cx))
            .map_err(mlua::Error::runtime)?
    }
    fn composition_id_string(&self, id: DocumentId) -> mlua::Result<String> {
        self.with_buffer(id, |_| Ok(id.to_string()))
    }
    fn close_composition(&mut self, id: DocumentId) -> mlua::Result<()> {
        if let Some(test) = self.test() {
            let mut w = test.borrow_mut();
            if !w.docs.contains_key(&id) {
                return Err(mlua::Error::runtime("composition is not open"));
            }
            w.close(id);
            return Ok(());
        }
        access::with_view(|v, w, cx| v.script_close_document(id, w, cx))
            .map_err(mlua::Error::runtime)?;
        Ok(())
    }
    fn replace_composition(&mut self, id: DocumentId, path: &Path) -> mlua::Result<DocumentId> {
        if let Some(test) = self.test() {
            return replace_in_test(&test, id, path);
        }
        access::with_view(|v, w, cx| v.script_replace_document(id, path.to_path_buf(), w, cx))
            .map_err(mlua::Error::runtime)?
            .map_err(mlua::Error::runtime)
    }
    fn with_open_document(
        &self,
        id: DocumentId,
        f: &mut dyn FnMut(&OpenDocument) -> mlua::Result<()>,
    ) -> mlua::Result<()> {
        let name = self.display_name(id).unwrap_or_else(|| id.to_string());
        let path = self.document_path(id);
        self.with_buffer(id, |doc| {
            let open = Self::open_from_buffer(doc, name, path);
            f(&open)
        })
    }
    fn with_open_document_mut(
        &mut self,
        id: DocumentId,
        f: &mut dyn FnMut(&mut OpenDocument) -> mlua::Result<()>,
    ) -> mlua::Result<()> {
        let name = self.display_name(id).unwrap_or_else(|| id.to_string());
        let path = self.document_path(id);
        self.with_buffer_mut(id, |doc| {
            let mut open = Self::open_from_buffer(doc, name, path);
            f(&mut open)?;
            Self::apply_open_buffer(doc, &open);
            Ok(())
        })
    }
    fn apply_document_channel_layout(
        &mut self,
        id: DocumentId,
        name: Option<&str>,
        labels: BTreeMap<usize, String>,
        default_chain: Option<String>,
    ) -> mlua::Result<()> {
        self.with_buffer_mut(id, |doc| {
            let mut c = doc.composition.write().unwrap();
            c.apply_channel_layout(name.map(str::to_owned), labels);
            if c.monitor_chain().is_none() {
                c.set_monitor_chain(default_chain);
            }
            Ok(())
        })
    }
    fn add_media(&mut self, path: &Path) -> mlua::Result<MediaId> {
        if let Some(test) = self.test() {
            return crate::media_pool::add_media(&test.borrow().media_store, path)
                .map(|r| r.id)
                .map_err(mlua::Error::runtime);
        }
        access::with_view(|v, _, cx| v.script_add_media(path.to_path_buf(), cx))
            .map_err(mlua::Error::runtime)?
            .map(|r| r.id)
            .map_err(mlua::Error::runtime)
    }
    fn remove_media(&mut self, id: MediaId) -> mlua::Result<()> {
        if let Some(test) = self.test() {
            let w = test.borrow();
            let mut refs = crate::media_pool::referenced_media_ids(
                &w.session,
                std::iter::empty::<&Composition>(),
            );
            for d in w.docs.values() {
                refs.extend(d.composition.read().unwrap().used_media_ids());
            }
            return crate::media_pool::remove_media(&w.media_store, id, &refs)
                .map(|_| ())
                .map_err(mlua::Error::runtime);
        }
        access::with_view(|v, _, cx| v.script_remove_media(id, cx))
            .map_err(mlua::Error::runtime)?
            .map_err(mlua::Error::runtime)
    }
    fn list_media(&self) -> Vec<MediaId> {
        if let Some(test) = self.test() {
            let world = test.borrow();
            let mut ids: Vec<_> = crate::media_pool::list_media(&world.media_store)
                .into_iter()
                .map(|row| row.id)
                .collect();
            for doc in world.docs.values() {
                for media in doc.composition.read().unwrap().pool().iter() {
                    if !ids.contains(&media.id) {
                        ids.push(media.id);
                    }
                }
            }
            return ids;
        }
        access::with_view(|v, _, cx| v.script_list_media(cx).into_iter().map(|r| r.id).collect())
            .unwrap_or_default()
    }
    fn get_media(&self, id: MediaId) -> mlua::Result<MediaRef> {
        let store: Arc<Mutex<MediaStore>> = if let Some(test) = self.test() {
            let world = test.borrow();
            for doc in world.docs.values() {
                if let Some(media) = doc
                    .composition
                    .read()
                    .unwrap()
                    .pool()
                    .iter()
                    .find(|media| media.id == id)
                    .cloned()
                {
                    return Ok(media);
                }
            }
            world.media_store.clone()
        } else {
            return Err(mlua::Error::runtime("media lookup requires app access"));
        };
        let media = store
            .lock()
            .unwrap()
            .pool()
            .iter()
            .find(|m| m.id == id)
            .cloned();
        media.ok_or_else(|| mlua::Error::runtime(format!("unknown media id: {id}")))
    }
    fn media_store(&self) -> Option<Arc<Mutex<MediaStore>>> {
        self.test().map(|t| t.borrow().media_store.clone())
    }
    fn toolbar_changed(&mut self) -> mlua::Result<()> {
        if self.test().is_none() {
            let _ = access::with_view(|v, _, cx| v.refresh_workflow_bar(cx));
        }
        Ok(())
    }
    fn composition_codec(&self, id: DocumentId) -> mlua::Result<Option<String>> {
        self.with_buffer(id, |d| Ok(d.composition.read().unwrap().codec()))
    }
    fn composition_channel_layout(&self, id: DocumentId) -> mlua::Result<Option<String>> {
        self.with_buffer(id, |d| {
            Ok(d.composition
                .read()
                .unwrap()
                .channel_layout()
                .map(str::to_owned))
        })
    }
    fn composition_monitor_chain(&self, id: DocumentId) -> mlua::Result<Option<String>> {
        self.with_buffer(id, |d| {
            Ok(d.composition
                .read()
                .unwrap()
                .monitor_chain()
                .map(str::to_owned))
        })
    }
    fn set_composition_monitor_chain(
        &mut self,
        id: DocumentId,
        chain: Option<String>,
    ) -> mlua::Result<()> {
        self.with_buffer_mut(id, |d| {
            d.composition.write().unwrap().set_monitor_chain(chain);
            Ok(())
        })
    }
    fn composition_playback_channels(&self, id: DocumentId) -> mlua::Result<Option<Vec<usize>>> {
        self.with_buffer(id, |d| {
            Ok(d.composition
                .read()
                .unwrap()
                .playback_channels()
                .map(|v| v.to_vec()))
        })
    }
    fn set_composition_playback_channels(
        &mut self,
        id: DocumentId,
        channels: Option<Vec<usize>>,
    ) -> mlua::Result<()> {
        self.with_buffer_mut(id, |d| {
            d.composition
                .write()
                .unwrap()
                .set_playback_channels(channels);
            Ok(())
        })
    }
}

fn open_in_test(test: &Rc<RefCell<TestWorld>>, path: &Path) -> mlua::Result<DocumentId> {
    if let Some(id) = test.borrow().session.find_by_path(path) {
        let mut world = test.borrow_mut();
        world.session.focus(id);
        world.active = Some(id);
        return Ok(id);
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let mut world = test.borrow_mut();
    let id = world.push(
        Composition::new(44_100, 2),
        crate::model::Buffer::empty(),
        name,
        Some(path.to_path_buf()),
    );
    if is_facomp_path(path) {
        world.session.set_project_path(id, path.to_path_buf());
    }
    Ok(id)
}

fn replace_in_test(
    test: &Rc<RefCell<TestWorld>>,
    id: DocumentId,
    path: &Path,
) -> mlua::Result<DocumentId> {
    let mut world = test.borrow_mut();
    if world.session.get(id).is_none() {
        return Err(mlua::Error::runtime("composition is not open"));
    }
    if let Some(existing) = world.session.find_by_path(path) {
        world.session.focus(existing);
        world.active = Some(existing);
        return Ok(existing);
    }
    world.session.replace_document_path(id, path.to_path_buf());
    world.paths.insert(id, Some(path.to_path_buf()));
    world.names.insert(
        id,
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
    );
    world.session.focus(id);
    world.active = Some(id);
    Ok(id)
}
