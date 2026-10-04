// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Host-only `app` facade for FieldAssist (`app.name == "field-assist"`).

use mlua::{FromLua, Function, Lua, UserData, UserDataFields, UserDataMethods, Value};

use field_scripting::{host_from_lua, LuaComposition, LuaSession};

use super::access;
use super::backend::backend_from_lua;
use super::theme::LuaTheme;

/// Stable process id for host-specific script logic.
pub const HOST_NAME: &str = "field-assist";

pub struct LuaApp;

struct ConfirmOptions {
    ok_label: String,
    cancel_label: String,
    danger: bool,
    on_confirm: Option<Function>,
    on_cancel: Option<Function>,
}

impl Default for ConfirmOptions {
    fn default() -> Self {
        Self {
            ok_label: "OK".into(),
            cancel_label: "Cancel".into(),
            danger: false,
            on_confirm: None,
            on_cancel: None,
        }
    }
}

fn stringify_arg(value: Value) -> String {
    match value {
        Value::Nil => String::new(),
        Value::String(v) => v.to_string_lossy(),
        other => other.to_string().unwrap_or_else(|_| "<unprintable>".into()),
    }
}

fn parse_confirm_options(opts: Value) -> mlua::Result<ConfirmOptions> {
    let mut out = ConfirmOptions::default();
    let Value::Table(table) = opts else {
        return Ok(out);
    };

    if let Ok(v) = table.get::<Value>("ok") {
        let s = stringify_arg(v);
        if !s.is_empty() {
            out.ok_label = s;
        }
    } else if let Ok(v) = table.get::<Value>("ok_text") {
        let s = stringify_arg(v);
        if !s.is_empty() {
            out.ok_label = s;
        }
    }

    if let Ok(v) = table.get::<Value>("cancel") {
        let s = stringify_arg(v);
        if !s.is_empty() {
            out.cancel_label = s;
        }
    } else if let Ok(v) = table.get::<Value>("cancel_text") {
        let s = stringify_arg(v);
        if !s.is_empty() {
            out.cancel_label = s;
        }
    }

    if let Ok(v) = table.get::<bool>("danger") {
        out.danger = v;
    }

    if let Ok(f) = table.get::<Function>("on_confirm") {
        out.on_confirm = Some(f);
    }
    if let Ok(f) = table.get::<Function>("on_cancel") {
        out.on_cancel = Some(f);
    }

    Ok(out)
}

fn script_confirm(lua: &Lua, subject: Value, body: Value, opts: Value) -> mlua::Result<bool> {
    let subject = stringify_arg(subject);
    let body = stringify_arg(body);
    let options = parse_confirm_options(opts)?;
    let host = host_from_lua(lua)?;

    // Tests / scripted automation: honor the FIFO queue before any GUI.
    if let Some(queued) = host.take_confirm_response() {
        return Ok(queued);
    }

    match access::with_view(|view, window, cx| {
        view.script_confirm(
            subject.clone(),
            body.clone(),
            options.ok_label.clone(),
            options.cancel_label.clone(),
            options.danger,
            options.on_confirm.clone(),
            options.on_cancel.clone(),
            window,
            cx,
        );
    }) {
        Ok(()) => {
            // Dialog is async; destructive work belongs in on_confirm.
            Ok(false)
        }
        Err(_) => {
            // Headless / no entered AppView: permissive default.
            Ok(host.confirm(&subject, &body))
        }
    }
}

impl UserData for LuaApp {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |_, _| Ok(HOST_NAME));
        fields.add_field_method_get("workflow", |lua, _| {
            let host = host_from_lua(lua)?;
            Ok(host.active_workflow())
        });
        fields.add_field_method_get("session", |_, _| Ok(LuaSession::focused()));
        fields.add_field_method_get("composition", |lua, _| {
            LuaSession::focused().composition(lua)
        });
        fields.add_field_method_set("composition", |lua, _, value: Value| {
            let doc = LuaComposition::from_lua(value, lua)?;
            LuaSession::focused().set_composition(lua, doc.id)
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
            host_from_lua(lua)?.alert(stringify_arg(subject), stringify_arg(body))
        });
        methods.add_method(
            "confirm",
            |lua, _, (subject, body, opts): (Value, Value, Value)| {
                script_confirm(lua, subject, body, opts)
            },
        );
    }
}

pub fn bind_app(lua: &mlua::Lua) -> mlua::Result<()> {
    lua.globals().set("app", LuaApp)
}
