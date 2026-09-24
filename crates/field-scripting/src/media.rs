// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Shared media pool bindings.

use std::str::FromStr;
use std::time::{Duration, UNIX_EPOCH};

use mlua::{FromLua, Table, UserData, UserDataFields, UserDataMethods, Value};

use field_audio_model::{MediaId, MediaRef};

use crate::host::host_from_lua;

/// Handle to one entry in the shared media store.
#[derive(Clone, Debug)]
pub struct LuaMedia {
    /// Stable media id.
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
            with_media(lua, this.id, |media| Ok(media.url.clone()))
        });
        fields.add_field_method_get("path", |lua, this| {
            with_media(lua, this.id, |media| Ok(media.path.display().to_string()))
        });
        fields.add_field_method_get("basename", |lua, this| {
            with_media(lua, this.id, |media| Ok(media.basename.clone()))
        });
        fields.add_field_method_get("sample_rate", |lua, this| {
            with_media(lua, this.id, |media| Ok(media.sample_rate as i64))
        });
        fields.add_field_method_get("channels", |lua, this| {
            with_media(lua, this.id, |media| Ok(media.channel_count as i64))
        });
        fields.add_field_method_get("frames", |lua, this| {
            with_media(lua, this.id, |media| Ok(media.frame_count as i64))
        });
        fields.add_field_method_get("bit_depth", |lua, this| {
            with_media(lua, this.id, |media| {
                Ok(media.bits_per_sample.map(|b| b as i64))
            })
        });
        fields.add_field_method_get("size_bytes", |lua, this| {
            with_media(lua, this.id, |media| Ok(media.size_bytes as i64))
        });
        fields.add_field_method_get("modified", |lua, this| {
            with_media(lua, this.id, |media| Ok(format_modified(media.modified)))
        });
        fields.add_field_method_get("container_format", |lua, this| {
            with_media(lua, this.id, |media| Ok(media.container_format.clone()))
        });
        fields.add_field_method_get("codec", |lua, this| {
            with_media(lua, this.id, |media| Ok(media.codec.clone()))
        });
        fields.add_field_method_get("duration", |lua, this| {
            with_media(lua, this.id, |media| {
                Ok(media.frame_count as f64 / media.sample_rate as f64)
            })
        });
    }
}

/// Singleton handle to the process media pool.
#[derive(Clone, Copy, Debug)]
pub struct LuaMediaPool;

impl UserData for LuaMediaPool {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("add", |lua, _, path: String| {
            let host = host_from_lua(lua)?;
            let id = host.with_backend_mut(|b| b.add_media(std::path::Path::new(&path)))?;
            Ok(LuaMedia { id })
        });
        methods.add_method("remove", |lua, _, value: Value| {
            let media = LuaMedia::from_lua(value, lua)?;
            let host = host_from_lua(lua)?;
            host.with_backend_mut(|b| b.remove_media(media.id))?;
            Ok(())
        });
        methods.add_method("items", |lua, _, ()| {
            let host = host_from_lua(lua)?;
            let ids = host.with_backend(|b| b.list_media());
            let table = lua.create_table_with_capacity(ids.len(), 0)?;
            for (index, id) in ids.into_iter().enumerate() {
                table.set(index + 1, LuaMedia { id })?;
            }
            Ok(table)
        });
    }
}

fn format_modified(time: std::time::SystemTime) -> String {
    let dur = time.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
    format!("{}", dur.as_secs())
}

fn with_media<R>(
    lua: &mlua::Lua,
    id: MediaId,
    f: impl FnOnce(&MediaRef) -> mlua::Result<R>,
) -> mlua::Result<R> {
    let host = host_from_lua(lua)?;
    let media = host.with_backend(|b| b.get_media(id))?;
    f(&media)
}

/// Install `field.media`.
pub fn bind_media_module(lua: &mlua::Lua, field: &Table) -> mlua::Result<()> {
    let media = lua.create_table()?;
    media.set(
        "shared_pool",
        lua.create_function(|_, ()| Ok(LuaMediaPool))?,
    )?;
    field.set("media", media)?;
    Ok(())
}
