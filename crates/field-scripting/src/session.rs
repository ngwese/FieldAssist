// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Session userdata and `field.session` module.

use std::collections::BTreeMap;
use std::path::PathBuf;

use mlua::{FromLua, Lua, Table, UserData, UserDataFields, UserDataMethods, Value};

use field_session::{DocumentId, SessionId};

use crate::composition::LuaComposition;
use crate::host::{host_from_lua, HostHandle};
use crate::util::{optional_lua_string, string_map_from_lua, string_map_to_lua};

/// Handle to a session in the scripting backend.
///
/// `detached_id = None` → the focused world/UI session (what
/// `field.session.focused()` returns).
///
/// `detached_id = Some(id)` → a session from `field.session.new()` /
/// `field.session.open()`; it lives in the backend's detached map and does
/// not replace the focused session. In Phase 1 its documents are
/// session-manifest entries only and are not opened into the world.
#[derive(Clone, Copy, Debug)]
pub struct LuaSession {
    /// `None` = focused world session; `Some(id)` = detached session.
    pub(crate) detached_id: Option<SessionId>,
}

impl LuaSession {
    /// Focused world/UI session (`field.session.focused()`).
    pub fn focused() -> Self {
        Self { detached_id: None }
    }
}

impl FromLua for LuaSession {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(data) => data.borrow::<Self>().map(|this| *this),
            _ => Err(mlua::Error::runtime("expected a session")),
        }
    }
}

impl UserData for LuaSession {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("id", |lua, this| {
            let host = host_from_lua(lua)?;
            Ok(host.session_id(this.detached_id))
        });
        fields.add_field_method_get("url", |lua, this| {
            Ok(host_from_lua(lua)?
                .session_path(this.detached_id)
                .map(|path| crate::url::LuaUrl::from_path(std::path::Path::new(&path))))
        });
        fields.add_field_method_get("workflow_name", |lua, this| {
            Ok(host_from_lua(lua)?.session_workflow(this.detached_id))
        });
        fields.add_field_method_set("workflow_name", |lua, this, value: Value| {
            host_from_lua(lua)?.set_session_workflow(this.detached_id, optional_lua_string(value)?)
        });
        fields.add_field_method_get("capture_ui", |lua, this| {
            Ok(host_from_lua(lua)?.session_capture_ui(this.detached_id))
        });
        fields.add_field_method_set("capture_ui", |lua, this, value: bool| {
            host_from_lua(lua)?.set_session_capture_ui(this.detached_id, value)
        });
        fields.add_field_method_get("properties", |lua, this| {
            string_map_to_lua(
                lua,
                &host_from_lua(lua)?.session_properties(this.detached_id),
            )
        });
        fields.add_field_method_set("properties", |lua, this, value: Value| {
            host_from_lua(lua)?
                .set_session_properties(this.detached_id, string_map_from_lua(value)?)
        });
        fields.add_field_method_get("composition", |lua, this| {
            Ok(host_from_lua(lua)?
                .session_active_document(this.detached_id)
                .map(|id| LuaComposition { id }))
        });
        fields.add_field_method_set("composition", |lua, this, value: Value| {
            let doc = LuaComposition::from_lua(value, lua)?;
            host_from_lua(lua)?.set_active_document(this.detached_id, doc.id)
        });
        fields.add_field_method_get("compositions", |lua, this| {
            let docs: Vec<LuaComposition> = host_from_lua(lua)?
                .session_documents(this.detached_id)
                .into_iter()
                .map(|id| LuaComposition { id })
                .collect();
            Ok(docs)
        });
        fields.add_field_method_get("groups", |lua, this| {
            let groups = host_from_lua(lua)?.session_groups(this.detached_id);
            let table = lua.create_table()?;
            for (index, name) in groups.into_iter().enumerate() {
                table.set(index + 1, name)?;
            }
            Ok(table)
        });
        fields.add_field_method_set("groups", |lua, this, value: Value| {
            host_from_lua(lua)?.set_session_groups(this.detached_id, string_list_from_lua(value)?)
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("group_count", |lua, this, group: String| {
            Ok(host_from_lua(lua)?.session_group_count(this.detached_id, &group) as i64)
        });
        methods.add_method("add_group", |lua, this, name: String| {
            host_from_lua(lua)?.add_session_group(this.detached_id, name)
        });
        methods.add_method("rename_group", |lua, this, (old, new): (String, String)| {
            host_from_lua(lua)?.rename_session_group(this.detached_id, old, new)
        });
        methods.add_method("delete_group", |lua, this, name: String| {
            host_from_lua(lua)?.delete_session_group(this.detached_id, name)
        });
        methods.add_method("move_group", |lua, this, (name, index): (String, i64)| {
            host_from_lua(lua)?.move_session_group(this.detached_id, name, index)
        });
        methods.add_method("move", |lua, this, (doc, index): (LuaComposition, i64)| {
            host_from_lua(lua)?.move_session_document(this.detached_id, doc.id, index)
        });
        methods.add_method("open", |lua, _, path: Value| {
            // `session.open(path)` opens a *document* into the **focused** session.
            // To open a .fasession file, use `field.session.open("…")` (module-level).
            let path = crate::fs::path_from_lua(path)?;
            host_from_lua(lua)?
                .open_path(&path.to_string_lossy())
                .map(|id| LuaComposition { id })
        });
        methods.add_method("save", |lua, this, ()| {
            host_from_lua(lua)?.save_session(this.detached_id, None)
        });
        methods.add_method("save_as", |lua, this, path: Value| {
            let path = crate::fs::path_from_lua(path)?;
            host_from_lua(lua)?
                .save_session(this.detached_id, Some(path.to_string_lossy().into_owned()))
        });
        methods.add_method("close", |lua, this, ()| -> mlua::Result<()> {
            let Some(id) = this.detached_id else {
                return Err(mlua::Error::runtime("cannot close the focused session"));
            };
            host_from_lua(lua)?.close_detached_session(id)
        });
    }
}

/// Install `field.session`.
pub fn bind_session_module(lua: &Lua, field: &Table) -> mlua::Result<()> {
    let session = lua.create_table()?;
    session.set(
        "focused",
        lua.create_function(|_, ()| Ok(LuaSession::focused()))?,
    )?;
    session.set(
        "new",
        lua.create_function(|lua, ()| {
            Ok(LuaSession {
                detached_id: Some(host_from_lua(lua)?.new_detached_session()),
            })
        })?,
    )?;
    session.set(
        "open",
        lua.create_function(|lua, path: Value| {
            // `field.session.open` loads a .fasession into the detached map
            // and returns a LuaSession identified by that session's id.
            // The focused world session is NOT replaced.
            // To replace the focused session (batch style), use
            // `field.session.load(path)` (internal / Phase-2 API).
            let path = crate::fs::path_from_lua(path)?;
            let detached_id = host_from_lua(lua)?.open_detached_session(&path.to_string_lossy())?;
            Ok(LuaSession {
                detached_id: Some(detached_id),
            })
        })?,
    )?;
    field.set("session", session)?;
    Ok(())
}

// ── HostHandle session methods ────────────────────────────────────────────────

impl HostHandle {
    pub(crate) fn session_id(&self, which: Option<SessionId>) -> String {
        self.with_backend(|b| b.session_id(which))
    }

    pub(crate) fn session_path(&self, which: Option<SessionId>) -> Option<String> {
        self.with_backend(|b| b.session_path(which))
    }

    pub(crate) fn session_workflow(&self, which: Option<SessionId>) -> Option<String> {
        self.with_backend(|b| b.session_workflow(which))
    }

    pub(crate) fn set_session_workflow(
        &self,
        which: Option<SessionId>,
        workflow: Option<String>,
    ) -> mlua::Result<()> {
        self.with_backend_mut(|b| b.set_session_workflow(which, workflow))
    }

    pub(crate) fn session_capture_ui(&self, which: Option<SessionId>) -> bool {
        self.with_backend(|b| b.session_capture_ui(which))
    }

    pub(crate) fn set_session_capture_ui(
        &self,
        which: Option<SessionId>,
        capture_ui: bool,
    ) -> mlua::Result<()> {
        self.with_backend_mut(|b| b.set_session_capture_ui(which, capture_ui))
    }

    pub(crate) fn session_properties(&self, which: Option<SessionId>) -> BTreeMap<String, String> {
        self.with_backend(|b| b.session_properties(which))
    }

    pub(crate) fn set_session_properties(
        &self,
        which: Option<SessionId>,
        properties: BTreeMap<String, String>,
    ) -> mlua::Result<()> {
        self.with_backend_mut(|b| b.set_session_properties(which, properties))
    }

    pub(crate) fn session_active_document(&self, which: Option<SessionId>) -> Option<DocumentId> {
        self.with_backend(|b| b.session_active_document(which))
    }

    pub(crate) fn session_documents(&self, which: Option<SessionId>) -> Vec<DocumentId> {
        self.with_backend(|b| b.session_documents(which))
    }

    pub(crate) fn session_group_count(&self, which: Option<SessionId>, group: &str) -> usize {
        self.with_backend(|b| b.session_group_count(which, group))
    }

    pub(crate) fn session_groups(&self, which: Option<SessionId>) -> Vec<String> {
        self.with_backend(|b| b.session_groups(which))
    }

    pub(crate) fn set_session_groups(
        &self,
        which: Option<SessionId>,
        groups: Vec<String>,
    ) -> mlua::Result<()> {
        self.with_backend_mut(|b| b.set_session_groups(which, groups))
    }

    pub(crate) fn add_session_group(
        &self,
        which: Option<SessionId>,
        name: String,
    ) -> mlua::Result<()> {
        self.with_backend_mut(|b| b.add_session_group(which, name))
    }

    pub(crate) fn rename_session_group(
        &self,
        which: Option<SessionId>,
        old: String,
        new: String,
    ) -> mlua::Result<()> {
        self.with_backend_mut(|b| b.rename_session_group(which, old, new))
    }

    pub(crate) fn delete_session_group(
        &self,
        which: Option<SessionId>,
        name: String,
    ) -> mlua::Result<()> {
        self.with_backend_mut(|b| b.delete_session_group(which, name))
    }

    pub(crate) fn move_session_group(
        &self,
        which: Option<SessionId>,
        name: String,
        index: i64,
    ) -> mlua::Result<()> {
        self.with_backend_mut(|b| b.move_session_group(which, name, index.max(0) as usize))
    }

    pub(crate) fn move_session_document(
        &self,
        which: Option<SessionId>,
        id: DocumentId,
        index: i64,
    ) -> mlua::Result<()> {
        let len = self.with_backend(|b| b.session_documents(which).len());
        if index < 1 || (len > 0 && index as usize > len) {
            return Err(mlua::Error::runtime(format!(
                "move index must be between 1 and {len}"
            )));
        }
        self.with_backend_mut(|b| {
            b.move_session_document(which, id, index.saturating_sub(1) as usize)
        })
    }

    pub(crate) fn set_active_document(
        &self,
        which: Option<SessionId>,
        id: DocumentId,
    ) -> mlua::Result<()> {
        // Clone-before-hooks: with_backend_mut releases HostInner before the
        // backend runs, so desktop focus_document can fire composition_selected
        // without RefCell re-borrow panics.
        let should_emit = self.with_backend_mut(|b| b.set_session_active_document(which, id))?;
        if which.is_none() && should_emit {
            self.emit_composition_selected(Some(id));
        }
        Ok(())
    }

    pub(crate) fn close_detached_session(&self, id: SessionId) -> mlua::Result<()> {
        self.with_backend_mut(|b| b.close_detached_session(id))
    }

    pub(crate) fn open_path(&self, path: &str) -> mlua::Result<DocumentId> {
        let path = PathBuf::from(path);
        self.with_backend_mut(|b| b.open_path(&path))
    }

    #[allow(dead_code)]
    pub(crate) fn reset_session(&self) -> mlua::Result<()> {
        self.with_backend_mut(|b| b.reset_session())?;
        self.inner.borrow_mut().active = None;
        Ok(())
    }

    pub(crate) fn open_detached_session(&self, path: &str) -> mlua::Result<SessionId> {
        let path = PathBuf::from(path);
        self.with_backend_mut(|b| b.open_detached_session(&path))
    }

    pub(crate) fn new_detached_session(&self) -> SessionId {
        self.with_backend_mut(|b| b.new_detached_session())
    }

    pub(crate) fn save_session(
        &self,
        which: Option<SessionId>,
        path: Option<String>,
    ) -> mlua::Result<()> {
        self.with_backend_mut(|b| b.save_session(which, path.map(PathBuf::from)))
    }

    pub(crate) fn document_group(&self, id: DocumentId) -> mlua::Result<Option<String>> {
        self.with_backend(|b| b.document_group(id))
    }

    pub(crate) fn set_document_group(
        &self,
        id: DocumentId,
        group: Option<String>,
    ) -> mlua::Result<()> {
        self.with_backend_mut(|b| b.set_document_group(id, group))
    }

    pub(crate) fn document_state(&self, id: DocumentId) -> mlua::Result<Option<String>> {
        self.with_backend(|b| b.document_state(id))
    }

    pub(crate) fn set_document_state(
        &self,
        id: DocumentId,
        state: Option<String>,
    ) -> mlua::Result<()> {
        self.with_backend_mut(|b| b.set_document_state(id, state))
    }

    pub(crate) fn document_properties(
        &self,
        id: DocumentId,
    ) -> mlua::Result<BTreeMap<String, String>> {
        self.with_backend(|b| b.document_properties(id))
    }

    pub(crate) fn set_document_properties(
        &self,
        id: DocumentId,
        properties: BTreeMap<String, String>,
    ) -> mlua::Result<()> {
        self.with_backend_mut(|b| b.set_document_properties(id, properties))
    }

    pub(crate) fn display_name(&self, id: DocumentId) -> Option<String> {
        self.with_backend(|b| b.display_name(id))
    }

    pub(crate) fn set_display_name(&self, id: DocumentId, name: String) -> mlua::Result<()> {
        self.with_backend_mut(|b| b.set_display_name(id, name))
    }

    pub(crate) fn document_path(&self, id: DocumentId) -> Option<PathBuf> {
        self.with_backend(|b| b.document_path(id))
    }

    pub(crate) fn composition_id_string(&self, id: DocumentId) -> mlua::Result<String> {
        self.with_backend(|b| b.composition_id_string(id))
    }

    pub(crate) fn close_composition(&self, id: DocumentId) -> mlua::Result<()> {
        self.with_backend_mut(|b| b.close_composition(id))
    }

    pub(crate) fn replace_composition(
        &self,
        id: DocumentId,
        path: &str,
    ) -> mlua::Result<DocumentId> {
        let path = PathBuf::from(path);
        self.with_backend_mut(|b| b.replace_composition(id, &path))
    }
}

fn string_list_from_lua(value: Value) -> mlua::Result<Vec<String>> {
    match value {
        Value::Table(table) => {
            let mut out = Vec::new();
            for pair in table.sequence_values::<Value>() {
                match pair? {
                    Value::String(value) => out.push(value.to_str()?.to_owned()),
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "groups entries must be strings, got {}",
                            other.type_name()
                        )))
                    }
                }
            }
            Ok(out)
        }
        other => Err(mlua::Error::runtime(format!(
            "groups must be a table, got {}",
            other.type_name()
        ))),
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;
    use crate::host::{HostProfile, ScriptHost};

    #[test]
    fn session_focused_exposes_id_and_properties() {
        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: None,
        })
        .unwrap();
        let out = host.eval(
            r#"(function() local s=field.session.focused(); s.properties={mode="test"}; return s.properties.mode, s.id~=nil end)()"#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("test\ttrue"), "{:?}", out.error);
    }
}
