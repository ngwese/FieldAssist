// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! `field` namespace root.

use mlua::{Function, Table, Value};

use crate::audio_devices::bind_audio_devices;
use crate::composition::bind_composition_module;
use crate::export::bind_exports;
use crate::fs::bind_fs;
use crate::host::{host_from_lua, LogLevel};
use crate::include::field_include;
use crate::layout::bind_layouts;
use crate::media::bind_media_module;
use crate::session::bind_session_module;
use crate::ui::bind_ui;
use crate::url::bind_url;
use crate::workflow::bind_workflow_module;

/// Install the global `field` table and its modules.
pub fn bind_field(lua: &mlua::Lua) -> mlua::Result<()> {
    let field = lua.create_table()?;
    bind_log(lua, &field)?;
    bind_on(lua, &field)?;
    field.set(
        "include",
        lua.create_function(|lua, spec: Value| field_include(lua, spec))?,
    )?;
    bind_scripting(lua, &field)?;
    bind_url(lua, &field)?;
    bind_fs(lua, &field)?;
    bind_session_module(lua, &field)?;
    bind_composition_module(lua, &field)?;
    bind_media_module(lua, &field)?;
    bind_layouts(lua, &field)?;
    bind_exports(lua, &field)?;
    bind_workflow_module(lua, &field)?;
    bind_ui(lua, &field)?;
    bind_audio_devices(lua, &field)?;
    lua.globals().set("field", field)?;
    Ok(())
}

fn bind_scripting(lua: &mlua::Lua, field: &Table) -> mlua::Result<()> {
    let scripting = lua.create_table()?;
    scripting.set(
        "enable_system_package_paths",
        lua.create_function(|lua, ()| host_from_lua(lua)?.enable_system_package_paths(lua))?,
    )?;
    scripting.set(
        "enable_native_modules",
        lua.create_function(|lua, ()| host_from_lua(lua)?.enable_native_modules(lua))?,
    )?;
    field.set("scripting", scripting)?;
    Ok(())
}

fn bind_log(lua: &mlua::Lua, field: &Table) -> mlua::Result<()> {
    let log = lua.create_table()?;
    log.set(
        "info",
        lua.create_function(|lua, (topic, message): (String, String)| {
            host_from_lua(lua)?.log(LogLevel::Info, topic, message);
            Ok(())
        })?,
    )?;
    log.set(
        "warn",
        lua.create_function(|lua, (topic, message): (String, String)| {
            host_from_lua(lua)?.log(LogLevel::Warn, topic, message);
            Ok(())
        })?,
    )?;
    log.set(
        "error",
        lua.create_function(|lua, (topic, message): (String, String)| {
            host_from_lua(lua)?.log(LogLevel::Error, topic, message);
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
            let mut inner = host.inner.borrow_mut();
            match event.as_str() {
                "loaded" => inner.loaded.push(callback),
                "saved" => inner.saved.push(callback),
                "session_loaded" => inner.session_loaded.push(callback),
                "session_saved" => inner.session_saved.push(callback),
                "session_selected" => inner.session_selected.push(callback),
                "composition_selected" => inner.composition_selected.push(callback),
                "detect_layout" => inner.detect_layout.push(callback),
                other => return Err(mlua::Error::runtime(format!("unknown event `{other}`"))),
            }
            Ok(())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::host::{HostProfile, ScriptHost};

    #[test]
    fn field_log_is_captured() {
        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: None,
        })
        .unwrap();
        let out = host.eval(r#"field.log.info("t", "hello")"#);
        assert!(out.error.is_none(), "{:?}", out.error);
        let logs = host.take_logs();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].topic, "t");
        assert_eq!(logs[0].message, "hello");
    }

    #[test]
    fn package_path_defaults_exclude_system() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: Some(dir.path().to_path_buf()),
        })
        .unwrap();
        let out = host.eval("return package.path");
        assert!(out.error.is_none(), "{:?}", out.error);
        let path = out.result.unwrap();
        let config = dir.path().display().to_string();
        assert!(path.contains(&format!("{config}/?.lua")), "{path}");
        assert!(path.contains("./?.lua"), "{path}");
        assert!(!path.contains("/usr/local"), "{path}");
    }

    #[test]
    fn enable_system_package_paths_from_lua() {
        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: None,
        })
        .unwrap();
        let before = host.eval("return package.path").result.unwrap();
        let out = host.eval(
            r#"
            field.scripting.enable_system_package_paths()
            field.scripting.enable_system_package_paths()
            return package.path
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        let after = out.result.unwrap();
        assert!(
            after.starts_with(&before) || after.contains(&before),
            "{after}"
        );
        assert!(after.len() >= before.len());
    }

    #[test]
    fn enable_native_modules_from_lua() {
        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: None,
        })
        .unwrap();
        let blocked = host.eval(r#"return require("missing_native_xyz")"#);
        assert!(blocked.error.is_some());
        let err = blocked.error.unwrap();
        assert!(
            err.contains("can't load C modules until field.scripting.enable_native_modules()"),
            "{err}"
        );
        let out = host.eval(
            r#"
            field.scripting.enable_native_modules()
            field.scripting.enable_native_modules()
            local ok, err = pcall(require, "missing_native_xyz")
            return ok, tostring(err)
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        let result = out.result.unwrap();
        assert!(result.starts_with("false"), "{result}");
        assert!(
            !result.contains("can't load C modules until field.scripting.enable_native_modules()"),
            "{result}"
        );
    }
}
