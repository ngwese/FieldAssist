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
