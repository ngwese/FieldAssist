// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use mlua::{Function, Table, UserData, UserDataFields, UserDataMethods, Value};

use super::composition::LuaComposition;
use super::host::{host_from_lua, stringify_value, LogLevel};
use super::layout::layout_from_lua;
use super::session::LuaSession;
use super::theme::LuaTheme;
use super::workflow::workflow_from_lua;

pub struct LuaApp;

impl UserData for LuaApp {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("session", |_, _| Ok(LuaSession::active()));
        fields.add_field_method_get("sessions", |lua, _| {
            let host = host_from_lua(lua)?;
            Ok(host.sessions())
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
        fields.add_field_method_get("output_device", |lua, _| {
            let host = host_from_lua(lua)?;
            Ok(host.output_device())
        });
        fields.add_field_method_set("output_device", |lua, _, value: Value| {
            let host = host_from_lua(lua)?;
            let spec = match value {
                Value::Nil => None,
                Value::String(name) => Some(name.to_str()?.to_owned()),
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "output_device must be a string or nil, got {}",
                        other.type_name()
                    )))
                }
            };
            host.set_output_device(spec.as_deref())
        });
        fields.add_field_method_get("output_devices", |lua, _| {
            let host = host_from_lua(lua)?;
            Ok(host.output_devices())
        });
        fields.add_field_method_get("theme", |_, _| Ok(LuaTheme));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("open", |lua, _, path: String| {
            let host = host_from_lua(lua)?;
            host.open(&path).map(|id| LuaComposition { id })
        });
        methods.add_method("load_session", |lua, _, path: String| {
            let host = host_from_lua(lua)?;
            host.load_session(&path)
                .map(|id| LuaSession { id: Some(id) })
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
        methods.add_method(
            "declare_workflow",
            |lua, _, (properties, func): (Table, Function)| {
                let workflow = workflow_from_lua(properties, func)?;
                host_from_lua(lua)?.declare_workflow(workflow);
                Ok(())
            },
        );
        methods.add_method("alert", |lua, _, (subject, body): (Value, Value)| {
            let host = host_from_lua(lua)?;
            host.alert(stringify_value(lua, subject), stringify_value(lua, body))
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
                "session_loaded" => host.on_session_loaded(callback),
                "session_saved" => host.on_session_saved(callback),
                _ => {
                    return Err(mlua::Error::runtime(format!(
                        "unknown event `{event}`; expected \"loaded\", \"detect_layout\", \"saved\", \"session_loaded\", or \"session_saved\""
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
