// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Shared Lua helpers.

use mlua::Value;

/// Optional string from Lua (`nil` → `None`).
pub fn optional_lua_string(value: Value) -> mlua::Result<Option<String>> {
    match value {
        Value::Nil => Ok(None),
        Value::String(s) => Ok(Some(s.to_str()?.to_owned())),
        other => Err(mlua::Error::runtime(format!(
            "expected string or nil, got {}",
            other.type_name()
        ))),
    }
}

/// Read a string→string map from a Lua table.
pub fn string_map_from_lua(
    value: Value,
) -> mlua::Result<std::collections::BTreeMap<String, String>> {
    let mut map = std::collections::BTreeMap::new();
    match value {
        Value::Nil => Ok(map),
        Value::Table(table) => {
            for pair in table.pairs::<Value, Value>() {
                let (key, val) = pair?;
                let k = match key {
                    Value::String(s) => s.to_str()?.to_owned(),
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "property keys must be strings, got {}",
                            other.type_name()
                        )))
                    }
                };
                let v = match val {
                    Value::String(s) => s.to_str()?.to_owned(),
                    Value::Nil => {
                        return Err(mlua::Error::runtime(format!(
                            "property `{k}` cannot be nil"
                        )))
                    }
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "property `{k}` must be a string, got {}",
                            other.type_name()
                        )))
                    }
                };
                map.insert(k, v);
            }
            Ok(map)
        }
        other => Err(mlua::Error::runtime(format!(
            "properties must be a table, got {}",
            other.type_name()
        ))),
    }
}

/// Write a string→string map to a Lua table.
pub fn string_map_to_lua(
    lua: &mlua::Lua,
    map: &std::collections::BTreeMap<String, String>,
) -> mlua::Result<mlua::Table> {
    let table = lua.create_table()?;
    for (k, v) in map {
        table.set(k.as_str(), v.as_str())?;
    }
    Ok(table)
}

/// Convert a [`field_variables::VariableTable`] to a Lua array of
/// `{ name, value, description? }` rows (also sets map-style `t[name] = value`).
pub fn variables_to_lua(
    lua: &mlua::Lua,
    table: &field_variables::VariableTable,
) -> mlua::Result<mlua::Table> {
    let out = lua.create_table()?;
    for (index, entry) in table.entries().iter().enumerate() {
        let row = lua.create_table()?;
        row.set("name", entry.name.as_str())?;
        row.set("value", entry.value.as_str())?;
        row.set("scope", entry.scope.as_str())?;
        if let Some(desc) = &entry.description {
            row.set("description", desc.as_str())?;
        }
        out.set(index + 1, row)?;
        out.set(entry.name.as_str(), entry.value.as_str())?;
    }
    Ok(out)
}

/// Parse variables from a Lua table.
///
/// Accepts either:
/// - a string→string map (`{ title = "x" }`)
/// - an array of `{ name, value, description? }` rows
pub fn variables_from_lua(
    value: Value,
    scope: &str,
) -> mlua::Result<field_variables::VariableTable> {
    use field_variables::{VariableEntry, VariableTable};

    let mut table = VariableTable::new();
    match value {
        Value::Nil => Ok(table),
        Value::Table(lua_table) => {
            // Prefer array rows when present.
            let len = lua_table.raw_len();
            if len > 0 {
                for i in 1..=len {
                    let row: Value = lua_table.get(i)?;
                    match row {
                        Value::Table(row) => {
                            let name: String = row.get("name")?;
                            let value: String = row.get("value")?;
                            let description: Option<String> = row.get("description")?;
                            let mut entry = VariableEntry::new(scope, name, value);
                            if let Some(desc) = description {
                                entry = entry.with_description(desc);
                            }
                            table.upsert(entry);
                        }
                        other => {
                            return Err(mlua::Error::runtime(format!(
                                "variable row must be a table, got {}",
                                other.type_name()
                            )));
                        }
                    }
                }
                return Ok(table);
            }
            // Map form: string keys → string values.
            for pair in lua_table.pairs::<Value, Value>() {
                let (key, val) = pair?;
                let Value::String(k) = key else {
                    continue; // skip non-string keys (e.g. leftover)
                };
                let name = k.to_str()?.to_owned();
                let value = match val {
                    Value::String(s) => s.to_str()?.to_owned(),
                    Value::Table(t) => {
                        let v: String = t.get("value")?;
                        let description: Option<String> = t.get("description")?;
                        let mut entry = VariableEntry::new(scope, name, v);
                        if let Some(desc) = description {
                            entry = entry.with_description(desc);
                        }
                        table.upsert(entry);
                        continue;
                    }
                    Value::Nil => continue,
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "variable `{name}` must be a string or table, got {}",
                            other.type_name()
                        )));
                    }
                };
                table.upsert(VariableEntry::new(scope, name, value));
            }
            Ok(table)
        }
        other => Err(mlua::Error::runtime(format!(
            "variables must be a table, got {}",
            other.type_name()
        ))),
    }
}
