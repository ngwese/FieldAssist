// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use mlua::{Table, UserData, Value};

/// Empty prototype table with `__index = self` so instances inherit methods.
pub fn create_prototype_table(lua: &mlua::Lua) -> mlua::Result<Table> {
    let proto = lua.create_table()?;
    proto.set("__index", proto.clone())?;
    Ok(proto)
}

/// New instance whose metatable is `prototype`. Calls `:init()` when defined.
pub fn new_instance(lua: &mlua::Lua, prototype: Table) -> mlua::Result<Table> {
    let instance = lua.create_table()?;
    instance.set_metatable(Some(prototype))?;
    match instance.get::<Value>("init")? {
        Value::Nil => {}
        Value::Function(init) => init.call::<()>(instance.clone())?,
        other => {
            return Err(mlua::Error::runtime(format!(
                "init must be a function, got {}",
                other.type_name()
            )))
        }
    }
    Ok(instance)
}

/// Read `__base_properties` UserData from a prototype or instance table.
pub fn base_properties<T>(table: &Table) -> mlua::Result<T>
where
    T: UserData + Clone + 'static,
{
    match table.get::<Value>("__base_properties")? {
        Value::UserData(data) => Ok(data.borrow::<T>()?.clone()),
        other => Err(mlua::Error::runtime(format!(
            "__base_properties must be userdata, got {}",
            other.type_name()
        ))),
    }
}
