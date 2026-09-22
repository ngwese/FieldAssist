// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Session userdata and `field.session` module.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use mlua::{FromLua, Lua, Table, UserData, UserDataFields, UserDataMethods, Value};

use field_composition::Composition;
use field_session::{is_fasession_path, DocumentId, Session, SessionDocument};

use crate::composition::LuaComposition;
use crate::host::{host_from_lua, HostHandle};
use crate::util::{optional_lua_string, string_map_from_lua, string_map_to_lua};
use crate::world::HeadlessWorld;

/// Handle to the headless [`HeadlessWorld`] session.
#[derive(Clone, Copy, Debug)]
pub struct LuaSession;

impl LuaSession {
    /// Active shared session in the host world.
    pub fn shared() -> Self {
        Self
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
        fields.add_field_method_get("id", |lua, _| {
            let host = host_from_lua(lua)?;
            Ok(host.session_id())
        });
        fields.add_field_method_get("path", |lua, _| Ok(host_from_lua(lua)?.session_path()));
        fields.add_field_method_get("workflow_name", |lua, _| {
            Ok(host_from_lua(lua)?.session_workflow())
        });
        fields.add_field_method_set("workflow_name", |lua, _, value: Value| {
            host_from_lua(lua)?.set_session_workflow(optional_lua_string(value)?)
        });
        fields.add_field_method_get("capture_ui", |lua, _| {
            Ok(host_from_lua(lua)?.session_capture_ui())
        });
        fields.add_field_method_set("capture_ui", |lua, _, value: bool| {
            host_from_lua(lua)?.set_session_capture_ui(value)
        });
        fields.add_field_method_get("properties", |lua, _| {
            string_map_to_lua(lua, &host_from_lua(lua)?.session_properties())
        });
        fields.add_field_method_set("properties", |lua, _, value: Value| {
            host_from_lua(lua)?.set_session_properties(string_map_from_lua(value)?)
        });
        fields.add_field_method_get("composition", |lua, _| {
            Ok(host_from_lua(lua)?
                .session_active_document()
                .map(|id| LuaComposition { id }))
        });
        fields.add_field_method_set("composition", |lua, _, value: Value| {
            let doc = LuaComposition::from_lua(value, lua)?;
            host_from_lua(lua)?.set_active_document(doc.id)
        });
        fields.add_field_method_get("compositions", |lua, _| {
            let docs: Vec<LuaComposition> = host_from_lua(lua)?
                .session_documents()
                .into_iter()
                .map(|id| LuaComposition { id })
                .collect();
            Ok(docs)
        });
        fields.add_field_method_get("groups", |lua, _| {
            let groups = host_from_lua(lua)?.session_groups();
            let table = lua.create_table()?;
            for (index, name) in groups.into_iter().enumerate() {
                table.set(index + 1, name)?;
            }
            Ok(table)
        });
        fields.add_field_method_set("groups", |lua, _, value: Value| {
            host_from_lua(lua)?.set_session_groups(string_list_from_lua(value)?)
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("group_count", |lua, _, group: String| {
            Ok(host_from_lua(lua)?.session_group_count(&group) as i64)
        });
        methods.add_method("add_group", |lua, _, name: String| {
            host_from_lua(lua)?.add_session_group(name)
        });
        methods.add_method("rename_group", |lua, _, (old, new): (String, String)| {
            host_from_lua(lua)?.rename_session_group(old, new)
        });
        methods.add_method("delete_group", |lua, _, name: String| {
            host_from_lua(lua)?.delete_session_group(name)
        });
        methods.add_method("move_group", |lua, _, (name, index): (String, i64)| {
            host_from_lua(lua)?.move_session_group(name, index)
        });
        methods.add_method("move", |lua, _, (doc, index): (LuaComposition, i64)| {
            host_from_lua(lua)?.move_session_document(doc.id, index)
        });
        methods.add_method("open", |lua, _, path: String| {
            host_from_lua(lua)?
                .open_path(&path)
                .map(|id| LuaComposition { id })
        });
        methods.add_method("save", |lua, _, ()| {
            host_from_lua(lua)?.save_session_stub(None)
        });
        methods.add_method("save_as", |lua, _, path: String| {
            host_from_lua(lua)?.save_session_stub(Some(path))
        });
        methods.add_method("close", |_lua, _, ()| -> mlua::Result<()> {
            Err(mlua::Error::runtime(
                "session.close is not available in the headless host",
            ))
        });
    }
}

/// Install `field.session`.
pub fn bind_session_module(lua: &Lua, field: &Table) -> mlua::Result<()> {
    let session = lua.create_table()?;
    session.set(
        "shared",
        lua.create_function(|_, ()| Ok(LuaSession::shared()))?,
    )?;
    session.set(
        "new",
        lua.create_function(|lua, ()| {
            host_from_lua(lua)?.reset_session()?;
            Ok(LuaSession::shared())
        })?,
    )?;
    session.set(
        "open",
        lua.create_function(|lua, path: String| {
            host_from_lua(lua)?.load_session_file(&path)?;
            Ok(LuaSession::shared())
        })?,
    )?;
    field.set("session", session)?;
    Ok(())
}

impl HostHandle {
    pub(crate) fn session_id(&self) -> String {
        self.with_world(|world| world.session.id().to_string())
    }

    pub(crate) fn session_path(&self) -> Option<String> {
        self.with_world(|world| world.session.path().map(|path| path.display().to_string()))
    }

    pub(crate) fn session_workflow(&self) -> Option<String> {
        self.with_world(|world| world.session.workflow().map(str::to_string))
    }

    pub(crate) fn set_session_workflow(&self, workflow: Option<String>) -> mlua::Result<()> {
        self.with_world_mut(|world| world.session.set_workflow(workflow));
        Ok(())
    }

    pub(crate) fn session_capture_ui(&self) -> bool {
        self.with_world(|world| world.session.capture_ui())
    }

    pub(crate) fn set_session_capture_ui(&self, capture_ui: bool) -> mlua::Result<()> {
        self.with_world_mut(|world| world.session.set_capture_ui(capture_ui));
        Ok(())
    }

    pub(crate) fn session_properties(&self) -> BTreeMap<String, String> {
        self.with_world(|world| world.session.properties().clone())
    }

    pub(crate) fn set_session_properties(
        &self,
        properties: BTreeMap<String, String>,
    ) -> mlua::Result<()> {
        self.with_world_mut(|world| world.session.set_properties(properties));
        Ok(())
    }

    pub(crate) fn session_active_document(&self) -> Option<DocumentId> {
        self.with_world(|world| world.session.active())
    }

    pub(crate) fn session_documents(&self) -> Vec<DocumentId> {
        self.with_world(|world| world.session.documents().iter().map(|doc| doc.id).collect())
    }

    pub(crate) fn session_group_count(&self, group: &str) -> usize {
        self.with_world(|world| world.session.group_count(group))
    }

    pub(crate) fn session_groups(&self) -> Vec<String> {
        self.with_world(|world| world.session.groups().to_vec())
    }

    pub(crate) fn set_session_groups(&self, groups: Vec<String>) -> mlua::Result<()> {
        self.with_world_mut(|world| world.session.set_groups(groups));
        Ok(())
    }

    pub(crate) fn add_session_group(&self, name: String) -> mlua::Result<()> {
        self.with_world_mut(|world| world.session.add_group(name));
        Ok(())
    }

    pub(crate) fn rename_session_group(&self, old: String, new: String) -> mlua::Result<()> {
        self.with_world_mut(|world| world.session.rename_group(&old, &new));
        Ok(())
    }

    pub(crate) fn delete_session_group(&self, name: String) -> mlua::Result<()> {
        self.with_world_mut(|world| world.session.delete_group(&name));
        Ok(())
    }

    pub(crate) fn move_session_group(&self, name: String, index: i64) -> mlua::Result<()> {
        self.with_world_mut(|world| world.session.move_group(&name, index.max(0) as usize));
        Ok(())
    }

    pub(crate) fn move_session_document(&self, id: DocumentId, index: i64) -> mlua::Result<()> {
        self.with_world_mut(|world| world.session.move_document(id, index.max(0) as usize));
        Ok(())
    }

    pub(crate) fn set_active_document(&self, id: DocumentId) -> mlua::Result<()> {
        self.with_world_mut(|world| {
            if world.docs.get(&id).is_none() {
                return Err(mlua::Error::runtime("composition is not open"));
            }
            world.set_active(id);
            Ok(())
        })
    }

    pub(crate) fn open_path(&self, path: &str) -> mlua::Result<DocumentId> {
        let path = PathBuf::from(path);
        if is_fasession_path(&path) {
            return Err(mlua::Error::runtime(
                "use field.session.open for .fasession files",
            ));
        }
        self.with_world_mut(|world| {
            if let Some(id) = world.session.find_by_path(&path) {
                world.set_active(id);
                return Ok(id);
            }
            world
                .open_path(&path)
                .map_err(|err| mlua::Error::runtime(err.to_string()))
        })
    }

    pub(crate) fn reset_session(&self) -> mlua::Result<()> {
        self.with_world_mut(|world| {
            world.docs.clear();
            world.session = Session::new();
        });
        self.inner.borrow_mut().active = None;
        Ok(())
    }

    pub(crate) fn load_session_file(&self, path: &str) -> mlua::Result<()> {
        let path = PathBuf::from(path);
        if !is_fasession_path(&path) {
            return Err(mlua::Error::runtime(
                "field.session.open expects a .fasession file",
            ));
        }
        let json =
            std::fs::read_to_string(&path).map_err(|err| mlua::Error::runtime(err.to_string()))?;
        let loaded = Session::from_json(&json, Some(&path))
            .map_err(|err| mlua::Error::runtime(err.to_string()))?;
        self.with_world_mut(|world| {
            world.docs.clear();
            world.session = loaded.session;
            world.session.set_path(Some(path));
            hydrate_session_documents(world)
        })?;
        self.inner.borrow_mut().active = None;
        Ok(())
    }

    pub(crate) fn save_session_stub(&self, path: Option<String>) -> mlua::Result<()> {
        let _ = path;
        Err(mlua::Error::runtime(
            "session save is not implemented in the headless host",
        ))
    }

    pub(crate) fn document_group(&self, id: DocumentId) -> mlua::Result<Option<String>> {
        Ok(self.with_world(|world| world.session.get(id).and_then(|doc| doc.group.clone())))
    }

    pub(crate) fn set_document_group(
        &self,
        id: DocumentId,
        group: Option<String>,
    ) -> mlua::Result<()> {
        self.with_world_mut(|world| {
            let Some(doc) = world.session.get_mut(id) else {
                return Err(mlua::Error::runtime("composition is not open"));
            };
            doc.group = group;
            world.session.mark_dirty();
            Ok(())
        })
    }

    pub(crate) fn document_state(&self, id: DocumentId) -> mlua::Result<Option<String>> {
        Ok(self.with_world(|world| world.session.get(id).and_then(|doc| doc.state.clone())))
    }

    pub(crate) fn set_document_state(
        &self,
        id: DocumentId,
        state: Option<String>,
    ) -> mlua::Result<()> {
        self.with_world_mut(|world| {
            let Some(doc) = world.session.get_mut(id) else {
                return Err(mlua::Error::runtime("composition is not open"));
            };
            doc.state = state;
            world.session.mark_dirty();
            Ok(())
        })
    }

    pub(crate) fn document_properties(
        &self,
        id: DocumentId,
    ) -> mlua::Result<BTreeMap<String, String>> {
        Ok(self.with_world(|world| {
            world
                .session
                .get(id)
                .map(|doc| doc.properties.clone())
                .unwrap_or_default()
        }))
    }

    pub(crate) fn set_document_properties(
        &self,
        id: DocumentId,
        properties: BTreeMap<String, String>,
    ) -> mlua::Result<()> {
        self.with_world_mut(|world| {
            let Some(doc) = world.session.get_mut(id) else {
                return Err(mlua::Error::runtime("composition is not open"));
            };
            doc.properties = properties;
            world.session.mark_dirty();
            Ok(())
        })
    }

    pub(crate) fn display_name(&self, id: DocumentId) -> Option<String> {
        self.with_world(|world| {
            world.docs.get(&id).map(|doc| doc.name.clone()).or_else(|| {
                world.session.get(id).and_then(|doc| {
                    doc.name.clone().or_else(|| {
                        doc.file_path()
                            .and_then(|path| path.file_stem())
                            .map(|stem| stem.to_string_lossy().into_owned())
                    })
                })
            })
        })
    }

    pub(crate) fn set_display_name(&self, id: DocumentId, name: String) -> mlua::Result<()> {
        self.with_world_mut(|world| {
            if let Some(doc) = world.docs.get_mut(&id) {
                doc.name = name.clone();
            }
            if let Some(doc) = world.session.get_mut(id) {
                doc.name = Some(name);
                world.session.mark_dirty();
            }
            Ok(())
        })
    }

    pub(crate) fn document_path(&self, id: DocumentId) -> Option<PathBuf> {
        self.with_world(|world| {
            world
                .docs
                .get(&id)
                .and_then(|doc| doc.path.clone())
                .or_else(|| {
                    world
                        .session
                        .get(id)
                        .and_then(|doc| doc.file_path().map(Path::to_path_buf))
                })
        })
    }

    pub(crate) fn composition_id_string(&self, id: DocumentId) -> mlua::Result<String> {
        self.with_world(|world| {
            let doc = world
                .docs
                .get(&id)
                .ok_or_else(|| mlua::Error::runtime("composition is not open"))?;
            Ok(doc.composition.read().unwrap().id().to_string())
        })
    }

    pub(crate) fn close_composition(&self, id: DocumentId) -> mlua::Result<()> {
        self.with_world_mut(|world| {
            world.close(id);
            Ok(())
        })
    }

    pub(crate) fn replace_composition(
        &self,
        id: DocumentId,
        path: &str,
    ) -> mlua::Result<DocumentId> {
        let path = PathBuf::from(path);
        self.with_world_mut(|world| {
            world
                .replace(id, &path)
                .map_err(|err| mlua::Error::runtime(err.to_string()))
        })
    }
}

fn hydrate_session_documents(world: &mut HeadlessWorld) -> mlua::Result<()> {
    let docs: Vec<SessionDocument> = world.session.documents().to_vec();
    for session_doc in docs {
        let name = session_doc
            .name
            .clone()
            .or_else(|| {
                session_doc.file_path().and_then(|path| {
                    path.file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                })
            })
            .unwrap_or_else(|| session_doc.id.to_string());
        if world.docs.contains_key(&session_doc.id) {
            continue;
        }
        if let Some(path) = session_doc.file_path() {
            if path.exists() && !is_fasession_path(path) {
                if let Ok(composition) = if field_composition::is_facomp_path(path) {
                    Composition::load_facomp(path).map(|(c, _)| c)
                } else {
                    Composition::from_media_path(path, None)
                } {
                    let doc = crate::world::OpenDocument::new(
                        composition,
                        name,
                        Some(path.to_path_buf()),
                    );
                    world.docs.insert(session_doc.id, doc);
                    continue;
                }
            }
        }
        let composition = Composition::new(48_000, 2);
        world.docs.insert(
            session_doc.id,
            crate::world::OpenDocument::new(composition, name, None),
        );
    }
    Ok(())
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
    fn session_shared_exposes_id_and_properties() {
        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: None,
        })
        .unwrap();
        let out = host.eval(
            r#"(function() local s=field.session.shared(); s.properties={mode="test"}; return s.properties.mode, s.id~=nil end)()"#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("test\ttrue"), "{:?}", out.error);
    }
}
