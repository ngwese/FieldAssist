// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use mlua::{Table, Value};

use super::marker::color_from_value;

const DEFAULT_TOGGLE_OFF: [f32; 4] = [0.55, 0.55, 0.6, 1.0];
const DEFAULT_TOGGLE_ON: [f32; 4] = [0.22, 0.72, 0.42, 1.0];

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
        command: String,
        label: String,
        align: ToolbarAlign,
    },
    Path {
        id: String,
        label: Option<String>,
        value: String,
        browse: Option<PathBrowse>,
        align: ToolbarAlign,
    },
    Toggle {
        id: String,
        label: String,
        value: bool,
        on_color: [f32; 4],
        off_color: [f32; 4],
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
    #[cfg(test)]
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Button { id, .. }
            | Self::Path { id, .. }
            | Self::Toggle { id, .. }
            | Self::Message { id, .. } => Some(id.as_str()),
            Self::Divider { id, .. } => id.as_deref(),
        }
    }

    pub fn align(&self) -> ToolbarAlign {
        match self {
            Self::Button { align, .. }
            | Self::Path { align, .. }
            | Self::Toggle { align, .. }
            | Self::Message { align, .. }
            | Self::Divider { align, .. } => *align,
        }
    }

    #[cfg(test)]
    pub fn command(&self) -> Option<&str> {
        match self {
            Self::Button { command, .. } => Some(command.as_str()),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn label(&self) -> Option<&str> {
        match self {
            Self::Button { label, .. } | Self::Toggle { label, .. } => Some(label.as_str()),
            Self::Path { label, .. } => label.as_deref(),
            _ => None,
        }
    }
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
                    return Err(mlua::Error::runtime("toolbar items must be tables"));
                };
                items.push(parse_toolbar_item(&row)?);
            }
            Ok(items)
        }
        other => Err(mlua::Error::runtime(format!(
            "toolbar must be a list of items or nil, got {}",
            other.type_name()
        ))),
    }
}

pub fn parse_toolbar_item(row: &Table) -> mlua::Result<ToolbarItem> {
    let align = parse_align(row.get("align")?)?;
    let kind = toolbar_kind(row)?;
    match kind.as_str() {
        "button" => parse_button_item(row, align),
        "path" => parse_path_item(row, align),
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
    if let Some(id) = optional_nonempty_string(row.get("id")?)? {
        return Ok(Some(id));
    }
    optional_nonempty_string(row.get("command")?)
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

fn set_toolbar(lua: &mlua::Lua, (this, items): (Table, Value)) -> mlua::Result<()> {
    parse_toolbar(items.clone())?;
    this.set("__fa_toolbar", items)?;
    super::host::host_from_lua(lua)?.toolbar_changed(&this)
}

fn set_item(lua: &mlua::Lua, (this, id, props): (Table, String, Table)) -> mlua::Result<()> {
    if id.is_empty() {
        return Err(mlua::Error::runtime("set_item id is required"));
    }
    let toolbar = match this.get::<Value>("__fa_toolbar")? {
        Value::Table(table) => table,
        Value::Nil => {
            return Err(mlua::Error::runtime(
                "set_item requires a toolbar; call set_toolbar first",
            ))
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "toolbar must be a list of items, got {}",
                other.type_name()
            )))
        }
    };
    let mut found = None;
    for row in toolbar.sequence_values::<Value>() {
        let Value::Table(row) = row? else {
            continue;
        };
        if toolbar_row_id(&row)?.as_deref() == Some(id.as_str()) {
            found = Some(row);
            break;
        }
    }
    let Some(row) = found else {
        return Err(mlua::Error::runtime(format!("no toolbar item `{id}`")));
    };
    if matches!(toolbar_kind(&row)?.as_str(), "divider") {
        return super::host::host_from_lua(lua)?.toolbar_changed(&this);
    }
    for pair in props.pairs::<Value, Value>() {
        let (key, value) = pair?;
        row.set(key, value)?;
    }
    parse_toolbar_item(&row)?;
    super::host::host_from_lua(lua)?.toolbar_changed(&this)
}

fn toolbar_kind(row: &Table) -> mlua::Result<String> {
    match row.get::<Value>("kind")? {
        Value::Nil => {
            if optional_nonempty_string(row.get("command")?)?.is_some() {
                Ok("button".into())
            } else {
                Err(mlua::Error::runtime("toolbar item needs kind or command"))
            }
        }
        Value::String(kind) => Ok(kind.to_str()?.to_owned()),
        other => Err(mlua::Error::runtime(format!(
            "toolbar kind must be a string, got {}",
            other.type_name()
        ))),
    }
}

fn parse_button_item(row: &Table, align: ToolbarAlign) -> mlua::Result<ToolbarItem> {
    let command = required_nonempty_string(row.get("command")?, "toolbar command")?;
    let label = match optional_nonempty_string(row.get("label")?)? {
        Some(label) => label,
        None => command.clone(),
    };
    let id = optional_nonempty_string(row.get("id")?)?.unwrap_or_else(|| command.clone());
    Ok(ToolbarItem::Button {
        id,
        command,
        label,
        align,
    })
}

fn parse_path_item(row: &Table, align: ToolbarAlign) -> mlua::Result<ToolbarItem> {
    let id = required_nonempty_string(row.get("id")?, "path id")?;
    let label = optional_nonempty_string(row.get("label")?)?;
    let value = match row.get::<Value>("value")? {
        Value::Nil => String::new(),
        Value::String(text) => text.to_str()?.to_owned(),
        other => {
            return Err(mlua::Error::runtime(format!(
                "path value must be a string, got {}",
                other.type_name()
            )))
        }
    };
    let browse = parse_browse(row.get("browse")?)?;
    Ok(ToolbarItem::Path {
        id,
        label,
        value,
        browse,
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
    let off_color = color_from_value(row.get("off_color")?)?.unwrap_or(DEFAULT_TOGGLE_OFF);
    Ok(ToolbarItem::Toggle {
        id,
        label,
        value,
        on_color,
        off_color,
        align,
    })
}

fn parse_message_item(row: &Table, align: ToolbarAlign) -> mlua::Result<ToolbarItem> {
    let id = required_nonempty_string(row.get("id")?, "message id")?;
    let text = match row.get::<Value>("text")? {
        Value::Nil => String::new(),
        Value::String(text) => text.to_str()?.to_owned(),
        other => {
            return Err(mlua::Error::runtime(format!(
                "message text must be a string, got {}",
                other.type_name()
            )))
        }
    };
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
        let value: Value = lua.load(code).eval().expect("toolbar lua");
        parse_toolbar(value).expect("parse")
    }

    #[test]
    fn parse_toolbar_button_defaults_and_kinds() {
        let items = lua_toolbar(
            r#"{
              { command = "go", label = "Go" },
              { kind = "button", id = "next", command = "next", label = "Next", align = "right" },
              { kind = "divider" },
              { kind = "message", id = "progress", text = "1 of 2", align = "left" },
              { kind = "path", id = "output", label = "Output", value = "/tmp", browse = "directory", align = "right" },
              { kind = "toggle", id = "reviewed", label = "Reviewed", value = true },
            }"#,
        );
        assert_eq!(items.len(), 6);
        assert_eq!(items[0].command(), Some("go"));
        assert_eq!(items[0].label(), Some("Go"));
        assert_eq!(items[0].id(), Some("go"));
        assert_eq!(items[0].align(), ToolbarAlign::Left);
        match &items[1] {
            ToolbarItem::Button {
                id, command, align, ..
            } => {
                assert_eq!(id, "next");
                assert_eq!(command, "next");
                assert_eq!(*align, ToolbarAlign::Right);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            items[2],
            ToolbarItem::Divider {
                align: ToolbarAlign::Left,
                ..
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
            ToolbarItem::Path {
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
                assert_eq!(id, "reviewed");
                assert!(*value);
            }
            other => panic!("{other:?}"),
        }
    }
}
