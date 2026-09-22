// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Channel layout registration for `field.layouts`.

use std::collections::BTreeMap;

use mlua::{Table, Value};

use crate::host::{host_from_lua, HostHandle};
use field_session::DocumentId;

#[derive(Clone, Debug)]
/// Registered channel layout from Lua.
pub struct ChannelLayoutDef {
    /// Layout name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Channel index to label map.
    pub channels: BTreeMap<usize, String>,
    /// Optional monitor JSON (may include `"chain"`).
    pub monitor: Option<serde_json::Value>,
}

impl ChannelLayoutDef {
    /// Default monitor chain id from the layout monitor table.
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

/// Register a layout definition on the host.
pub fn bind_layouts(lua: &mlua::Lua, field: &Table) -> mlua::Result<()> {
    let layouts = lua.create_table()?;
    layouts.set(
        "define",
        lua.create_function(|lua, spec: Table| {
            let layout = layout_from_lua(spec)?;
            host_from_lua(lua)?.define_layout(layout);
            Ok(())
        })?,
    )?;
    layouts.set(
        "shared_registry",
        lua.create_function(|lua, ()| {
            let host = host_from_lua(lua)?;
            let names = host.layout_names();
            let table = lua.create_table_with_capacity(names.len(), 0)?;
            for (index, name) in names.into_iter().enumerate() {
                table.set(index + 1, name)?;
            }
            Ok(table)
        })?,
    )?;
    field.set("layouts", layouts)?;
    Ok(())
}

impl HostHandle {
    pub(crate) fn define_layout(&self, layout: ChannelLayoutDef) {
        let mut inner = self.inner.borrow_mut();
        if let Some(existing) = inner
            .layouts
            .iter_mut()
            .find(|defined| defined.name == layout.name)
        {
            *existing = layout;
        } else {
            inner.layouts.push(layout);
        }
    }

    #[allow(dead_code)]
    pub(crate) fn layout(&self, name: &str) -> Option<ChannelLayoutDef> {
        self.inner
            .borrow()
            .layouts
            .iter()
            .find(|layout| layout.name == name)
            .cloned()
    }

    pub(crate) fn layout_names(&self) -> Vec<String> {
        self.inner
            .borrow()
            .layouts
            .iter()
            .map(|layout| layout.name.clone())
            .collect()
    }

    #[allow(dead_code)]
    pub(crate) fn choose_layout(&self, id: DocumentId, name: Option<&str>) -> mlua::Result<()> {
        let (labels, default_chain) = match name {
            Some(name) => {
                let layout = self.layout(name);
                let labels = layout
                    .as_ref()
                    .map(|layout| layout.channels.clone())
                    .unwrap_or_default();
                let chain = layout
                    .as_ref()
                    .and_then(|layout| layout.monitor_chain_id().map(str::to_string));
                (labels, chain)
            }
            None => (BTreeMap::new(), None),
        };
        self.with_world_mut(|world| {
            let doc = world
                .docs
                .get_mut(&id)
                .ok_or_else(|| mlua::Error::runtime("composition is not open"))?;
            let mut composition = doc.composition.write().unwrap();
            composition.apply_channel_layout(name.map(str::to_string), labels);
            if composition.monitor_chain().is_none() {
                if let Some(chain) = default_chain {
                    composition.set_monitor_chain(Some(chain));
                }
            }
            Ok(())
        })
    }
}
