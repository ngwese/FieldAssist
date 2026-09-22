// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Base `app` userdata (host-only facade).

use mlua::{MultiValue, UserData, UserDataFields, UserDataMethods, Value};

use crate::host::{host_from_lua, LogLevel};

/// Global `app` userdata.
pub struct LuaApp;

impl UserData for LuaApp {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |lua, _| {
            let host = host_from_lua(lua)?;
            let name = host.inner.borrow().profile.name;
            Ok(name)
        });
        fields.add_field_method_get("args", |lua, _| {
            let host = host_from_lua(lua)?;
            let args = host.inner.borrow().args.clone();
            Ok(args)
        });
        fields.add_field_method_get("workflow", |lua, _| {
            let host = host_from_lua(lua)?;
            let active = host.inner.borrow().active.clone();
            Ok(active)
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("alert", |lua, _, (subject, body): (String, String)| {
            host_from_lua(lua)?.alert(subject, body)
        });
        methods.add_method("info", |lua, _, (topic, message): (String, String)| {
            host_from_lua(lua)?.log(LogLevel::Info, topic, message);
            Ok(())
        });
        methods.add_method("warn", |lua, _, (topic, message): (String, String)| {
            host_from_lua(lua)?.log(LogLevel::Warn, topic, message);
            Ok(())
        });
        methods.add_method("error", |lua, _, (topic, message): (String, String)| {
            host_from_lua(lua)?.log(LogLevel::Error, topic, message);
            Ok(())
        });
        // Compatibility shims during migration — prefer field.workflow / field.log.
        methods.add_function("finish_workflow", |lua, ()| {
            host_from_lua(lua)?.finish_workflow(lua)
        });
        methods.add_function("cancel_workflow", |lua, ()| {
            host_from_lua(lua)?.cancel_workflow(lua)
        });
        methods.add_function("run_workflow", |lua, args: MultiValue| {
            let host = host_from_lua(lua)?;
            let mut iter = args.into_iter();
            let name = match iter.next() {
                Some(Value::String(s)) => s.to_str()?.to_owned(),
                _ => {
                    return Err(mlua::Error::runtime(
                        "run_workflow expects a workflow name string",
                    ))
                }
            };
            let payload = match iter.next() {
                Some(Value::Table(t)) => t,
                Some(Value::Nil) | None => {
                    let t = lua.create_table()?;
                    t.set("scope", "run")?;
                    t
                }
                Some(other) => {
                    return Err(mlua::Error::runtime(format!(
                        "run_workflow payload must be a table, got {}",
                        other.type_name()
                    )));
                }
            };
            host.workflow_run(lua, &name, payload)?;
            Ok(())
        });
    }
}

/// Install the global `app` object.
pub fn bind_app(lua: &mlua::Lua) -> mlua::Result<()> {
    lua.globals().set("app", LuaApp)?;
    Ok(())
}
