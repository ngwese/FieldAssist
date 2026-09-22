// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! `field` namespace for the FieldAssist host (GPUI-backed documents).

use std::path::Path;

use mlua::{Function, MultiValue, Table, Value};

use super::composition::LuaComposition;
use super::files::{find_files, find_files_matching, normalize_extension};
use super::host::{host_from_lua, stringify_value, LogLevel};
use super::layout::layout_from_lua;
use super::media::LuaMedia;
use super::session::LuaSession;
use super::theme::{LuaThemeNamed, LuaThemeSemantic};
use super::workflow::{
    workflow_create_prototype, workflow_from_lua, workflow_from_prototype, workflow_run,
};
use super::workflow_toolbar;

/// Install the global `field` table.
pub fn bind_field(lua: &mlua::Lua) -> mlua::Result<()> {
    let field = lua.create_table()?;
    bind_log(lua, &field)?;
    bind_on(lua, &field)?;
    field.set(
        "include",
        lua.create_function(|lua, spec: Value| field_include(lua, spec))?,
    )?;
    bind_fs(lua, &field)?;
    bind_session(lua, &field)?;
    bind_composition(lua, &field)?;
    bind_media(lua, &field)?;
    bind_layouts(lua, &field)?;
    bind_workflow(lua, &field)?;
    bind_ui(lua, &field)?;
    bind_audio_devices(lua, &field)?;
    lua.globals().set("field", field)?;
    Ok(())
}

fn bind_log(lua: &mlua::Lua, field: &Table) -> mlua::Result<()> {
    let log = lua.create_table()?;
    log.set(
        "info",
        lua.create_function(|lua, (topic, message): (Value, Value)| {
            host_from_lua(lua)?.log(
                LogLevel::Info,
                stringify_value(lua, topic),
                stringify_value(lua, message),
            );
            Ok(())
        })?,
    )?;
    log.set(
        "warn",
        lua.create_function(|lua, (topic, message): (Value, Value)| {
            host_from_lua(lua)?.log(
                LogLevel::Warn,
                stringify_value(lua, topic),
                stringify_value(lua, message),
            );
            Ok(())
        })?,
    )?;
    log.set(
        "error",
        lua.create_function(|lua, (topic, message): (Value, Value)| {
            host_from_lua(lua)?.log(
                LogLevel::Error,
                stringify_value(lua, topic),
                stringify_value(lua, message),
            );
            Ok(())
        })?,
    )?;
    field.set("log", log)?;
    Ok(())
}

fn bind_on(lua: &mlua::Lua, field: &Table) -> mlua::Result<()> {
    field.set(
        "on",
        lua.create_function(|lua, (event, callback): (String, Function)| {
            let host = host_from_lua(lua)?;
            match event.as_str() {
                "loaded" => host.on_loaded(callback),
                "detect_layout" => host.on_detect_layout(callback),
                "saved" => host.on_saved(callback),
                "session_loaded" => host.on_session_loaded(callback),
                "session_saved" => host.on_session_saved(callback),
                "session_selected" => host.on_session_selected(callback),
                "composition_selected" => host.on_composition_selected(callback),
                other => return Err(mlua::Error::runtime(super::host::unknown_app_event(other))),
            }
            Ok(())
        })?,
    )?;
    Ok(())
}

fn field_include(lua: &mlua::Lua, spec: Value) -> mlua::Result<MultiValue> {
    let path = match spec {
        Value::String(s) => s.to_str()?.to_owned(),
        other => {
            return Err(mlua::Error::runtime(format!(
                "field.include expects a path string, got {}",
                other.type_name()
            )))
        }
    };
    let chunk = lua.load(Path::new(&path));
    chunk.eval()
}

fn bind_fs(lua: &mlua::Lua, field: &Table) -> mlua::Result<()> {
    let fs = lua.create_table()?;
    fs.set(
        "find_files",
        lua.create_function(|lua, args: MultiValue| find_files_from_lua(lua, args))?,
    )?;
    field.set("fs", fs)?;
    Ok(())
}

fn bind_session(lua: &mlua::Lua, field: &Table) -> mlua::Result<()> {
    let session = lua.create_table()?;
    session.set(
        "shared",
        lua.create_function(|_, ()| Ok(LuaSession::active()))?,
    )?;
    session.set(
        "open",
        lua.create_function(|lua, path: String| {
            let host = host_from_lua(lua)?;
            host.load_session(&path)
                .map(|id| LuaSession { id: Some(id) })
        })?,
    )?;
    field.set("session", session)?;
    Ok(())
}

fn bind_composition(lua: &mlua::Lua, field: &Table) -> mlua::Result<()> {
    let composition = lua.create_table()?;
    composition.set(
        "open",
        lua.create_function(|lua, path: String| {
            let host = host_from_lua(lua)?;
            host.open(&path).map(|id| LuaComposition { id })
        })?,
    )?;
    field.set("composition", composition)?;
    Ok(())
}

fn bind_media(lua: &mlua::Lua, field: &Table) -> mlua::Result<()> {
    let media = lua.create_table()?;
    media.set(
        "shared_pool",
        lua.create_function(|_, ()| Ok(LuaMediaPool))?,
    )?;
    field.set("media", media)?;
    Ok(())
}

/// Singleton handle to the process media pool (FieldAssist host).
struct LuaMediaPool;

impl mlua::UserData for LuaMediaPool {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("add", |lua, _, path: String| {
            let host = host_from_lua(lua)?;
            host.add_media(&path).map(|row| LuaMedia { id: row.id })
        });
        methods.add_method("remove", |lua, _, value: Value| {
            let host = host_from_lua(lua)?;
            let media = <LuaMedia as mlua::FromLua>::from_lua(value, lua)?;
            host.remove_media(media.id)
        });
        methods.add_method("list", |lua, _, ()| {
            let host = host_from_lua(lua)?;
            let rows = host.list_media()?;
            let table = lua.create_table_with_capacity(rows.len(), 0)?;
            for (index, row) in rows.into_iter().enumerate() {
                table.set(index + 1, LuaMedia { id: row.id })?;
            }
            Ok(table)
        });
    }
}

fn bind_layouts(lua: &mlua::Lua, field: &Table) -> mlua::Result<()> {
    let layouts = lua.create_table()?;
    layouts.set(
        "define",
        lua.create_function(|lua, spec: Table| {
            let layout = layout_from_lua(spec)?;
            host_from_lua(lua)?.define_layout(layout);
            Ok(())
        })?,
    )?;
    field.set("layouts", layouts)?;
    Ok(())
}

fn bind_workflow(lua: &mlua::Lua, field: &Table) -> mlua::Result<()> {
    let workflow = lua.create_table()?;
    workflow.set(
        "create",
        lua.create_function(|lua, properties: Table| workflow_create_prototype(lua, properties))?,
    )?;
    workflow.set(
        "declare",
        lua.create_function(|lua, args: MultiValue| {
            let mut args = args.into_iter();
            let first = args.next().ok_or_else(|| {
                mlua::Error::runtime("declare expects a prototype or (properties, function)")
            })?;
            let second = args.next();
            match (first, second) {
                (Value::Table(properties), Some(Value::Function(func))) => {
                    let def = workflow_from_lua(lua, properties, func)?;
                    host_from_lua(lua)?.declare_workflow(def);
                }
                (Value::Table(prototype), None | Some(Value::Nil)) => {
                    let def = workflow_from_prototype(prototype)?;
                    host_from_lua(lua)?.declare_workflow(def);
                }
                (other, _) => {
                    return Err(mlua::Error::runtime(format!(
                        "declare expects a prototype table or (properties, function), got {}",
                        other.type_name()
                    )))
                }
            }
            Ok(())
        })?,
    )?;
    workflow.set(
        "run",
        lua.create_function(|lua, args: MultiValue| run_workflow_from_lua(lua, args))?,
    )?;
    workflow.set(
        "finish",
        lua.create_function(|lua, ()| {
            host_from_lua(lua)?
                .finish_workflow()
                .map_err(mlua::Error::runtime)
        })?,
    )?;
    workflow.set(
        "cancel",
        lua.create_function(|lua, ()| {
            host_from_lua(lua)?
                .cancel_workflow()
                .map_err(mlua::Error::runtime)
        })?,
    )?;
    field.set("workflow", workflow)?;
    Ok(())
}

fn bind_ui(lua: &mlua::Lua, field: &Table) -> mlua::Result<()> {
    let ui = workflow_toolbar::ui_namespace(lua)?;
    // Default semantic/named palette (also available via live app.theme).
    ui.set("named", LuaThemeNamed)?;
    ui.set("semantic", LuaThemeSemantic)?;
    field.set("ui", ui)?;
    Ok(())
}

fn bind_audio_devices(lua: &mlua::Lua, field: &Table) -> mlua::Result<()> {
    let devices = lua.create_table()?;
    devices.set(
        "list",
        lua.create_function(|lua, ()| {
            let host = host_from_lua(lua)?;
            let names = host.output_devices();
            let table = lua.create_table_with_capacity(names.len(), 0)?;
            for (index, name) in names.into_iter().enumerate() {
                let row = lua.create_table()?;
                row.set("index", (index + 1) as i64)?;
                row.set("name", name)?;
                table.set(index + 1, row)?;
            }
            Ok(table)
        })?,
    )?;
    field.set("audio_devices", devices)?;
    Ok(())
}

fn run_workflow_from_lua(lua: &mlua::Lua, args: MultiValue) -> mlua::Result<Option<Table>> {
    let mut args = args.into_iter();
    let name = match args.next() {
        Some(Value::String(name)) => name.to_str()?.to_owned(),
        Some(other) => {
            return Err(mlua::Error::runtime(format!(
                "run expects a name, got {}",
                other.type_name()
            )))
        }
        None => return Err(mlua::Error::runtime("run expects a name")),
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
                "run payload must be a table, got {}",
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
