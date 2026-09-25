// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Host-only `app` facade for FieldAssist (`app.name == "field-assist"`).

use mlua::{UserData, UserDataFields, UserDataMethods, Value};

use super::backend::backend_from_lua;
use super::theme::LuaTheme;

/// Stable process id for host-specific script logic.
pub const HOST_NAME: &str = "field-assist";

pub struct LuaApp;

impl UserData for LuaApp {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |_, _| Ok(HOST_NAME));
        fields.add_field_method_get("workflow", |lua, _| {
            let host = field_scripting::host_from_lua(lua)?;
            Ok(host.active_workflow())
        });
        fields.add_field_method_get("output_device", |lua, _| {
            Ok(backend_from_lua(lua)?.output_device())
        });
        fields.add_field_method_set("output_device", |lua, _, value: Value| {
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
            backend_from_lua(lua)?.set_output_device(spec.as_deref())
        });
        fields.add_field_method_get("output_devices", |lua, _| {
            Ok(backend_from_lua(lua)?.output_devices())
        });
        fields.add_field_method_get("theme", |_, _| Ok(LuaTheme));
        fields.add_field_method_get("themes", |lua, _| super::theme::themes_table(lua));
        fields.add_field_method_get("looping", |lua, _| Ok(backend_from_lua(lua)?.looping()));
        fields.add_field_method_get("preview", |lua, _| Ok(backend_from_lua(lua)?.preview()));
        fields.add_field_method_get("explorer", |lua, _| Ok(backend_from_lua(lua)?.explorer()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("command", |lua, _, id: String| {
            backend_from_lua(lua)?
                .command(&id)
                .map_err(mlua::Error::runtime)
        });
        methods.add_method("load_settings", |lua, _, ()| {
            backend_from_lua(lua)?.load_settings()
        });
        methods.add_method("alert", |lua, _, (subject, body): (Value, Value)| {
            let stringify = |value| match value {
                Value::String(v) => v.to_string_lossy(),
                other => other.to_string().unwrap_or_else(|_| "<unprintable>".into()),
            };
            field_scripting::host_from_lua(lua)?.alert(stringify(subject), stringify(body))
        });
    }
}

pub fn bind_app(lua: &mlua::Lua) -> mlua::Result<()> {
    lua.globals().set("app", LuaApp)
}
