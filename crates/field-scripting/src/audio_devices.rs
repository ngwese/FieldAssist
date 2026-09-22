// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Output device listing for scripts.

use mlua::Table;

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
                table.set(index + 1, row)?;
            }
            Ok(table)
        })?,
    )?;
    field.set("audio_devices", devices)?;
    Ok(())
}
