// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use mlua::{Table, Value};

#[derive(Clone, Debug)]
pub struct ChannelLayoutDef {
    pub name: String,
    pub description: String,
    pub channels: BTreeMap<usize, String>,
    pub monitor: Option<serde_json::Value>,
}

impl ChannelLayoutDef {
    pub fn monitor_chain_id(&self) -> Option<&str> {
        self.monitor
            .as_ref()
            .and_then(|value| value.get("chain"))
            .and_then(|value| value.as_str())
    }
}

pub fn layout_from_lua(table: Table) -> mlua::Result<ChannelLayoutDef> {
    let name: String = table.get("name")?;
    if name.is_empty() {
        return Err(mlua::Error::runtime("layout name is required"));
    }
    let description: String = table.get("description").unwrap_or_default();
    let channels_tbl: Table = table.get("channels")?;
    let mut channels = BTreeMap::new();
    for pair in channels_tbl.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let index = channel_index(key)?;
        let label = match value {
            Value::String(text) => text.to_str()?.to_owned(),
            other => {
                return Err(mlua::Error::runtime(format!(
                    "channel label must be a string, got {}",
                    other.type_name()
                )))
            }
        };
        channels.insert(index, label);
    }
    let monitor = match table.get::<Value>("monitor")? {
        Value::Nil => None,
        other => Some(lua_to_json(other)?),
    };
    Ok(ChannelLayoutDef {
        name,
        description,
        channels,
        monitor,
    })
}

fn channel_index(value: Value) -> mlua::Result<usize> {
    match value {
        Value::Integer(index) if index >= 0 => Ok(index as usize),
        Value::Number(index) if index >= 0.0 && index.fract() == 0.0 => Ok(index as usize),
        other => Err(mlua::Error::runtime(format!(
            "channel index must be a non-negative integer, got {}",
            other.type_name()
        ))),
    }
}

fn lua_to_json(value: Value) -> mlua::Result<serde_json::Value> {
    match value {
        Value::Nil => Ok(serde_json::Value::Null),
        Value::Boolean(flag) => Ok(serde_json::Value::Bool(flag)),
        Value::Integer(number) => Ok(serde_json::Value::Number(number.into())),
        Value::Number(number) => serde_json::Number::from_f64(number)
            .map(serde_json::Value::Number)
            .ok_or_else(|| mlua::Error::runtime("monitor number is not finite")),
        Value::String(text) => Ok(serde_json::Value::String(text.to_str()?.to_owned())),
        Value::Table(table) => table_to_json(table),
        other => Err(mlua::Error::runtime(format!(
            "monitor values must be JSON-compatible, got {}",
            other.type_name()
        ))),
    }
}

fn table_to_json(table: Table) -> mlua::Result<serde_json::Value> {
    let mut pairs = Vec::new();
    let mut array_len = 0usize;
    let mut is_array = true;
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair?;
        match key {
            Value::Integer(index) if index >= 1 => {
                let index = index as usize;
                array_len = array_len.max(index);
                pairs.push((JsonKey::Index(index), lua_to_json(value)?));
            }
            Value::String(text) => {
                is_array = false;
                pairs.push((
                    JsonKey::Name(text.to_str()?.to_owned()),
                    lua_to_json(value)?,
                ));
            }
            other => {
                return Err(mlua::Error::runtime(format!(
                    "monitor table keys must be strings or 1-based integers, got {}",
                    other.type_name()
                )))
            }
        }
    }
    if is_array && !pairs.is_empty() && pairs.len() == array_len {
        let mut items = vec![serde_json::Value::Null; array_len];
        for (key, value) in pairs {
            if let JsonKey::Index(index) = key {
                items[index - 1] = value;
            }
        }
        return Ok(serde_json::Value::Array(items));
    }
    let mut object = serde_json::Map::new();
    for (key, value) in pairs {
        match key {
            JsonKey::Name(name) => {
                object.insert(name, value);
            }
            JsonKey::Index(index) => {
                object.insert(index.to_string(), value);
            }
        }
    }
    Ok(serde_json::Value::Object(object))
}

enum JsonKey {
    Index(usize),
    Name(String),
}
