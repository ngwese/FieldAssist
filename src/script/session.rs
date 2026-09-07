// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use mlua::{Lua, Table, UserData, UserDataFields, UserDataMethods, Value};

use super::composition::LuaComposition;
use super::host::host_from_lua;

pub struct LuaSession;

impl UserData for LuaSession {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("id", |lua, _| {
            let host = host_from_lua(lua)?;
            Ok(host.session_id())
        });
        fields.add_field_method_get("path", |lua, _| {
            let host = host_from_lua(lua)?;
            Ok(host.session_path())
        });
        fields.add_field_method_get("workflow", |lua, _| {
            let host = host_from_lua(lua)?;
            Ok(host.session_workflow())
        });
        fields.add_field_method_set("workflow", |lua, _, value: Value| {
            let host = host_from_lua(lua)?;
            host.set_session_workflow(optional_lua_string(value)?)
        });
        fields.add_field_method_get("capture_ui", |lua, _| {
            let host = host_from_lua(lua)?;
            Ok(host.session_capture_ui())
        });
        fields.add_field_method_set("capture_ui", |lua, _, value: bool| {
            let host = host_from_lua(lua)?;
            host.set_session_capture_ui(value)
        });
        fields.add_field_method_get("properties", |lua, _| {
            let host = host_from_lua(lua)?;
            string_map_to_lua(lua, &host.session_properties())
        });
        fields.add_field_method_set("properties", |lua, _, value: Value| {
            let host = host_from_lua(lua)?;
            host.set_session_properties(string_map_from_lua(value)?)
        });
        fields.add_field_method_get("active", |lua, _| {
            let host = host_from_lua(lua)?;
            Ok(host.active().map(|id| LuaComposition { id }))
        });
        fields.add_field_method_get("documents", |lua, _| {
            let host = host_from_lua(lua)?;
            let docs: Vec<LuaComposition> = host
                .documents()
                .into_iter()
                .map(|id| LuaComposition { id })
                .collect();
            Ok(docs)
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("open", |lua, _, path: String| {
            let host = host_from_lua(lua)?;
            host.open(&path).map(|id| LuaComposition { id })
        });
        methods.add_method("save", |lua, _, ()| host_from_lua(lua)?.save_session(None));
        methods.add_method("save_as", |lua, _, path: String| {
            host_from_lua(lua)?.save_session(Some(path))
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
