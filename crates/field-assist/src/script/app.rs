// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::path::Path;

use mlua::{
    FromLua, Function, MultiValue, Table, UserData, UserDataFields, UserDataMethods, Value,
};

use super::composition::LuaComposition;
use super::files::{find_files, find_files_matching, normalize_extension};
use super::host::{host_from_lua, stringify_value, LogLevel};
use super::layout::layout_from_lua;
use super::media::LuaMedia;
use super::session::LuaSession;
use super::theme::LuaTheme;
use super::workflow::{
    workflow_create_prototype, workflow_from_lua, workflow_from_prototype, workflow_run,
};

pub struct LuaApp;

impl UserData for LuaApp {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("session", |_, _| Ok(LuaSession::active()));
        fields.add_field_method_get("sessions", |lua, _| {
            let host = host_from_lua(lua)?;
            Ok(host.sessions())
        });
        fields.add_field_method_get("composition", |lua, _| {
            let host = host_from_lua(lua)?;
            Ok(host.active().map(|id| LuaComposition { id }))
        });
        fields.add_field_function_set("composition", |lua, _this, value: Value| {
            // Do not borrow `app` here. Selecting a composition runs
            // composition_selected hooks, and those hooks read `app` fields.
            let host = host_from_lua(lua)?;
            let doc = LuaComposition::from_lua(value, lua)?;
            host.set_active(doc.id)
        });
        fields.add_field_method_get("workflow", |lua, _| {
            let host = host_from_lua(lua)?;
            Ok(host.active_workflow())
        });
        fields.add_field_method_get("compositions", |lua, _| {
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
        fields.add_field_method_get("themes", |lua, _| super::theme::themes_table(lua));
        fields.add_field_method_get("ui", |lua, _| super::workflow_toolbar::ui_namespace(lua));
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
        fields.add_field_method_get("media", |lua, _| {
            let host = host_from_lua(lua)?;
            let rows = host.list_media()?;
            let table = lua.create_table_with_capacity(rows.len(), 0)?;
            for (index, row) in rows.into_iter().enumerate() {
                table.set(index + 1, LuaMedia { id: row.id })?;
            }
            Ok(table)
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("open", |lua, _, path: String| {
            let host = host_from_lua(lua)?;
            host.open(&path).map(|id| LuaComposition { id })
        });
        methods.add_method("add_media", |lua, _, path: String| {
            let host = host_from_lua(lua)?;
            host.add_media(&path).map(|row| LuaMedia { id: row.id })
        });
        methods.add_method("remove_media", |lua, _, value: Value| {
            let host = host_from_lua(lua)?;
            let media = LuaMedia::from_lua(value, lua)?;
            host.remove_media(media.id)
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
        methods.add_method("find_files", |lua, _, args: MultiValue| {
            find_files_from_lua(lua, args)
        });
        methods.add_method("define_layout", |lua, _, spec: Table| {
            let layout = layout_from_lua(spec)?;
            host_from_lua(lua)?.define_layout(layout);
            Ok(())
        });
        methods.add_method("create_workflow", |lua, _, properties: Table| {
            workflow_create_prototype(lua, properties)
        });
        methods.add_method("run_workflow", |lua, _, args: MultiValue| {
            run_workflow_from_lua(lua, args)
        });
        methods.add_method("declare_workflow", |lua, _, args: MultiValue| {
            let mut args = args.into_iter();
            let first = args.next().ok_or_else(|| {
                mlua::Error::runtime(
                    "declare_workflow expects a prototype or (properties, function)",
                )
            })?;
            let second = args.next();
            match (first, second) {
                (Value::Table(properties), Some(Value::Function(func))) => {
                    let workflow = workflow_from_lua(lua, properties, func)?;
                    host_from_lua(lua)?.declare_workflow(workflow);
                }
                (Value::Table(prototype), None | Some(Value::Nil)) => {
                    let workflow = workflow_from_prototype(prototype)?;
                    host_from_lua(lua)?.declare_workflow(workflow);
                }
                (other, _) => {
                    return Err(mlua::Error::runtime(format!(
                    "declare_workflow expects a prototype table or (properties, function), got {}",
                    other.type_name()
                )))
                }
            }
            Ok(())
        });
        methods.add_method("finish_workflow", |lua, _, ()| {
            host_from_lua(lua)?
                .finish_workflow()
                .map_err(mlua::Error::runtime)
        });
        methods.add_method("cancel_workflow", |lua, _, ()| {
            host_from_lua(lua)?
                .cancel_workflow()
                .map_err(mlua::Error::runtime)
        });
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
                "session_selected" => host.on_session_selected(callback),
                "composition_selected" => host.on_composition_selected(callback),
                _ => return Err(mlua::Error::runtime(super::host::unknown_app_event(&event))),
            }
            Ok(())
        });
    }
}

fn run_workflow_from_lua(lua: &mlua::Lua, args: MultiValue) -> mlua::Result<Option<Table>> {
    let mut args = args.into_iter();
    let name = match args.next() {
        Some(Value::String(name)) => name.to_str()?.to_owned(),
        Some(other) => {
            return Err(mlua::Error::runtime(format!(
                "run_workflow expects a name, got {}",
                other.type_name()
            )))
        }
        None => return Err(mlua::Error::runtime("run_workflow expects a name")),
    };
    let payload = match args.next() {
        None | Some(Value::Nil) => {
            let table = lua.create_table()?;
            table.set("scope", "run")?;
            table
        }
        Some(Value::Table(table)) => table,
        Some(other) => {
            return Err(mlua::Error::runtime(format!(
                "run_workflow payload must be a table, got {}",
                other.type_name()
            )))
        }
    };
    workflow_run(lua, &name, payload)
}

fn find_files_from_lua(lua: &mlua::Lua, args: MultiValue) -> mlua::Result<Table> {
    let mut args = args.into_iter();
    let dir = match args.next() {
        Some(Value::String(path)) => path.to_str()?.to_owned(),
        Some(other) => {
            return Err(mlua::Error::runtime(format!(
                "find_files expects a directory path, got {}",
                other.type_name()
            )))
        }
        None => return Err(mlua::Error::runtime("find_files expects a directory path")),
    };
    let paths = match args.next() {
        None | Some(Value::Nil) => {
            find_files(Path::new(&dir), None).map_err(mlua::Error::runtime)?
        }
        Some(Value::Table(table)) => {
            let extensions = extensions_from_lua(table)?;
            find_files(Path::new(&dir), Some(&extensions)).map_err(mlua::Error::runtime)?
        }
        Some(Value::Function(predicate)) => {
            find_files_matching(Path::new(&dir), |dirname, basename| {
                let value: Value = predicate
                    .call((dirname, basename))
                    .map_err(|err| err.to_string())?;
                Ok(lua_is_truthy(value))
            })
            .map_err(mlua::Error::runtime)?
        }
        Some(other) => {
            return Err(mlua::Error::runtime(format!(
                "find_files filter must be a list of extensions or a function, got {}",
                other.type_name()
            )))
        }
    };
    let table = lua.create_table()?;
    for (index, path) in paths.iter().enumerate() {
        table.set(index + 1, path.as_str())?;
    }
    Ok(table)
}

fn extensions_from_lua(table: Table) -> mlua::Result<Vec<String>> {
    let mut extensions = Vec::new();
    for value in table.sequence_values::<Value>() {
        match value? {
            Value::String(text) => {
                let ext = normalize_extension(&text.to_str()?);
                if !ext.is_empty() {
                    extensions.push(ext);
                }
            }
            other => {
                return Err(mlua::Error::runtime(format!(
                    "find_files extensions must be strings, got {}",
                    other.type_name()
                )))
            }
        }
    }
    Ok(extensions)
}

fn lua_is_truthy(value: Value) -> bool {
    !matches!(value, Value::Nil | Value::Boolean(false))
}

pub fn bind_app(lua: &mlua::Lua) -> mlua::Result<()> {
    lua.globals().set("app", LuaApp)
}
