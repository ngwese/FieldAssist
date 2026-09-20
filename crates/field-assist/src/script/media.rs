// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Lua userdata for shared media-pool entries.

use std::str::FromStr;

use mlua::{FromLua, UserData, UserDataFields, Value};

use crate::media_pool::MediaPoolRow;
use crate::model::composition::MediaId;

use super::host::host_from_lua;

/// Live handle to one entry in the window shared media pool.
#[derive(Clone, Debug)]
pub struct LuaMedia {
    pub id: MediaId,
}

impl FromLua for LuaMedia {
    fn from_lua(value: Value, _lua: &mlua::Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(ud) => ud.borrow::<Self>().map(|m| m.clone()),
            Value::String(s) => {
                let text = s.to_str()?.to_owned();
                let id = MediaId::from_str(&text).map_err(mlua::Error::runtime)?;
                Ok(Self { id })
            }
            other => Err(mlua::Error::runtime(format!(
                "expected media userdata or media id string, got {}",
                other.type_name()
            ))),
        }
    }
}

impl UserData for LuaMedia {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("id", |_, this| Ok(this.id.to_string()));
        fields.add_field_method_get("url", |lua, this| {
            with_row(lua, this.id, |row| Ok(row.url.clone()))
        });
        fields.add_field_method_get("path", |lua, this| {
            with_row(lua, this.id, |row| Ok(row.path.display().to_string()))
        });
        fields.add_field_method_get("basename", |lua, this| {
            with_row(lua, this.id, |row| Ok(row.basename.clone()))
        });
        fields.add_field_method_get("sample_rate", |lua, this| {
            with_row(lua, this.id, |row| Ok(row.sample_rate as i64))
        });
        fields.add_field_method_get("channels", |lua, this| {
            with_row(lua, this.id, |row| Ok(row.channel_count as i64))
        });
        fields.add_field_method_get("frames", |lua, this| {
            with_row(lua, this.id, |row| Ok(row.frame_count as i64))
        });
        fields.add_field_method_get("bit_depth", |lua, this| {
            with_row(lua, this.id, |row| {
                Ok(row.bits_per_sample.map(|bits| bits as i64))
            })
        });
        fields.add_field_method_get("size_bytes", |lua, this| {
            with_row(lua, this.id, |row| Ok(row.size_bytes as i64))
        });
        fields.add_field_method_get("modified", |lua, this| {
            with_row(lua, this.id, |row| Ok(row.modified_text.clone()))
        });
        fields.add_field_method_get("container_format", |lua, this| {
            with_row(lua, this.id, |row| Ok(row.container_format.clone()))
        });
        fields.add_field_method_get("codec", |lua, this| {
            with_row(lua, this.id, |row| Ok(row.codec.clone()))
        });
        fields.add_field_method_get("duration", |lua, this| {
            with_row(lua, this.id, |row| Ok(row.duration_secs()))
        });
    }
}

fn with_row<R>(
    lua: &mlua::Lua,
    id: MediaId,
    f: impl FnOnce(&MediaPoolRow) -> mlua::Result<R>,
) -> mlua::Result<R> {
    let host = host_from_lua(lua)?;
    let row = host
        .media_row(id)?
        .ok_or_else(|| mlua::Error::runtime(format!("media no longer exists: {id}")))?;
    f(&row)
}
