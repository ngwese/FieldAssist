// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use mlua::{Function, Table, Value};

use super::marker::color_from_value;

pub const SCOPE_DRAG_DROP: &str = "drag-drop";
pub const SCOPE_MENU: &str = "menu";
pub const DEFAULT_WORKFLOW_COLOR: [f32; 4] = [0.45, 0.45, 0.5, 1.0];

#[derive(Clone, Debug)]
pub struct WorkflowMeta {
    pub name: String,
    pub display_name: String,
    #[allow(dead_code)]
    pub description: String,
    pub scopes: Vec<String>,
    pub row: i64,
    pub priority: f64,
    pub color: [f32; 4],
}

pub struct WorkflowDef {
    pub meta: WorkflowMeta,
    pub prototype: Table,
}

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

const DEFAULT_TOGGLE_OFF: [f32; 4] = [0.55, 0.55, 0.6, 1.0];
const DEFAULT_TOGGLE_ON: [f32; 4] = [0.22, 0.72, 0.42, 1.0];

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

    pub fn command(&self) -> Option<&str> {
        match self {
            Self::Button { command, .. } => Some(command.as_str()),
            _ => None,
        }
    }

    pub fn label(&self) -> Option<&str> {
        match self {
            Self::Button { label, .. } | Self::Toggle { label, .. } => Some(label.as_str()),
            Self::Path { label, .. } => label.as_deref(),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DropLayout {
    pub rows: Vec<DropRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DropRow {
    pub cells: Vec<DropCell>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DropCell {
    pub name: String,
    pub display_name: String,
    pub color: [f32; 4],
    pub weight: f64,
}

pub fn workflow_meta_from_lua(properties: Table) -> mlua::Result<WorkflowMeta> {
    let name: String = properties.get("name")?;
    if name.is_empty() {
        return Err(mlua::Error::runtime("workflow name is required"));
    }
    let display_name: String = match properties.get::<Value>("display_name")? {
        Value::Nil => name.clone(),
        Value::String(value) => {
            let text = value.to_str()?.to_owned();
            if text.is_empty() {
                name.clone()
            } else {
                text
            }
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "display_name must be a string, got {}",
                other.type_name()
            )))
        }
    };
    let description: String = match properties.get::<Value>("description")? {
        Value::Nil => String::new(),
        Value::String(value) => value.to_str()?.to_owned(),
        other => {
            return Err(mlua::Error::runtime(format!(
                "description must be a string, got {}",
                other.type_name()
            )))
        }
    };
    let scopes = string_list_from_lua(properties.get("scopes")?)?;
    let (row, priority, color) = drop_spec_from_lua(properties.get("drop")?)?;
    Ok(WorkflowMeta {
        name,
        display_name,
        description,
        scopes,
        row,
        priority,
        color,
    })
}

pub fn create_prototype(lua: &mlua::Lua, properties: Table) -> mlua::Result<Table> {
    let meta = workflow_meta_from_lua(properties)?;
    let proto = lua.create_table()?;
    proto.set("__fa_name", meta.name.clone())?;
    proto.set("__fa_display_name", meta.display_name.clone())?;
    proto.set("__fa_description", meta.description.clone())?;
    proto.set("__fa_row", meta.row)?;
    proto.set("__fa_priority", meta.priority)?;
    let scopes = lua.create_table()?;
    for (index, scope) in meta.scopes.iter().enumerate() {
        scopes.set(index + 1, scope.as_str())?;
    }
    proto.set("__fa_scopes", scopes)?;
    let color = lua.create_table()?;
    for (index, component) in meta.color.iter().enumerate() {
        color.set(index + 1, *component)?;
    }
    proto.set("__fa_color", color)?;
    proto.set("on", lua.create_function(prototype_on)?)?;
    proto.set("set_toolbar", lua.create_function(prototype_set_toolbar)?)?;
    proto.set("set_item", lua.create_function(prototype_set_item)?)?;
    Ok(proto)
}

pub fn workflow_from_lua(
    lua: &mlua::Lua,
    properties: Table,
    func: Function,
) -> mlua::Result<WorkflowDef> {
    let prototype = create_prototype(lua, properties)?;
    let start =
        lua.create_function(move |_, (_this, payload): (Table, Table)| func.call::<()>(payload))?;
    prototype.set("start", start)?;
    workflow_from_prototype(prototype)
}

pub fn workflow_from_prototype(prototype: Table) -> mlua::Result<WorkflowDef> {
    let name: String = prototype.get("__fa_name")?;
    if name.is_empty() {
        return Err(mlua::Error::runtime("workflow prototype is missing a name"));
    }
    let display_name: String = prototype
        .get::<Option<String>>("__fa_display_name")?
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| name.clone());
    let description: String = prototype
        .get::<Option<String>>("__fa_description")?
        .unwrap_or_default();
    let scopes = match prototype.get::<Value>("__fa_scopes")? {
        Value::Nil => Vec::new(),
        value => string_list_from_lua(value)?,
    };
    let row = optional_i64(prototype.get("__fa_row")?, 1)?;
    let priority = optional_positive_number(prototype.get("__fa_priority")?, 1.0)?;
    let color = color_from_value(prototype.get("__fa_color")?)?.unwrap_or(DEFAULT_WORKFLOW_COLOR);
    Ok(WorkflowDef {
        meta: WorkflowMeta {
            name,
            display_name,
            description,
            scopes,
            row,
            priority,
            color,
        },
        prototype,
    })
}

pub fn prototype_method(prototype: &Table, name: &str) -> Option<Function> {
    match prototype.get::<Value>(name) {
        Ok(Value::Function(func)) => Some(func),
        _ => None,
    }
}

pub fn prototype_is_stateful(prototype: &Table) -> bool {
    prototype_method(prototype, "suspend").is_some()
        || prototype_method(prototype, "resume").is_some()
}

pub fn toolbar_from_prototype(prototype: &Table) -> Vec<ToolbarItem> {
    parse_toolbar(prototype.get("__fa_toolbar").unwrap_or(Value::Nil)).unwrap_or_default()
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

fn prototype_on(
    lua: &mlua::Lua,
    (this, event, handler): (Table, String, Function),
) -> mlua::Result<()> {
    if event != "command" {
        return Err(mlua::Error::runtime(format!(
            "unknown workflow event `{event}`; expected \"command\""
        )));
    }
    this.set("__fa_command", handler)?;
    let _ = lua;
    Ok(())
}

fn prototype_set_toolbar(lua: &mlua::Lua, (this, items): (Table, Value)) -> mlua::Result<()> {
    parse_toolbar(items.clone())?;
    this.set("__fa_toolbar", items)?;
    super::host::host_from_lua(lua)?.toolbar_changed(&this)
}

fn prototype_set_item(
    lua: &mlua::Lua,
    (this, id, props): (Table, String, Table),
) -> mlua::Result<()> {
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

fn drop_spec_from_lua(value: Value) -> mlua::Result<(i64, f64, [f32; 4])> {
    match value {
        Value::Nil => Ok((1, 1.0, DEFAULT_WORKFLOW_COLOR)),
        Value::Table(table) => {
            let row = optional_i64(table.get("row")?, 1)?;
            let priority = optional_positive_number(table.get("priority")?, 1.0)?;
            let color = color_from_value(table.get("color")?)?.unwrap_or(DEFAULT_WORKFLOW_COLOR);
            Ok((row, priority, color))
        }
        other => Err(mlua::Error::runtime(format!(
            "drop must be a table, got {}",
            other.type_name()
        ))),
    }
}

fn optional_i64(value: Value, default: i64) -> mlua::Result<i64> {
    match value {
        Value::Nil => Ok(default),
        Value::Integer(value) => Ok(value),
        Value::Number(value) if value.fract() == 0.0 => Ok(value as i64),
        other => Err(mlua::Error::runtime(format!(
            "drop.row must be an integer, got {}",
            other.type_name()
        ))),
    }
}

fn optional_positive_number(value: Value, default: f64) -> mlua::Result<f64> {
    let number = match value {
        Value::Nil => return Ok(default),
        Value::Integer(value) => value as f64,
        Value::Number(value) => value,
        other => {
            return Err(mlua::Error::runtime(format!(
                "drop.priority must be a number, got {}",
                other.type_name()
            )))
        }
    };
    if number <= 0.0 || !number.is_finite() {
        return Err(mlua::Error::runtime("drop.priority must be greater than 0"));
    }
    Ok(number)
}

fn string_list_from_lua(value: Value) -> mlua::Result<Vec<String>> {
    match value {
        Value::Table(table) => {
            let mut scopes = Vec::new();
            for value in table.sequence_values::<Value>() {
                match value? {
                    Value::String(text) => scopes.push(text.to_str()?.to_owned()),
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "scopes must be strings, got {}",
                            other.type_name()
                        )))
                    }
                }
            }
            Ok(scopes)
        }
        other => Err(mlua::Error::runtime(format!(
            "scopes must be a list of strings, got {}",
            other.type_name()
        ))),
    }
}

/// Collect drag-drop workflows into rows. Missing row numbers are not padded.
/// Within a row, higher `priority` is first (left); ties sort by `display_name`.
/// Cell `weight` is `priority / sum(priorities in the row)`.
pub fn layout_drop_targets<'a>(
    workflows: impl IntoIterator<Item = &'a WorkflowMeta>,
    scope: &str,
) -> DropLayout {
    let mut grouped: std::collections::BTreeMap<i64, Vec<&WorkflowMeta>> =
        std::collections::BTreeMap::new();
    for workflow in workflows {
        if !workflow.scopes.iter().any(|name| name == scope) {
            continue;
        }
        grouped.entry(workflow.row).or_default().push(workflow);
    }
    let mut rows = Vec::new();
    for mut group in grouped.into_values() {
        group.sort_by(|a, b| {
            b.priority
                .partial_cmp(&a.priority)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.display_name.cmp(&b.display_name))
                .then_with(|| a.name.cmp(&b.name))
        });
        let sum: f64 = group.iter().map(|workflow| workflow.priority).sum();
        let cells = group
            .into_iter()
            .map(|workflow| DropCell {
                name: workflow.name.clone(),
                display_name: workflow.display_name.clone(),
                color: workflow.color,
                weight: if sum > 0.0 {
                    workflow.priority / sum
                } else {
                    0.0
                },
            })
            .collect();
        rows.push(DropRow { cells });
    }
    DropLayout { rows }
}

/// Workflows bound to the `"menu"` scope, sorted by `display_name` then `name`.
pub fn workflows_for_menu(workflows: &[WorkflowMeta]) -> Vec<&WorkflowMeta> {
    let mut items: Vec<&WorkflowMeta> = workflows
        .iter()
        .filter(|workflow| workflow.scopes.iter().any(|scope| scope == SCOPE_MENU))
        .collect();
    items.sort_by(|a, b| {
        a.display_name
            .cmp(&b.display_name)
            .then_with(|| a.name.cmp(&b.name))
    });
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(name: &str, display: &str, row: i64, priority: f64, scopes: &[&str]) -> WorkflowMeta {
        WorkflowMeta {
            name: name.into(),
            display_name: display.into(),
            description: String::new(),
            scopes: scopes.iter().map(|scope| (*scope).to_string()).collect(),
            row,
            priority,
            color: DEFAULT_WORKFLOW_COLOR,
        }
    }

    #[test]
    fn layout_groups_rows_without_padding() {
        let workflows = [
            meta("a", "A", 1, 1.0, &[SCOPE_DRAG_DROP]),
            meta("c", "C", 3, 1.0, &[SCOPE_DRAG_DROP]),
            meta("other", "Other", 2, 1.0, &["later"]),
        ];
        let layout = layout_drop_targets(&workflows, SCOPE_DRAG_DROP);
        assert_eq!(layout.rows.len(), 2);
        assert_eq!(layout.rows[0].cells[0].name, "a");
        assert_eq!(layout.rows[1].cells[0].name, "c");
    }

    #[test]
    fn layout_sorts_priority_then_display_name_and_weights() {
        let workflows = [
            meta("low_b", "Beta", 1, 1.0, &[SCOPE_DRAG_DROP]),
            meta("high", "Wide", 1, 2.0, &[SCOPE_DRAG_DROP]),
            meta("low_a", "Alpha", 1, 1.0, &[SCOPE_DRAG_DROP]),
        ];
        let layout = layout_drop_targets(&workflows, SCOPE_DRAG_DROP);
        let names: Vec<_> = layout.rows[0]
            .cells
            .iter()
            .map(|cell| cell.name.as_str())
            .collect();
        assert_eq!(names, ["high", "low_a", "low_b"]);
        let weights: Vec<_> = layout.rows[0]
            .cells
            .iter()
            .map(|cell| (cell.weight * 100.0).round() as i32)
            .collect();
        assert_eq!(weights, [50, 25, 25]);
    }

    #[test]
    fn menu_workflows_sorted_by_display_name() {
        let workflows = [
            meta("zeta", "Zebra", 1, 1.0, &[SCOPE_MENU]),
            meta("drop", "Drop Only", 1, 1.0, &[SCOPE_DRAG_DROP]),
            meta("alpha", "Apple", 1, 1.0, &[SCOPE_DRAG_DROP, SCOPE_MENU]),
            meta("same_b", "Same", 1, 1.0, &[SCOPE_MENU]),
            meta("same_a", "Same", 1, 1.0, &[SCOPE_MENU]),
        ];
        let names: Vec<_> = workflows_for_menu(&workflows)
            .into_iter()
            .map(|workflow| workflow.name.as_str())
            .collect();
        assert_eq!(names, ["alpha", "same_a", "same_b", "zeta"]);
    }

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
