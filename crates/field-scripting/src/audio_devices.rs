// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Output device listing for scripts.

use mlua::{Table, Value};

/// Install `field.audio_devices`.
pub fn bind_audio_devices(lua: &mlua::Lua, field: &Table) -> mlua::Result<()> {
    let devices = lua.create_table()?;
    devices.set(
        "list",
        lua.create_function(|lua, ()| {
            let listed = field_audio_playback::list_output_devices()
                .map_err(|err| mlua::Error::runtime(err.to_string()))?;
            let table = lua.create_table_with_capacity(listed.len(), 0)?;
            for (index, info) in listed.into_iter().enumerate() {
                let row = lua.create_table()?;
                row.set("index", info.index as i64)?;
                row.set("name", info.name)?;
                row.set("is_default", info.is_default)?;
                match info.sample_rate {
                    Some(rate) => row.set("sample_rate", rate as i64)?,
                    None => row.set("sample_rate", Value::Nil)?,
                }
                match info.channels {
                    Some(n) => row.set("channels", i64::from(n))?,
                    None => row.set("channels", Value::Nil)?,
                }
                match info.sample_format {
                    Some(fmt) => row.set("sample_format", fmt)?,
                    None => row.set("sample_format", Value::Nil)?,
                }
                table.set(index + 1, row)?;
            }
            Ok(table)
        })?,
    )?;
    field.set("audio_devices", devices)?;
    Ok(())
}
