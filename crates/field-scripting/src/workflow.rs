// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Workflow prototype registration and `field.workflow` bindings.

#![allow(missing_docs)]

use mlua::{Function, Table, UserData, UserDataFields, Value};

use crate::host::host_from_lua;
use crate::marker::color_from_value;
use crate::prototype::{base_properties, create_prototype_table, new_instance};
use crate::workflow_toolbar;

pub const DEFAULT_WORKFLOW_COLOR: [f32; 4] = [0.45, 0.45, 0.5, 1.0];

pub type WorkflowMeta = WorkflowBaseProperties;

#[derive(Clone, Debug)]
pub struct WorkflowBaseProperties {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub scopes: Vec<String>,
    pub row: i64,
    pub priority: f64,
    pub color: [f32; 4],
}

impl UserData for WorkflowBaseProperties {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |_, this| Ok(this.name.clone()));
        fields.add_field_method_get("display_name", |_, this| Ok(this.display_name.clone()));
        fields.add_field_method_get("description", |_, this| Ok(this.description.clone()));
        fields.add_field_method_get("scopes", |lua, this| scopes_to_lua(lua, &this.scopes));
        fields.add_field_method_get("row", |_, this| Ok(this.row));
        fields.add_field_method_get("priority", |_, this| Ok(this.priority));
        fields.add_field_method_get("color", |lua, this| color_to_table(lua, this.color));
    }
}

pub struct WorkflowDef {
    pub meta: WorkflowMeta,
    pub prototype: Table,
}

pub fn workflow_create_prototype(lua: &mlua::Lua, properties: Table) -> mlua::Result<Table> {
    let props = properties_from_lua(properties)?;
    let proto = create_prototype_table(lua)?;
    proto.set("__base_properties", props)?;
    proto.set(
        "name",
        lua.create_function(|_, this: Table| {
            Ok(base_properties::<WorkflowBaseProperties>(&this)?.name)
        })?,
    )?;
    proto.set(
        "display_name",
        lua.create_function(|_, this: Table| {
            Ok(base_properties::<WorkflowBaseProperties>(&this)?.display_name)
        })?,
    )?;
    proto.set(
        "description",
        lua.create_function(|_, this: Table| {
            Ok(base_properties::<WorkflowBaseProperties>(&this)?.description)
        })?,
    )?;
    proto.set(
        "scopes",
        lua.create_function(|lua, this: Table| {
            scopes_to_lua(
                lua,
                &base_properties::<WorkflowBaseProperties>(&this)?.scopes,
            )
        })?,
    )?;
    proto.set("on", lua.create_function(prototype_on)?)?;
    workflow_toolbar::install_methods(lua, &proto)?;
    Ok(proto)
}

pub fn workflow_new(lua: &mlua::Lua, prototype: Table) -> mlua::Result<Table> {
    new_instance(lua, prototype)
}

pub fn workflow_start(instance: &Table, payload: Table) -> mlua::Result<()> {
    if let Some(start) = table_method(instance, "start") {
        start.call::<()>((instance.clone(), payload))?;
    }
    Ok(())
}

pub fn workflow_run(lua: &mlua::Lua, name: &str, payload: Table) -> mlua::Result<Option<Table>> {
    host_from_lua(lua)?.workflow_run(lua, name, payload)
}

pub fn workflow_from_lua(
    lua: &mlua::Lua,
    properties: Table,
    func: Function,
) -> mlua::Result<WorkflowDef> {
    let prototype = workflow_create_prototype(lua, properties)?;
    let start =
        lua.create_function(move |_, (_this, payload): (Table, Table)| func.call::<()>(payload))?;
    prototype.set("start", start)?;
    workflow_from_prototype(prototype)
}

pub fn workflow_from_prototype(prototype: Table) -> mlua::Result<WorkflowDef> {
    let meta = base_properties::<WorkflowBaseProperties>(&prototype)?;
    if meta.name.is_empty() {
        return Err(mlua::Error::runtime("workflow prototype is missing a name"));
    }
    Ok(WorkflowDef { meta, prototype })
}

pub fn table_method(table: &Table, name: &str) -> Option<Function> {
    match table.get::<Value>(name) {
        Ok(Value::Function(func)) => Some(func),
        _ => None,
    }
}

pub fn prototype_is_stateful(prototype: &Table) -> bool {
    table_method(prototype, "suspend").is_some() || table_method(prototype, "resume").is_some()
}

pub fn instance_name(instance: &Table) -> mlua::Result<String> {
    call_reader(instance, "name")
}

pub fn instance_display_name(instance: &Table) -> mlua::Result<String> {
    call_reader(instance, "display_name")
}

fn call_reader(table: &Table, name: &str) -> mlua::Result<String> {
    match table.get::<Value>(name)? {
        Value::Function(func) => func.call(table.clone()),
        other => Err(mlua::Error::runtime(format!(
            "{name}() is required, got {}",
            other.type_name()
        ))),
    }
}

fn prototype_on(
    lua: &mlua::Lua,
    (this, event, handler): (Table, String, Function),
) -> mlua::Result<()> {
    if !is_field_event(&event) {
        return Err(mlua::Error::runtime(unknown_field_event(&event)));
    }
    let hooks = match this.raw_get::<Value>("__fa_hooks")? {
        Value::Table(table) => table,
        _ => {
            let table = lua.create_table()?;
            this.raw_set("__fa_hooks", table.clone())?;
            table
        }
    };
    let list = match hooks.raw_get::<Value>(event.as_str())? {
        Value::Table(table) => table,
        _ => {
            let table = lua.create_table()?;
            hooks.raw_set(event.as_str(), table.clone())?;
            table
        }
    };
    list.raw_set(list.raw_len() + 1, handler)?;
    Ok(())
}

fn properties_from_lua(properties: Table) -> mlua::Result<WorkflowBaseProperties> {
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
    Ok(WorkflowBaseProperties {
        name,
        display_name,
        description,
        scopes,
        row,
        priority,
        color,
    })
}

fn scopes_to_lua(lua: &mlua::Lua, scopes: &[String]) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for (index, scope) in scopes.iter().enumerate() {
        table.set(index + 1, scope.as_str())?;
    }
    Ok(table)
}

fn color_to_table(lua: &mlua::Lua, color: [f32; 4]) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for (index, component) in color.iter().enumerate() {
        table.set(index + 1, *component)?;
    }
    Ok(table)
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

fn is_field_event(event: &str) -> bool {
    matches!(
        event,
        "loaded"
            | "saved"
            | "detect_layout"
            | "session_loaded"
            | "session_saved"
            | "session_selected"
            | "composition_selected"
    )
}

fn unknown_field_event(event: &str) -> String {
    format!(
        "unknown event `{event}`; expected \"loaded\", \"detect_layout\", \"saved\", \"session_loaded\", \"session_saved\", \"session_selected\", or \"composition_selected\""
    )
}

/// Install `field.workflow` helpers.
pub fn bind_workflow_module(lua: &mlua::Lua, field: &mlua::Table) -> mlua::Result<()> {
    use mlua::MultiValue;

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
            let def = match (first, second) {
                (Value::Table(properties), Some(Value::Function(func))) => {
                    workflow_from_lua(lua, properties, func)?
                }
                (Value::Table(prototype), None | Some(Value::Nil)) => {
                    workflow_from_prototype(prototype)?
                }
                (other, _) => {
                    return Err(mlua::Error::runtime(format!(
                        "declare expects a prototype table or (properties, function), got {}",
                        other.type_name()
                    )))
                }
            };
            host_from_lua(lua)?
                .inner
                .borrow_mut()
                .workflows
                .insert(def.meta.name.clone(), def);
            Ok(())
        })?,
    )?;
    workflow.set(
        "run",
        lua.create_function(|lua, args: MultiValue| {
            let mut args = args.into_iter();
            let name = match args.next() {
                Some(Value::String(name)) => name.to_str()?.to_owned(),
                Some(other) => {
                    return Err(mlua::Error::runtime(format!(
                        "run expects a workflow name, got {}",
                        other.type_name()
                    )))
                }
                None => return Err(mlua::Error::runtime("run expects a workflow name")),
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
        })?,
    )?;
    workflow.set(
        "finish",
        lua.create_function(|lua, ()| host_from_lua(lua)?.finish_workflow(lua))?,
    )?;
    workflow.set(
        "cancel",
        lua.create_function(|lua, ()| host_from_lua(lua)?.cancel_workflow(lua))?,
    )?;
    field.set("workflow", workflow)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;
    use crate::host::{HostProfile, ScriptHost};

    #[test]
    fn field_workflow_declare_and_run() {
        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: None,
        })
        .unwrap();
        let out = host.eval(
            r#"(function() field.workflow.declare({name="ping",display_name="Ping",scopes={"run"}},function()end); return field.workflow.run("ping")~=nil end)()"#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("true"));
    }
}
