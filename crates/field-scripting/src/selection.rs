// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Region collections and channel parsing for Lua.

use mlua::{FromLua, Lua, MetaMethod, UserData, UserDataFields, UserDataMethods, Value};

use field_audio_model::{ChannelScope, SELECTION_COLLECTION};
use field_session::DocumentId;

use crate::composition::with_document;
use crate::region::LuaRegion;

/// Named or selection region collection handle.
#[derive(Clone, Debug)]
pub struct LuaCollection {
    /// Owning document.
    pub doc: DocumentId,
    /// Collection name (`"selection"` or a user name).
    pub name: String,
}

impl FromLua for LuaCollection {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(data) => data.borrow::<Self>().map(|this| this.clone()),
            _ => Err(mlua::Error::runtime("expected a collection")),
        }
    }
}

impl UserData for LuaCollection {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |_, this| Ok(this.name.clone()));
        fields.add_field_method_get("regions", |lua, this| {
            with_document(lua, this.doc, |doc| Ok(collection_regions(doc, this)))
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::Len, |lua, this, ()| {
            with_document(lua, this.doc, |doc| {
                Ok(collection_regions(doc, this).len() as i64)
            })
        });
    }
}

fn collection_regions(doc: &crate::world::OpenDocument, this: &LuaCollection) -> Vec<LuaRegion> {
    if this.name == SELECTION_COLLECTION {
        return doc
            .selection
            .regions
            .iter()
            .map(|region| LuaRegion {
                doc: this.doc,
                collection: SELECTION_COLLECTION.into(),
                id: region.id,
            })
            .collect();
    }
    doc.collections
        .get(&this.name)
        .map(|col| {
            col.regions
                .iter()
                .map(|region| LuaRegion {
                    doc: this.doc,
                    collection: this.name.clone(),
                    id: region.id,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse a Lua channels argument (`nil`, `"all"`, or a 1-based index list).
pub fn channels_from_lua(_lua: &Lua, value: Value) -> mlua::Result<ChannelScope> {
    match value {
        Value::Nil => Ok(ChannelScope::all()),
        Value::String(s) => {
            let text = s.to_str()?.to_string();
            if text == "all" {
                Ok(ChannelScope::all())
            } else {
                Err(mlua::Error::runtime(format!(
                    "channels must be \"all\" or an array, got {text:?}"
                )))
            }
        }
        Value::Table(table) => {
            let mut channels = Vec::new();
            for pair in table.sequence_values::<i64>() {
                let index = pair?;
                if index < 0 {
                    return Err(mlua::Error::runtime("channel indices must be non-negative"));
                }
                channels.push(index as usize);
            }
            if channels.is_empty() {
                Ok(ChannelScope::all())
            } else {
                Ok(ChannelScope::Channels(channels))
            }
        }
        other => Err(mlua::Error::runtime(format!(
            "channels must be \"all\" or an array, got {}",
            other.type_name()
        ))),
    }
}

/// Encode channel scope for Lua.
pub fn channels_to_lua(lua: &Lua, channels: &ChannelScope) -> mlua::Result<Value> {
    match channels {
        ChannelScope::AllChannels => Ok(Value::String(lua.create_string("all")?)),
        ChannelScope::Channels(list) => {
            let table = lua.create_table()?;
            for (i, ch) in list.iter().enumerate() {
                table.set(i + 1, *ch as i64)?;
            }
            Ok(Value::Table(table))
        }
    }
}

/// Optional integer from Lua.
pub fn optional_i64(value: Value) -> mlua::Result<Option<i64>> {
    match value {
        Value::Nil => Ok(None),
        Value::Integer(n) => Ok(Some(n)),
        Value::Number(n) => Ok(Some(n as i64)),
        other => Err(mlua::Error::runtime(format!(
            "expected integer, got {}",
            other.type_name()
        ))),
    }
}

/// Collection name (`nil` → selection).
pub fn collection_name_from_lua(value: Value) -> mlua::Result<String> {
    match value {
        Value::Nil => Ok(SELECTION_COLLECTION.into()),
        Value::String(s) => Ok(s.to_str()?.to_string()),
        other => Err(mlua::Error::runtime(format!(
            "collection name must be a string, got {}",
            other.type_name()
        ))),
    }
}
