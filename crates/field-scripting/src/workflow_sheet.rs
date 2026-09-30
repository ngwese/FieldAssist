// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Workflow modal sheet model (panes, guide, body controls, button bar).

#![allow(missing_docs)]

use mlua::{Function, Table, Value};

use crate::host::host_from_lua;
use crate::workflow_toolbar::{
    control_is_control, parse_control_item, stamp_control_owner, toolbar_row_id, ToolbarItem,
};

/// Snapshot of an open workflow sheet for the host UI.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetSnapshot {
    pub title: String,
    pub panes: Vec<SheetPane>,
    pub current: String,
}

/// One named pane in a sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetPane {
    pub id: String,
    pub name: String,
    pub text: String,
    pub controls: Vec<ToolbarItem>,
    pub buttons: Vec<ToolbarItem>,
}

pub fn install_methods(lua: &mlua::Lua, proto: &Table) -> mlua::Result<()> {
    proto.set("set_sheet", lua.create_function(set_sheet)?)?;
    proto.set("set_pane", lua.create_function(set_pane)?)?;
    proto.set("go", lua.create_function(go)?)?;
    proto.set("close_sheet", lua.create_function(close_sheet)?)?;
    proto.set("defer", lua.create_function(defer_fn)?)?;
    Ok(())
}

pub fn sheet_from_table(table: &Table) -> Option<SheetSnapshot> {
    parse_sheet(table.get("__fa_sheet").unwrap_or(Value::Nil))
        .ok()
        .flatten()
}

pub fn has_sheet(table: &Table) -> bool {
    matches!(
        table.get::<Value>("__fa_sheet").unwrap_or(Value::Nil),
        Value::Table(_)
    )
}

pub fn parse_sheet(value: Value) -> mlua::Result<Option<SheetSnapshot>> {
    match value {
        Value::Nil => Ok(None),
        Value::Table(table) => Ok(Some(parse_sheet_table(&table)?)),
        other => Err(mlua::Error::runtime(format!(
            "sheet must be a table or nil, got {}",
            other.type_name()
        ))),
    }
}

fn parse_sheet_table(table: &Table) -> mlua::Result<SheetSnapshot> {
    let title = match table.get::<Value>("title")? {
        Value::Nil => String::new(),
        Value::String(text) => text.to_str()?.to_owned(),
        other => {
            return Err(mlua::Error::runtime(format!(
                "sheet title must be a string, got {}",
                other.type_name()
            )))
        }
    };
    let panes_value = table.get::<Value>("panes")?;
    let panes = parse_panes(panes_value)?;
    if panes.is_empty() {
        return Err(mlua::Error::runtime("sheet panes must be a non-empty list"));
    }
    let current = match table.get::<Value>("current")? {
        Value::Nil => panes[0].id.clone(),
        Value::String(text) => {
            let id = text.to_str()?.to_owned();
            if !panes.iter().any(|pane| pane.id == id) {
                return Err(mlua::Error::runtime(format!(
                    "sheet current pane `{id}` is not in panes"
                )));
            }
            id
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "sheet current must be a string, got {}",
                other.type_name()
            )))
        }
    };
    Ok(SheetSnapshot {
        title,
        panes,
        current,
    })
}

fn parse_panes(value: Value) -> mlua::Result<Vec<SheetPane>> {
    let Value::Table(table) = value else {
        return Err(mlua::Error::runtime(format!(
            "sheet panes must be a list, got {}",
            value.type_name()
        )));
    };
    let mut panes = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for row in table.sequence_values::<Value>() {
        let Value::Table(row) = row? else {
            return Err(mlua::Error::runtime("sheet panes must be tables"));
        };
        let pane = parse_pane(&row)?;
        if !seen.insert(pane.id.clone()) {
            return Err(mlua::Error::runtime(format!(
                "duplicate sheet pane id `{}`",
                pane.id
            )));
        }
        panes.push(pane);
    }
    Ok(panes)
}

fn parse_pane(row: &Table) -> mlua::Result<SheetPane> {
    let id = required_string(row.get("id")?, "pane id")?;
    let name = match optional_string(row.get("name")?)? {
        Some(name) => name,
        None => id.clone(),
    };
    let text = match row.get::<Value>("text")? {
        Value::Nil => String::new(),
        Value::String(text) => text.to_str()?.to_owned(),
        other => {
            return Err(mlua::Error::runtime(format!(
                "pane text must be a string, got {}",
                other.type_name()
            )))
        }
    };
    let controls = parse_control_list(row.get("controls")?, "controls")?;
    let buttons = parse_buttons(row.get("buttons")?)?;
    let mut ids = std::collections::HashSet::new();
    for item in controls.iter().chain(buttons.iter()) {
        if let Some(item_id) = item.id() {
            if !ids.insert(item_id.to_owned()) {
                return Err(mlua::Error::runtime(format!(
                    "duplicate control id `{item_id}` across sheet panes"
                )));
            }
        }
    }
    Ok(SheetPane {
        id,
        name,
        text,
        controls,
        buttons,
    })
}

fn parse_control_list(value: Value, what: &str) -> mlua::Result<Vec<ToolbarItem>> {
    match value {
        Value::Nil => Ok(Vec::new()),
        Value::Table(table) => {
            let mut items = Vec::new();
            for row in table.sequence_values::<Value>() {
                let Value::Table(row) = row? else {
                    return Err(mlua::Error::runtime(format!(
                        "sheet {what} must be field.ui controls"
                    )));
                };
                if !control_is_control(&row)? {
                    return Err(mlua::Error::runtime(format!(
                        "sheet {what} must be field.ui controls"
                    )));
                }
                items.push(parse_control_item(&row)?);
            }
            Ok(items)
        }
        other => Err(mlua::Error::runtime(format!(
            "sheet {what} must be a list or nil, got {}",
            other.type_name()
        ))),
    }
}

fn parse_buttons(value: Value) -> mlua::Result<Vec<ToolbarItem>> {
    // Sheet button bars default to the right cluster; use align = "left" for Back.
    if let Value::Table(table) = &value {
        for row in table.sequence_values::<Value>() {
            let Value::Table(ctrl) = row? else {
                continue;
            };
            if matches!(ctrl.get::<Value>("align")?, Value::Nil) {
                ctrl.set("align", "right")?;
            }
        }
    }
    let items = parse_control_list(value, "buttons")?;
    for item in &items {
        match item {
            ToolbarItem::Button { .. } => {}
            other => {
                return Err(mlua::Error::runtime(format!(
                    "sheet buttons must be field.ui.button, got {:?}",
                    other.id().unwrap_or("<divider>")
                )));
            }
        }
    }
    Ok(items)
}

fn set_sheet(lua: &mlua::Lua, (this, props): (Table, Value)) -> mlua::Result<()> {
    let Value::Table(props) = props else {
        return Err(mlua::Error::runtime(format!(
            "set_sheet expects a table, got {}",
            props.type_name()
        )));
    };
    let sheet = lua.create_table()?;
    if let Ok(Value::String(title)) = props.get::<Value>("title") {
        sheet.set("title", title)?;
    } else {
        let title = match this.get::<Value>("display_name")? {
            Value::Function(func) => func.call::<String>(this.clone())?,
            Value::String(text) => text.to_str()?.to_owned(),
            _ => match this.get::<Value>("name")? {
                Value::Function(func) => func.call::<String>(this.clone())?,
                Value::String(text) => text.to_str()?.to_owned(),
                _ => String::new(),
            },
        };
        sheet.set("title", title)?;
    }
    let panes = match props.get::<Value>("panes")? {
        Value::Table(panes) => panes,
        other => {
            return Err(mlua::Error::runtime(format!(
                "set_sheet panes must be a list, got {}",
                other.type_name()
            )))
        }
    };
    stamp_pane_controls(&panes, &this)?;
    // Cross-pane id uniqueness.
    let mut all_ids = std::collections::HashSet::new();
    for pane_value in panes.sequence_values::<Value>() {
        let Value::Table(pane) = pane_value? else {
            continue;
        };
        collect_control_ids(&pane, "controls", &mut all_ids)?;
        collect_control_ids(&pane, "buttons", &mut all_ids)?;
    }
    parse_panes(Value::Table(panes.clone()))?;
    sheet.set("panes", panes)?;
    if let Ok(Value::String(current)) = props.get::<Value>("current") {
        sheet.set("current", current)?;
    }
    parse_sheet_table(&sheet)?;
    this.set("__fa_sheet", sheet)?;
    host_from_lua(lua)?.sheet_opened(&this)
}

fn stamp_pane_controls(panes: &Table, owner: &Table) -> mlua::Result<()> {
    for pane_value in panes.sequence_values::<Value>() {
        let Value::Table(pane) = pane_value? else {
            return Err(mlua::Error::runtime("sheet panes must be tables"));
        };
        stamp_list(pane.get("controls")?, owner)?;
        stamp_list(pane.get("buttons")?, owner)?;
        // Validate buttons are buttons only.
        if let Value::Table(buttons) = pane.get::<Value>("buttons")? {
            for row in buttons.sequence_values::<Value>() {
                let Value::Table(row) = row? else {
                    return Err(mlua::Error::runtime(
                        "sheet buttons must be field.ui.button",
                    ));
                };
                if let Ok(kind) = row.get::<String>("kind") {
                    if kind != "button" {
                        return Err(mlua::Error::runtime(
                            "sheet buttons must be field.ui.button",
                        ));
                    }
                }
                parse_control_item(&row)?;
            }
        }
        if let Value::Table(controls) = pane.get::<Value>("controls")? {
            for row in controls.sequence_values::<Value>() {
                let Value::Table(row) = row? else {
                    return Err(mlua::Error::runtime(
                        "sheet controls must be field.ui controls",
                    ));
                };
                parse_control_item(&row)?;
            }
        }
    }
    Ok(())
}

fn stamp_list(value: Value, owner: &Table) -> mlua::Result<()> {
    let Value::Table(table) = value else {
        return Ok(());
    };
    for row in table.sequence_values::<Value>() {
        let Value::Table(row) = row? else {
            continue;
        };
        if control_is_control(&row)? {
            stamp_control_owner(&row, owner)?;
        }
    }
    Ok(())
}

fn collect_control_ids(
    pane: &Table,
    key: &str,
    ids: &mut std::collections::HashSet<String>,
) -> mlua::Result<()> {
    let Value::Table(table) = pane.get::<Value>(key)? else {
        return Ok(());
    };
    for row in table.sequence_values::<Value>() {
        let Value::Table(row) = row? else {
            continue;
        };
        if let Some(id) = toolbar_row_id(&row)? {
            if !ids.insert(id.clone()) {
                return Err(mlua::Error::runtime(format!(
                    "duplicate control id `{id}` across sheet panes"
                )));
            }
        }
    }
    Ok(())
}

fn set_pane(lua: &mlua::Lua, (this, id, props): (Table, String, Table)) -> mlua::Result<()> {
    if id.is_empty() {
        return Err(mlua::Error::runtime("set_pane id is required"));
    }
    let sheet = sheet_table(&this)?;
    let panes = match sheet.get::<Value>("panes")? {
        Value::Table(panes) => panes,
        _ => return Err(mlua::Error::runtime("sheet has no panes")),
    };
    let pane = find_pane(&panes, &id)?;
    if let Ok(Value::String(text)) = props.get::<Value>("text") {
        pane.set("text", text)?;
    }
    if let Ok(Value::Table(controls)) = props.get::<Value>("controls") {
        stamp_list(Value::Table(controls.clone()), &this)?;
        for row in controls.sequence_values::<Value>() {
            let Value::Table(row) = row? else {
                return Err(mlua::Error::runtime(
                    "sheet controls must be field.ui controls",
                ));
            };
            parse_control_item(&row)?;
        }
        pane.set("controls", controls)?;
    }
    if let Ok(Value::Table(buttons)) = props.get::<Value>("buttons") {
        stamp_list(Value::Table(buttons.clone()), &this)?;
        for row in buttons.sequence_values::<Value>() {
            let Value::Table(row) = row? else {
                return Err(mlua::Error::runtime(
                    "sheet buttons must be field.ui.button",
                ));
            };
            if let Ok(kind) = row.get::<String>("kind") {
                if kind != "button" {
                    return Err(mlua::Error::runtime(
                        "sheet buttons must be field.ui.button",
                    ));
                }
            }
            parse_control_item(&row)?;
        }
        pane.set("buttons", buttons)?;
    }
    parse_sheet_table(&sheet)?;
    host_from_lua(lua)?.toolbar_changed(&this)
}

fn go(lua: &mlua::Lua, (this, id): (Table, String)) -> mlua::Result<()> {
    if id.is_empty() {
        return Err(mlua::Error::runtime("go pane id is required"));
    }
    let sheet = sheet_table(&this)?;
    let panes = match sheet.get::<Value>("panes")? {
        Value::Table(panes) => panes,
        _ => return Err(mlua::Error::runtime("sheet has no panes")),
    };
    let _ = find_pane(&panes, &id)?;
    sheet.set("current", id)?;
    host_from_lua(lua)?.toolbar_changed(&this)
}

fn close_sheet(lua: &mlua::Lua, this: Table) -> mlua::Result<()> {
    this.set("__fa_sheet", Value::Nil)?;
    host_from_lua(lua)?.sheet_closed(&this)
}

fn defer_fn(lua: &mlua::Lua, (this, func): (Table, Function)) -> mlua::Result<()> {
    host_from_lua(lua)?.defer_workflow(this, func)
}

fn sheet_table(this: &Table) -> mlua::Result<Table> {
    match this.get::<Value>("__fa_sheet")? {
        Value::Table(sheet) => Ok(sheet),
        _ => Err(mlua::Error::runtime(
            "set_pane/go require an open sheet; call set_sheet first",
        )),
    }
}

fn find_pane(panes: &Table, id: &str) -> mlua::Result<Table> {
    for row in panes.sequence_values::<Value>() {
        let Value::Table(row) = row? else {
            continue;
        };
        if row.get::<String>("id")?.as_str() == id {
            return Ok(row);
        }
    }
    Err(mlua::Error::runtime(format!("no sheet pane `{id}`")))
}

/// Look up a control by id across the sheet's panes (body and buttons).
pub fn sheet_control_row(table: &Table, id: &str) -> mlua::Result<Table> {
    let sheet = sheet_table(table)?;
    let panes = match sheet.get::<Value>("panes")? {
        Value::Table(panes) => panes,
        _ => return Err(mlua::Error::runtime("sheet has no panes")),
    };
    for pane_value in panes.sequence_values::<Value>() {
        let Value::Table(pane) = pane_value? else {
            continue;
        };
        if let Some(row) = find_in_list(pane.get("controls")?, id)? {
            return Ok(row);
        }
        if let Some(row) = find_in_list(pane.get("buttons")?, id)? {
            return Ok(row);
        }
    }
    Err(mlua::Error::runtime(format!("no sheet control `{id}`")))
}

fn find_in_list(value: Value, id: &str) -> mlua::Result<Option<Table>> {
    let Value::Table(table) = value else {
        return Ok(None);
    };
    for row in table.sequence_values::<Value>() {
        let Value::Table(row) = row? else {
            continue;
        };
        if toolbar_row_id(&row)?.as_deref() == Some(id) {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

fn required_string(value: Value, what: &str) -> mlua::Result<String> {
    match optional_string(value)? {
        Some(text) => Ok(text),
        None => Err(mlua::Error::runtime(format!("{what} is required"))),
    }
}

fn optional_string(value: Value) -> mlua::Result<Option<String>> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_toolbar::ui_namespace;

    fn eval_sheet(code: &str) -> SheetSnapshot {
        let lua = mlua::Lua::new();
        let ui = ui_namespace(&lua).expect("ui");
        lua.globals().set("ui", ui).expect("global");
        // Minimal owner table with display_name for title default.
        let owner = lua.create_table().unwrap();
        owner
            .set(
                "display_name",
                lua.create_function(|_, (): ()| Ok("Ingest".to_string()))
                    .unwrap(),
            )
            .unwrap();
        owner
            .set(
                "name",
                lua.create_function(|_, (): ()| Ok("ingest".to_string()))
                    .unwrap(),
            )
            .unwrap();
        // Host not available — parse via raw table construction.
        let value: Value = lua.load(code).eval().expect("sheet lua");
        let Value::Table(props) = value else {
            panic!("expected table");
        };
        let sheet = lua.create_table().unwrap();
        sheet.set("title", "Ingest").unwrap();
        sheet
            .set("panes", props.get::<Value>("panes").unwrap())
            .unwrap();
        if let Ok(Value::String(current)) = props.get::<Value>("current") {
            sheet.set("current", current).unwrap();
        }
        parse_sheet_table(&sheet).expect("parse")
    }

    #[test]
    fn parse_sheet_panes_and_buttons() {
        let snap = eval_sheet(
            r#"{
              panes = {
                {
                  id = "configure",
                  name = "Configure",
                  text = "Choose a source.",
                  controls = {
                    ui.path_entry({ id = "source", label = "Source", value = "" }),
                    ui.progress({ id = "p", value = 25 }),
                    ui.log({ id = "log", text = "" }),
                  },
                  buttons = {
                    ui.button({ id = "next", label = "Next" }),
                    ui.button({ id = "back", label = "Back", align = "left" }),
                  },
                },
              },
            }"#,
        );
        assert_eq!(snap.panes.len(), 1);
        assert_eq!(snap.current, "configure");
        assert_eq!(snap.panes[0].controls.len(), 3);
        assert_eq!(snap.panes[0].buttons.len(), 2);
        assert!(matches!(
            snap.panes[0].buttons[0],
            ToolbarItem::Button {
                align: crate::workflow_toolbar::ToolbarAlign::Right,
                ..
            }
        ));
        assert!(matches!(
            snap.panes[0].buttons[1],
            ToolbarItem::Button {
                align: crate::workflow_toolbar::ToolbarAlign::Left,
                ..
            }
        ));
        assert!(matches!(
            &snap.panes[0].controls[1],
            ToolbarItem::Progress {
                value: Some(25.0),
                loading: false,
                ..
            }
        ));
    }

    #[test]
    fn buttons_reject_non_button() {
        let lua = mlua::Lua::new();
        let ui = ui_namespace(&lua).unwrap();
        lua.globals().set("ui", ui).unwrap();
        let value: Value = lua
            .load(
                r#"{
                  id = "p",
                  name = "P",
                  buttons = { ui.message({ id = "m", text = "x" }) },
                }"#,
            )
            .eval()
            .unwrap();
        let Value::Table(row) = value else {
            panic!();
        };
        let err = parse_pane(&row).unwrap_err().to_string();
        assert!(err.contains("button"), "{err}");
    }
}
