// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use mlua::{Function, Table, UserData, UserDataFields, UserDataMethods, Value};

use super::composition::LuaComposition;
use super::host::{host_from_lua, stringify_value, LogLevel};
use super::layout::layout_from_lua;

pub struct LuaApp;

impl UserData for LuaApp {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
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
        methods.add_method("command", |lua, _, id: String| {
            host_from_lua(lua)?
                .command(&id)
                .map_err(mlua::Error::runtime)
        });
        methods.add_method("dofile", |lua, _, path: String| {
            lua.load(std::path::Path::new(&path))
                .exec()
                .map_err(mlua::Error::runtime)
        });
        methods.add_method("define_layout", |lua, _, spec: Table| {
            let layout = layout_from_lua(spec)?;
            host_from_lua(lua)?.define_layout(layout);
            Ok(())
        });
        methods.add_method("info", |lua, _, (topic, message): (Value, Value)| {
            let host = host_from_lua(lua)?;
            host.log(
                LogLevel::Info,
                stringify_value(lua, topic),
                stringify_value(lua, message),
            );
            Ok(())
        });
        methods.add_method("warn", |lua, _, (topic, message): (Value, Value)| {
            let host = host_from_lua(lua)?;
            host.log(
                LogLevel::Warn,
                stringify_value(lua, topic),
                stringify_value(lua, message),
            );
            Ok(())
        });
        methods.add_method("error", |lua, _, (topic, message): (Value, Value)| {
            let host = host_from_lua(lua)?;
            host.log(
                LogLevel::Error,
                stringify_value(lua, topic),
                stringify_value(lua, message),
            );
            Ok(())
        });
        methods.add_method("on", |lua, _, (event, callback): (String, Function)| {
            let host = host_from_lua(lua)?;
            match event.as_str() {
                "loaded" => host.on_loaded(callback),
                "detect_layout" => host.on_detect_layout(callback),
                "saved" => host.on_saved(callback),
                _ => {
                    return Err(mlua::Error::runtime(format!(
                    "unknown event `{event}`; expected \"loaded\", \"detect_layout\", or \"saved\""
                )))
                }
            }
            Ok(())
        });
    }
}

pub fn bind_app(lua: &mlua::Lua) -> mlua::Result<()> {
    lua.globals().set("app", LuaApp)
}
