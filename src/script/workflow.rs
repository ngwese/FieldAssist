// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use mlua::{Function, Table, Value};

use super::marker::color_from_value;

pub const SCOPE_DRAG_DROP: &str = "drag-drop";
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolbarItem {
    pub command: String,
    pub label: String,
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
                    return Err(mlua::Error::runtime(
                        "toolbar items must be tables with command and label",
                    ));
                };
                let command: String = row.get("command")?;
                if command.is_empty() {
                    return Err(mlua::Error::runtime("toolbar command is required"));
                }
                let label: String = match row.get::<Value>("label")? {
                    Value::Nil => command.clone(),
                    Value::String(text) => {
                        let text = text.to_str()?.to_owned();
                        if text.is_empty() {
                            command.clone()
                        } else {
                            text
                        }
                    }
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "toolbar label must be a string, got {}",
                            other.type_name()
                        )))
                    }
                };
                items.push(ToolbarItem { command, label });
            }
            Ok(items)
        }
        other => Err(mlua::Error::runtime(format!(
            "toolbar must be a list of items or nil, got {}",
            other.type_name()
        ))),
    }
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
}
