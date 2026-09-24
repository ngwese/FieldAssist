// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Channel layout registration for `field.layouts`.

use std::collections::BTreeMap;

use mlua::{FromLua, Table, UserData, UserDataFields, UserDataMethods, Value};

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

/// Handle to one registered channel layout (looked up live by name).
#[derive(Clone, Debug)]
pub struct LuaLayout {
    name: String,
}

impl FromLua for LuaLayout {
    fn from_lua(value: Value, _lua: &mlua::Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(ud) => ud.borrow::<Self>().map(|layout| layout.clone()),
            Value::String(s) => Ok(Self {
                name: s.to_str()?.to_owned(),
            }),
            other => Err(mlua::Error::runtime(format!(
                "expected layout userdata or layout name string, got {}",
                other.type_name()
            ))),
        }
    }
}

impl UserData for LuaLayout {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |_, this| Ok(this.name.clone()));
        fields.add_field_method_get("description", |lua, this| {
            with_layout(lua, &this.name, |layout| Ok(layout.description.clone()))
        });
        fields.add_field_method_get("channels", |lua, this| {
            with_layout(lua, &this.name, |layout| {
                let table = lua.create_table_with_capacity(0, layout.channels.len())?;
                for (index, label) in &layout.channels {
                    table.set(*index, label.clone())?;
                }
                Ok(table)
            })
        });
        fields.add_field_method_get("monitor", |lua, this| {
            with_layout(lua, &this.name, |layout| match &layout.monitor {
                Some(value) => json_to_lua(lua, value),
                None => Ok(Value::Nil),
            })
        });
    }
}

/// Process-wide channel layout registry.
#[derive(Clone, Copy, Debug)]
pub struct LuaLayoutRegistry;

impl UserData for LuaLayoutRegistry {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("define", |lua, _, spec: Table| {
            let layout = layout_from_lua(spec)?;
            host_from_lua(lua)?.define_layout(layout);
            Ok(())
        });
        methods.add_method("remove", |lua, _, value: Value| {
            let layout = LuaLayout::from_lua(value, lua)?;
            host_from_lua(lua)?.remove_layout(&layout.name);
            Ok(())
        });
        methods.add_method("items", |lua, _, ()| {
            let host = host_from_lua(lua)?;
            let names = host.layout_names();
            let table = lua.create_table_with_capacity(names.len(), 0)?;
            for (index, name) in names.into_iter().enumerate() {
                table.set(index + 1, LuaLayout { name })?;
            }
            Ok(table)
        });
    }
}

fn with_layout<R>(
    lua: &mlua::Lua,
    name: &str,
    f: impl FnOnce(&ChannelLayoutDef) -> mlua::Result<R>,
) -> mlua::Result<R> {
    let host = host_from_lua(lua)?;
    let layout = host
        .layout(name)
        .ok_or_else(|| mlua::Error::runtime(format!("unknown channel layout `{name}`")))?;
    f(&layout)
}

/// Parse a layout definition table from Lua.
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

fn json_to_lua(lua: &mlua::Lua, value: &serde_json::Value) -> mlua::Result<Value> {
    match value {
        serde_json::Value::Null => Ok(Value::Nil),
        serde_json::Value::Bool(flag) => Ok(Value::Boolean(*flag)),
        serde_json::Value::Number(number) => {
            if let Some(i) = number.as_i64() {
                Ok(Value::Integer(i))
            } else if let Some(u) = number.as_u64() {
                if u <= i64::MAX as u64 {
                    Ok(Value::Integer(u as i64))
                } else {
                    Ok(Value::Number(u as f64))
                }
            } else if let Some(f) = number.as_f64() {
                Ok(Value::Number(f))
            } else {
                Err(mlua::Error::runtime("monitor number is not representable"))
            }
        }
        serde_json::Value::String(text) => Ok(Value::String(lua.create_string(text)?)),
        serde_json::Value::Array(items) => {
            let table = lua.create_table_with_capacity(items.len(), 0)?;
            for (index, item) in items.iter().enumerate() {
                table.set(index + 1, json_to_lua(lua, item)?)?;
            }
            Ok(Value::Table(table))
        }
        serde_json::Value::Object(map) => {
            let table = lua.create_table_with_capacity(0, map.len())?;
            for (key, item) in map {
                table.set(key.as_str(), json_to_lua(lua, item)?)?;
            }
            Ok(Value::Table(table))
        }
    }
}

enum JsonKey {
    Index(usize),
    Name(String),
}

/// Register layout module functions on the host.
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
        lua.create_function(|_, ()| Ok(LuaLayoutRegistry))?,
    )?;
    field.set("layouts", layouts)?;
    Ok(())
}

impl HostHandle {
    /// Register or replace a channel layout definition.
    pub(crate) fn define_layout(&self, layout: ChannelLayoutDef) {
        let mut inner = self.inner.borrow_mut();
        if let Some(existing) = inner.layouts.iter_mut().find(|l| l.name == layout.name) {
            *existing = layout;
        } else {
            inner.layouts.push(layout);
        }
    }

    /// Remove a registered layout by name. No-op if missing.
    pub(crate) fn remove_layout(&self, name: &str) {
        let mut inner = self.inner.borrow_mut();
        inner.layouts.retain(|layout| layout.name != name);
    }

    /// Look up a registered layout by name.
    pub(crate) fn layout(&self, name: &str) -> Option<ChannelLayoutDef> {
        self.inner
            .borrow()
            .layouts
            .iter()
            .find(|l| l.name == name)
            .cloned()
    }

    /// Names of all registered layouts.
    pub(crate) fn layout_names(&self) -> Vec<String> {
        self.inner
            .borrow()
            .layouts
            .iter()
            .map(|l| l.name.clone())
            .collect()
    }

    /// Apply a named layout to a document (`None` clears the chosen layout).
    pub(crate) fn choose_layout(&self, id: DocumentId, name: Option<&str>) -> mlua::Result<()> {
        let name_owned = name.map(str::to_string);
        let (labels, default_chain) = if let Some(ref name_str) = name_owned {
            let layout = self.layout(name_str).ok_or_else(|| {
                mlua::Error::runtime(format!("unknown channel layout `{name_str}`"))
            })?;
            let chain = layout.monitor_chain_id().map(str::to_string);
            (layout.channels, chain)
        } else {
            (BTreeMap::new(), None)
        };
        self.with_backend_mut(|backend| {
            backend.with_open_document_mut(id, &mut |doc| {
                let mut composition = doc.composition.write().unwrap();
                composition.choose_channel_layout(name_owned.clone(), labels.clone());
                composition.set_monitor_chain(default_chain.clone());
                Ok(())
            })
        })
    }
}
