// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Host-only `app` facade for FieldAssist (`app.name == "field-assist"`).

use mlua::{UserData, UserDataFields, UserDataMethods, Value};

use super::host::{host_from_lua, stringify_value};
use super::theme::LuaTheme;

/// Stable process id for host-specific script logic.
pub const HOST_NAME: &str = "field-assist";

pub struct LuaApp;

impl UserData for LuaApp {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |_, _| Ok(HOST_NAME));
        fields.add_field_method_get("workflow", |lua, _| {
            let host = host_from_lua(lua)?;
            Ok(host.active_workflow())
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
        fields.add_field_method_get("themes", |lua, _| super::theme::themes_table(lua));
        fields.add_field_method_get("looping", |lua, _| {
            let host = host_from_lua(lua)?;
            Ok(host.looping())
        });
        fields.add_field_method_get("preview", |lua, _| {
            let host = host_from_lua(lua)?;
            Ok(host.preview())
        });
        fields.add_field_method_get("explorer", |lua, _| {
            let host = host_from_lua(lua)?;
            Ok(host.explorer())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("command", |lua, _, id: String| {
            host_from_lua(lua)?
                .command(&id)
                .map_err(mlua::Error::runtime)
        });
        methods.add_method("alert", |lua, _, (subject, body): (Value, Value)| {
            let host = host_from_lua(lua)?;
            host.alert(stringify_value(lua, subject), stringify_value(lua, body))
        });
    }
}

pub fn bind_app(lua: &mlua::Lua) -> mlua::Result<()> {
    lua.globals().set("app", LuaApp)
}
