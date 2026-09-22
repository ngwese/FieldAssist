// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Workflow toolbar control model (adapted from FieldAssist script layer).

#![allow(missing_docs)]

use mlua::{Table, Value};

use crate::host::host_from_lua;
use crate::marker::color_from_value;

const DEFAULT_TOGGLE_ON: [f32; 4] = [0.22, 0.72, 0.42, 1.0];
const UI_NAMESPACE_KEY: &str = "fa_ui_namespace";
const CONTROL_META_KEY: &str = "fa_toolbar_control_meta";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolbarAlign {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathBrowse {
    File,
    Directory,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolbarItem {
    Button {
        id: String,
        label: String,
        /// When set, the bar shows this icon (no label text). `label` is used
        /// for tooltip and accessibility.
        icon: Option<String>,
        align: ToolbarAlign,
    },
    PathEntry {
        id: String,
        label: Option<String>,
        value: String,
        browse: Option<PathBrowse>,
        align: ToolbarAlign,
    },
    Text {
        id: String,
        label: Option<String>,
        value: String,
        align: ToolbarAlign,
    },
    Toggle {
        id: String,
        label: String,
        value: bool,
        on_color: [f32; 4],
        /// When `None`, the bar uses the ghost-button foreground
        /// (`secondary_foreground`) so the off state matches toolbar buttons.
        off_color: Option<[f32; 4]>,
        /// Icon when `value` is true. One of `check` (default), `circle_check`,
        /// `circle_x`, or `circle_alert` (hyphens also accepted).
        on_icon: String,
        align: ToolbarAlign,
    },
    Message {
        id: String,
        text: String,
        color: Option<[f32; 4]>,
        align: ToolbarAlign,
    },
    Divider {
        id: Option<String>,
        align: ToolbarAlign,
    },
}

impl ToolbarItem {
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Button { id, .. }
            | Self::PathEntry { id, .. }
            | Self::Text { id, .. }
            | Self::Toggle { id, .. }
            | Self::Message { id, .. } => Some(id.as_str()),
            Self::Divider { id, .. } => id.as_deref(),
        }
    }

    pub fn align(&self) -> ToolbarAlign {
        match self {
            Self::Button { align, .. }
            | Self::PathEntry { align, .. }
            | Self::Text { align, .. }
            | Self::Toggle { align, .. }
            | Self::Message { align, .. }
            | Self::Divider { align, .. } => *align,
        }
    }

    pub fn label(&self) -> Option<&str> {
        match self {
            Self::Button { label, .. } | Self::Toggle { label, .. } => Some(label.as_str()),
            Self::PathEntry { label, .. } | Self::Text { label, .. } => label.as_deref(),
            _ => None,
        }
    }
}

/// `field.ui` control constructors (also exposed as `field.ui` from [`crate::ui`]).
pub fn ui_namespace(lua: &mlua::Lua) -> mlua::Result<Table> {
    if let Ok(table) = lua.named_registry_value::<Table>(UI_NAMESPACE_KEY) {
        return Ok(table);
    }
    let ui = lua.create_table()?;
    ui.set(
        "button",
        lua.create_function(|lua, props: Value| constructor(lua, "button", props))?,
    )?;
    ui.set(
        "toggle",
        lua.create_function(|lua, props: Value| constructor(lua, "toggle", props))?,
    )?;
    ui.set(
        "message",
        lua.create_function(|lua, props: Value| constructor(lua, "message", props))?,
    )?;
    ui.set(
        "text_entry",
        lua.create_function(|lua, props: Value| constructor(lua, "text_entry", props))?,
    )?;
    ui.set(
        "path_entry",
        lua.create_function(|lua, props: Value| constructor(lua, "path_entry", props))?,
    )?;
    ui.set("divider", lua.create_function(divider_constructor)?)?;
    lua.set_named_registry_value(UI_NAMESPACE_KEY, ui.clone())?;
    Ok(ui)
}

pub fn install_methods(lua: &mlua::Lua, proto: &Table) -> mlua::Result<()> {
    proto.set("set_toolbar", lua.create_function(set_toolbar)?)?;
    proto.set("set_item", lua.create_function(set_item)?)?;
    Ok(())
}

pub fn toolbar_from_table(table: &Table) -> Vec<ToolbarItem> {
    parse_toolbar(table.get("__fa_toolbar").unwrap_or(Value::Nil)).unwrap_or_default()
}

pub fn parse_toolbar(value: Value) -> mlua::Result<Vec<ToolbarItem>> {
    match value {
        Value::Nil => Ok(Vec::new()),
        Value::Table(table) => {
            let mut items = Vec::new();
            for row in table.sequence_values::<Value>() {
                let Value::Table(row) = row? else {
                    return Err(mlua::Error::runtime(
                        "toolbar items must be field.ui controls",
                    ));
                };
                if !is_control(&row)? {
                    return Err(mlua::Error::runtime(
                        "toolbar items must be field.ui controls",
                    ));
                }
                items.push(parse_toolbar_item(&row)?);
            }
            Ok(items)
        }
        other => Err(mlua::Error::runtime(format!(
            "toolbar must be a list of field.ui controls or nil, got {}",
            other.type_name()
        ))),
    }
}

pub fn parse_toolbar_item(row: &Table) -> mlua::Result<ToolbarItem> {
    let align = parse_align(row.get("align")?)?;
    let kind = toolbar_kind(row)?;
    match kind.as_str() {
        "button" => parse_button_item(row, align),
        "path_entry" => parse_path_item(row, align),
        "text_entry" => parse_text_item(row, align),
        "toggle" => parse_toggle_item(row, align),
        "message" => parse_message_item(row, align),
        "divider" => Ok(ToolbarItem::Divider {
            id: optional_nonempty_string(row.get("id")?)?,
            align,
        }),
        other => Err(mlua::Error::runtime(format!(
            "unknown toolbar kind `{other}`"
        ))),
    }
}

pub fn toolbar_row_id(row: &Table) -> mlua::Result<Option<String>> {
    optional_nonempty_string(row.get("id")?)
}

pub fn toolbar_row(table: &Table, id: &str) -> mlua::Result<Table> {
    let toolbar = match table.get::<Value>("__fa_toolbar")? {
        Value::Table(table) => table,
        _ => return Err(mlua::Error::runtime("workflow has no toolbar")),
    };
    for row in toolbar.sequence_values::<Value>() {
        let Value::Table(row) = row? else {
            continue;
        };
        if toolbar_row_id(&row)?.as_deref() == Some(id) {
            return Ok(row);
        }
    }
    Err(mlua::Error::runtime(format!("no toolbar item `{id}`")))
}

pub fn control_store(row: &Table) -> mlua::Result<Table> {
    match row.raw_get::<Value>("__fa_store")? {
        Value::Table(store) => Ok(store),
        _ => Err(mlua::Error::runtime("toolbar control is missing its store")),
    }
}

/// Call `action(control, workflow)` when it is a function. Returns whether it ran.
pub fn invoke_action(row: &Table, workflow: &Table) -> mlua::Result<bool> {
    match control_store(row)?.raw_get::<Value>("action")? {
        Value::Nil => Ok(false),
        Value::Function(func) => {
            func.call::<()>((row.clone(), workflow.clone()))?;
            Ok(true)
        }
        other => Err(mlua::Error::runtime(format!(
            "action must be a function, got {}",
            other.type_name()
        ))),
    }
}

fn constructor(lua: &mlua::Lua, kind: &str, props: Value) -> mlua::Result<Table> {
    let props = match props {
        Value::Table(table) => table,
        other => {
            return Err(mlua::Error::runtime(format!(
                "field.ui.{kind} expects a table, got {}",
                other.type_name()
            )))
        }
    };
    make_control(lua, kind, Some(props))
}

fn divider_constructor(lua: &mlua::Lua, props: Value) -> mlua::Result<Table> {
    let props = match props {
        Value::Nil => None,
        Value::Table(table) => Some(table),
        other => {
            return Err(mlua::Error::runtime(format!(
                "field.ui.divider expects a table or nil, got {}",
                other.type_name()
            )))
        }
    };
    make_control(lua, "divider", props)
}

fn make_control(lua: &mlua::Lua, kind: &str, props: Option<Table>) -> mlua::Result<Table> {
    let control = lua.create_table()?;
    let store = lua.create_table()?;
    if let Some(props) = props {
        for pair in props.pairs::<Value, Value>() {
            let (key, value) = pair?;
            store.raw_set(key, value)?;
        }
    }
    store.raw_set("kind", kind)?;
    match store.raw_get::<Value>("action")? {
        Value::Nil | Value::Function(_) => {}
        other => {
            return Err(mlua::Error::runtime(format!(
                "action must be a function, got {}",
                other.type_name()
            )))
        }
    }
    control.raw_set("__fa_store", store)?;
    control.set_metatable(Some(control_metatable(lua)?))?;
    parse_toolbar_item(&control)?;
    Ok(control)
}

fn control_metatable(lua: &mlua::Lua) -> mlua::Result<Table> {
    if let Ok(table) = lua.named_registry_value::<Table>(CONTROL_META_KEY) {
        return Ok(table);
    }
    let meta = lua.create_table()?;
    meta.raw_set("__fa_control", true)?;
    meta.set("__index", lua.create_function(control_index)?)?;
    meta.set("__newindex", lua.create_function(control_newindex)?)?;
    lua.set_named_registry_value(CONTROL_META_KEY, meta.clone())?;
    Ok(meta)
}

fn control_index(_lua: &mlua::Lua, (this, key): (Table, Value)) -> mlua::Result<Value> {
    control_store(&this)?.raw_get(key)
}

fn control_newindex(
    lua: &mlua::Lua,
    (this, key, value): (Table, Value, Value),
) -> mlua::Result<()> {
    let watched = key_is_watched(&key)?;
    let store = control_store(&this)?;
    store.raw_set(key, value)?;
    if !watched {
        return Ok(());
    }
    if let Value::Table(owner) = store.raw_get::<Value>("__fa_owner")? {
        host_from_lua(lua)?.toolbar_changed(&owner)?;
    }
    Ok(())
}

fn key_is_watched(key: &Value) -> mlua::Result<bool> {
    let Value::String(text) = key else {
        return Ok(false);
    };
    Ok(matches!(
        text.to_str()?.as_ref(),
        "label"
            | "value"
            | "text"
            | "color"
            | "on_color"
            | "off_color"
            | "align"
            | "icon"
            | "on_icon"
    ))
}

fn is_control(row: &Table) -> mlua::Result<bool> {
    let Some(meta) = row.metatable() else {
        return Ok(false);
    };
    Ok(matches!(
        meta.raw_get::<Value>("__fa_control")?,
        Value::Boolean(true)
    ))
}

fn stamp_owner(row: &Table, owner: &Table) -> mlua::Result<()> {
    control_store(row)?.raw_set("__fa_owner", owner.clone())
}

fn set_toolbar(lua: &mlua::Lua, (this, items): (Table, Value)) -> mlua::Result<()> {
    match &items {
        Value::Nil => {}
        Value::Table(table) => {
            for row in table.sequence_values::<Value>() {
                let Value::Table(row) = row? else {
                    return Err(mlua::Error::runtime(
                        "toolbar items must be field.ui controls",
                    ));
                };
                if !is_control(&row)? {
                    return Err(mlua::Error::runtime(
                        "toolbar items must be field.ui controls",
                    ));
                }
                stamp_owner(&row, &this)?;
                parse_toolbar_item(&row)?;
            }
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "toolbar must be a list of field.ui controls or nil, got {}",
                other.type_name()
            )))
        }
    }
    this.set("__fa_toolbar", items)?;
    host_from_lua(lua)?.toolbar_changed(&this)
}

fn set_item(lua: &mlua::Lua, (this, id, props): (Table, String, Table)) -> mlua::Result<()> {
    if id.is_empty() {
        return Err(mlua::Error::runtime("set_item id is required"));
    }
    let row = toolbar_row(&this, &id).map_err(|err| match err {
        mlua::Error::RuntimeError(message) if message == "workflow has no toolbar" => {
            mlua::Error::runtime("set_item requires a toolbar; call set_toolbar first")
        }
        other => other,
    })?;
    if matches!(toolbar_kind(&row)?.as_str(), "divider") {
        return host_from_lua(lua)?.toolbar_changed(&this);
    }
    for pair in props.pairs::<Value, Value>() {
        let (key, value) = pair?;
        row.set(key, value)?;
    }
    parse_toolbar_item(&row)?;
    host_from_lua(lua)?.toolbar_changed(&this)
}

fn toolbar_kind(row: &Table) -> mlua::Result<String> {
    match row.get::<Value>("kind")? {
        Value::String(kind) => Ok(kind.to_str()?.to_owned()),
        Value::Nil => Err(mlua::Error::runtime(
            "toolbar item needs kind; use an field.ui constructor",
        )),
        other => Err(mlua::Error::runtime(format!(
            "toolbar kind must be a string, got {}",
            other.type_name()
        ))),
    }
}

fn parse_button_item(row: &Table, align: ToolbarAlign) -> mlua::Result<ToolbarItem> {
    let id = optional_nonempty_string(row.get("id")?)?;
    let label = optional_nonempty_string(row.get("label")?)?;
    let icon = match optional_nonempty_string(row.get("icon")?)? {
        Some(name) => Some(parse_toolbar_icon(&name)?),
        None => None,
    };
    let (id, label) = match (id, label, icon.is_some()) {
        (Some(id), Some(label), _) => (id, label),
        (Some(id), None, _) => {
            let label = id.clone();
            (id, label)
        }
        (None, Some(label), _) => {
            let id = label.clone();
            (id, label)
        }
        (None, None, true) => {
            return Err(mlua::Error::runtime(
                "button with icon needs id or label for accessibility",
            ));
        }
        (None, None, false) => {
            return Err(mlua::Error::runtime("button id or label is required"));
        }
    };
    Ok(ToolbarItem::Button {
        id,
        label,
        icon,
        align,
    })
}

fn parse_path_item(row: &Table, align: ToolbarAlign) -> mlua::Result<ToolbarItem> {
    let id = required_nonempty_string(row.get("id")?, "path_entry id")?;
    let label = optional_nonempty_string(row.get("label")?)?;
    let value = string_field(row.get("value")?, "path_entry value")?;
    let browse = parse_browse(row.get("browse")?)?;
    Ok(ToolbarItem::PathEntry {
        id,
        label,
        value,
        browse,
        align,
    })
}

fn parse_text_item(row: &Table, align: ToolbarAlign) -> mlua::Result<ToolbarItem> {
    let id = required_nonempty_string(row.get("id")?, "text_entry id")?;
    let label = optional_nonempty_string(row.get("label")?)?;
    let value = string_field(row.get("value")?, "text_entry value")?;
    Ok(ToolbarItem::Text {
        id,
        label,
        value,
        align,
    })
}

fn parse_toggle_item(row: &Table, align: ToolbarAlign) -> mlua::Result<ToolbarItem> {
    let id = required_nonempty_string(row.get("id")?, "toggle id")?;
    let label = match optional_nonempty_string(row.get("label")?)? {
        Some(label) => label,
        None => id.clone(),
    };
    let value = lua_bool(row.get("value")?, false)?;
    let on_color = color_from_value(row.get("on_color")?)?.unwrap_or(DEFAULT_TOGGLE_ON);
    let off_color = color_from_value(row.get("off_color")?)?;
    let on_icon = match optional_nonempty_string(row.get("on_icon")?)? {
        Some(name) => parse_toolbar_icon(&name)?,
        None => "check".into(),
    };
    Ok(ToolbarItem::Toggle {
        id,
        label,
        value,
        on_color,
        off_color,
        on_icon,
        align,
    })
}

fn parse_toolbar_icon(name: &str) -> mlua::Result<String> {
    let normalized = name.replace('-', "_");
    match normalized.as_str() {
        "check"
        | "circle_check"
        | "circle_x"
        | "circle_alert"
        | "arrow_left"
        | "arrow_right" => Ok(normalized),
        other => Err(mlua::Error::runtime(format!(
            "icon must be \"check\", \"circle-check\", \"circle-x\", \"circle-alert\", \"arrow-left\", or \"arrow-right\", got `{other}`"
        ))),
    }
}

fn parse_message_item(row: &Table, align: ToolbarAlign) -> mlua::Result<ToolbarItem> {
    let id = required_nonempty_string(row.get("id")?, "message id")?;
    let text = string_field(row.get("text")?, "message text")?;
    let color = color_from_value(row.get("color")?)?;
    Ok(ToolbarItem::Message {
        id,
        text,
        color,
        align,
    })
}

fn parse_align(value: Value) -> mlua::Result<ToolbarAlign> {
    match value {
        Value::Nil => Ok(ToolbarAlign::Left),
        Value::String(text) => match text.to_str()?.as_ref() {
            "left" => Ok(ToolbarAlign::Left),
            "right" => Ok(ToolbarAlign::Right),
            other => Err(mlua::Error::runtime(format!(
                "align must be \"left\" or \"right\", got `{other}`"
            ))),
        },
        other => Err(mlua::Error::runtime(format!(
            "align must be \"left\" or \"right\", got {}",
            other.type_name()
        ))),
    }
}

fn parse_browse(value: Value) -> mlua::Result<Option<PathBrowse>> {
    match value {
        Value::Nil | Value::Boolean(false) => Ok(None),
        Value::Boolean(true) => Ok(Some(PathBrowse::Directory)),
        Value::String(text) => match text.to_str()?.as_ref() {
            "file" => Ok(Some(PathBrowse::File)),
            "directory" => Ok(Some(PathBrowse::Directory)),
            other => Err(mlua::Error::runtime(format!(
                "browse must be \"file\", \"directory\", or false, got `{other}`"
            ))),
        },
        other => Err(mlua::Error::runtime(format!(
            "browse must be \"file\", \"directory\", or false, got {}",
            other.type_name()
        ))),
    }
}

fn string_field(value: Value, what: &str) -> mlua::Result<String> {
    match value {
        Value::Nil => Ok(String::new()),
        Value::String(text) => Ok(text.to_str()?.to_owned()),
        other => Err(mlua::Error::runtime(format!(
            "{what} must be a string, got {}",
            other.type_name()
        ))),
    }
}

fn required_nonempty_string(value: Value, what: &str) -> mlua::Result<String> {
    match optional_nonempty_string(value)? {
        Some(text) => Ok(text),
        None => Err(mlua::Error::runtime(format!("{what} is required"))),
    }
}

fn optional_nonempty_string(value: Value) -> mlua::Result<Option<String>> {
    match value {
        Value::Nil => Ok(None),
        Value::String(text) => {
            let text = text.to_str()?.to_owned();
            if text.is_empty() {
                Ok(None)
            } else {
                Ok(Some(text))
            }
        }
        other => Err(mlua::Error::runtime(format!(
            "expected a string, got {}",
            other.type_name()
        ))),
    }
}

fn lua_bool(value: Value, default: bool) -> mlua::Result<bool> {
    match value {
        Value::Nil => Ok(default),
        Value::Boolean(value) => Ok(value),
        other => Err(mlua::Error::runtime(format!(
            "expected a boolean, got {}",
            other.type_name()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lua_toolbar(code: &str) -> Vec<ToolbarItem> {
        let lua = mlua::Lua::new();
        let ui = ui_namespace(&lua).expect("ui");
        lua.globals().set("ui", ui).expect("global");
        let value: Value = lua.load(code).eval().expect("toolbar lua");
        parse_toolbar(value).expect("parse")
    }

    #[test]
    fn parse_toolbar_button_defaults_and_kinds() {
        let items = lua_toolbar(
            r#"{
              ui.button({ label = "Go" }),
              ui.button({ id = "next", label = "Next", align = "right" }),
              ui.divider(),
              ui.message({ id = "progress", text = "1 of 2", align = "left" }),
              ui.path_entry({ id = "output", label = "Output", value = "/tmp", browse = "directory", align = "right" }),
              ui.toggle({ id = "keep", label = "Keep", value = true }),
              ui.text_entry({ id = "note", label = "Note", value = "hi" }),
              ui.divider({ id = "end", align = "right" }),
            }"#,
        );
        assert_eq!(items.len(), 8);
        assert_eq!(items[0].id(), Some("Go"));
        assert_eq!(items[0].label(), Some("Go"));
        assert_eq!(items[0].align(), ToolbarAlign::Left);
        match &items[1] {
            ToolbarItem::Button {
                id,
                label,
                icon,
                align,
            } => {
                assert_eq!(id, "next");
                assert_eq!(label, "Next");
                assert!(icon.is_none());
                assert_eq!(*align, ToolbarAlign::Right);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            items[2],
            ToolbarItem::Divider {
                id: None,
                align: ToolbarAlign::Left,
            }
        ));
        match &items[3] {
            ToolbarItem::Message { id, text, .. } => {
                assert_eq!(id, "progress");
                assert_eq!(text, "1 of 2");
            }
            other => panic!("{other:?}"),
        }
        match &items[4] {
            ToolbarItem::PathEntry {
                id,
                browse,
                align,
                value,
                ..
            } => {
                assert_eq!(id, "output");
                assert_eq!(value, "/tmp");
                assert_eq!(*browse, Some(PathBrowse::Directory));
                assert_eq!(*align, ToolbarAlign::Right);
            }
            other => panic!("{other:?}"),
        }
        match &items[5] {
            ToolbarItem::Toggle { id, value, .. } => {
                assert_eq!(id, "keep");
                assert!(*value);
            }
            other => panic!("{other:?}"),
        }
        match &items[6] {
            ToolbarItem::Text {
                id, value, label, ..
            } => {
                assert_eq!(id, "note");
                assert_eq!(label.as_deref(), Some("Note"));
                assert_eq!(value, "hi");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            items[7],
            ToolbarItem::Divider {
                id: Some(_),
                align: ToolbarAlign::Right,
            }
        ));
    }

    #[test]
    fn toggle_on_icon_accepts_hyphen_and_underscore_names() {
        let items = lua_toolbar(
            r#"{
              ui.toggle({ id = "a", on_icon = "circle-check" }),
              ui.toggle({ id = "b", on_icon = "circle_alert" }),
              ui.toggle({ id = "c", on_icon = "circle-x" }),
              ui.toggle({ id = "d" }),
            }"#,
        );
        match &items[..] {
            [ToolbarItem::Toggle { on_icon: a, .. }, ToolbarItem::Toggle { on_icon: b, .. }, ToolbarItem::Toggle { on_icon: c, .. }, ToolbarItem::Toggle { on_icon: d, .. }] =>
            {
                assert_eq!(a, "circle_check");
                assert_eq!(b, "circle_alert");
                assert_eq!(c, "circle_x");
                assert_eq!(d, "check");
            }
            other => panic!("{other:?}"),
        }
        let lua = mlua::Lua::new();
        let ui = ui_namespace(&lua).expect("ui");
        lua.globals().set("ui", ui).expect("global");
        let err: String = lua
            .load(
                r#"
                local ok, err = pcall(ui.toggle, { id = "bad", on_icon = "star" })
                assert(not ok)
                return tostring(err)
                "#,
            )
            .eval()
            .expect("pcall");
        assert!(err.contains("icon"), "{err}");
    }

    #[test]
    fn button_icon_is_icon_only() {
        let items = lua_toolbar(
            r#"{
              ui.button({ id = "previous", label = "Previous", icon = "arrow-left" }),
              ui.button({ id = "next", icon = "arrow_right" }),
            }"#,
        );
        match &items[..] {
            [ToolbarItem::Button {
                id: a,
                label: la,
                icon: Some(ia),
                ..
            }, ToolbarItem::Button {
                id: b,
                label: lb,
                icon: Some(ib),
                ..
            }] => {
                assert_eq!(a, "previous");
                assert_eq!(la, "Previous");
                assert_eq!(ia, "arrow_left");
                assert_eq!(b, "next");
                assert_eq!(lb, "next");
                assert_eq!(ib, "arrow_right");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn plain_tables_are_rejected() {
        let lua = mlua::Lua::new();
        let value: Value = lua
            .load(r#"{ { command = "go", label = "Go" } }"#)
            .eval()
            .unwrap();
        let err = parse_toolbar(value).expect_err("plain table");
        assert!(err.to_string().contains("field.ui"), "{err}");
    }
}
