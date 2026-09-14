// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use mlua::{FromLua, Lua, Table, UserData, UserDataFields, UserDataMethods, Value};

use crate::model::SessionId;

use super::composition::LuaComposition;
use super::host::host_from_lua;

#[derive(Clone, Copy, Debug)]
pub struct LuaSession {
    /// `None` is the UI-active session; `Some` is a Lua-held loaded session.
    pub id: Option<SessionId>,
}

impl LuaSession {
    pub fn active() -> Self {
        Self { id: None }
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
            Ok(host.session_id(this.id))
        });
        fields.add_field_method_get("path", |lua, this| {
            let host = host_from_lua(lua)?;
            Ok(host.session_path(this.id))
        });
        fields.add_field_method_get("workflow_name", |lua, this| {
            let host = host_from_lua(lua)?;
            Ok(host.session_workflow(this.id))
        });
        fields.add_field_method_set("workflow_name", |lua, this, value: Value| {
            let host = host_from_lua(lua)?;
            host.set_session_workflow(this.id, optional_lua_string(value)?)
        });
        fields.add_field_method_get("capture_ui", |lua, this| {
            let host = host_from_lua(lua)?;
            Ok(host.session_capture_ui(this.id))
        });
        fields.add_field_method_set("capture_ui", |lua, this, value: bool| {
            let host = host_from_lua(lua)?;
            host.set_session_capture_ui(this.id, value)
        });
        fields.add_field_method_get("properties", |lua, this| {
            let host = host_from_lua(lua)?;
            string_map_to_lua(lua, &host.session_properties(this.id))
        });
        fields.add_field_method_set("properties", |lua, this, value: Value| {
            let host = host_from_lua(lua)?;
            host.set_session_properties(this.id, string_map_from_lua(value)?)
        });
        fields.add_field_method_get("composition", |lua, this| {
            let host = host_from_lua(lua)?;
            Ok(host
                .session_active_document(this.id)
                .map(|id| LuaComposition { id }))
        });
        fields.add_field_method_set("composition", |lua, this, value: Value| {
            let host = host_from_lua(lua)?;
            if this.id.is_some() {
                return Err(mlua::Error::runtime(
                    "composition can only be set on the UI session",
                ));
            }
            let doc = LuaComposition::from_lua(value, lua)?;
            host.set_active(doc.id)
        });
        fields.add_field_method_get("compositions", |lua, this| {
            let host = host_from_lua(lua)?;
            let docs: Vec<LuaComposition> = host
                .session_documents(this.id)
                .into_iter()
                .map(|id| LuaComposition { id })
                .collect();
            Ok(docs)
        });
        fields.add_field_method_get("groups", |lua, this| {
            let host = host_from_lua(lua)?;
            let groups = host.session_groups(this.id);
            let table = lua.create_table()?;
            for (index, name) in groups.into_iter().enumerate() {
                table.set(index + 1, name)?;
            }
            Ok(table)
        });
        fields.add_field_method_set("groups", |lua, this, value: Value| {
            let host = host_from_lua(lua)?;
            let groups = string_list_from_lua(value)?;
            host.set_session_groups(this.id, groups)
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("group_count", |lua, this, group: String| {
            let host = host_from_lua(lua)?;
            Ok(host.session_group_count(this.id, &group) as i64)
        });
        methods.add_method("add_group", |lua, this, name: String| {
            let host = host_from_lua(lua)?;
            host.add_session_group(this.id, name)
        });
        methods.add_method("rename_group", |lua, this, (old, new): (String, String)| {
            let host = host_from_lua(lua)?;
            host.rename_session_group(this.id, old, new)
        });
        methods.add_method("delete_group", |lua, this, name: String| {
            let host = host_from_lua(lua)?;
            host.delete_session_group(this.id, name)
        });
        methods.add_method("move_group", |lua, this, (name, index): (String, i64)| {
            let host = host_from_lua(lua)?;
            host.move_session_group(this.id, name, index)
        });
        methods.add_method("move", |lua, this, (doc, index): (LuaComposition, i64)| {
            let host = host_from_lua(lua)?;
            host.move_session_document(this.id, doc.id, index)
        });
        methods.add_method("open", |lua, this, path: String| {
            let host = host_from_lua(lua)?;
            if this.id.is_some() {
                return Err(mlua::Error::runtime(
                    "open is only available on the active session",
                ));
            }
            host.open(&path).map(|id| LuaComposition { id })
        });
        methods.add_method("save", |lua, this, ()| {
            let host = host_from_lua(lua)?;
            if this.id.is_some() {
                return Err(mlua::Error::runtime(
                    "save is only available on the active session",
                ));
            }
            host.save_session(None)
        });
        methods.add_method("save_as", |lua, this, path: String| {
            let host = host_from_lua(lua)?;
            if this.id.is_some() {
                return Err(mlua::Error::runtime(
                    "save_as is only available on the active session",
                ));
            }
            host.save_session(Some(path))
        });
        methods.add_method("close", |lua, this, ()| {
            let host = host_from_lua(lua)?;
            host.close_session(this.id)
        });
    }
}

pub fn optional_lua_string(value: Value) -> mlua::Result<Option<String>> {
    match value {
        Value::Nil => Ok(None),
        Value::String(value) => {
            let value = value.to_str()?.to_owned();
            Ok((!value.is_empty()).then_some(value))
        }
        other => Err(mlua::Error::runtime(format!(
            "expected a string or nil, got {}",
            other.type_name()
        ))),
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

pub fn string_map_from_lua(value: Value) -> mlua::Result<BTreeMap<String, String>> {
    match value {
        Value::Table(table) => {
            let mut map = BTreeMap::new();
            for pair in table.pairs::<Value, Value>() {
                let (key, value) = pair?;
                let key = match key {
                    Value::String(key) => key.to_str()?.to_owned(),
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "property keys must be strings, got {}",
                            other.type_name()
                        )))
                    }
                };
                let value = match value {
                    Value::String(value) => value.to_str()?.to_owned(),
                    Value::Nil => {
                        return Err(mlua::Error::runtime(format!(
                            "property `{key}` cannot be nil"
                        )))
                    }
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "property `{key}` must be a string, got {}",
                            other.type_name()
                        )))
                    }
                };
                map.insert(key, value);
            }
            Ok(map)
        }
        other => Err(mlua::Error::runtime(format!(
            "properties must be a table, got {}",
            other.type_name()
        ))),
    }
}

pub fn string_map_to_lua(lua: &Lua, map: &BTreeMap<String, String>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for (key, value) in map {
        table.set(key.as_str(), value.as_str())?;
    }
    Ok(table)
}
