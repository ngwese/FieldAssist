// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! `field.include` — load Lua relative to the caller or config dir.

use std::path::{Path, PathBuf};

use mlua::{Lua, MultiValue, Value};

use crate::host::host_from_lua;
use crate::url::LuaUrl;

/// Resolve and execute an include path / URL.
pub fn field_include(lua: &Lua, spec: Value) -> mlua::Result<MultiValue> {
    let host = host_from_lua(lua)?;
    let path = resolve_include_path(&host, spec)?;
    let key = path
        .canonicalize()
        .unwrap_or_else(|_| path.clone())
        .to_string_lossy()
        .into_owned();
    {
        let inner = host.inner.borrow();
        if let Some(cached) = inner.include_cache.get(&key) {
            return match cached.clone() {
                Value::Nil => Ok(MultiValue::new()),
                other => {
                    let mut mv = MultiValue::new();
                    mv.push_back(other);
                    Ok(mv)
                }
            };
        }
    }

    let parent = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    host.inner.borrow_mut().include_stack.push(parent);

    let chunk = std::fs::read_to_string(&path)
        .map_err(|err| mlua::Error::runtime(format!("{}: {err}", path.display())))?;
    let name = format!("@{}", path.display());
    let result = lua
        .load(&chunk)
        .set_name(name)
        .eval::<MultiValue>()
        .or_else(|_: mlua::Error| -> mlua::Result<MultiValue> {
            lua.load(&chunk)
                .set_name(format!("@{}", path.display()))
                .exec()?;
            Ok(MultiValue::new())
        });

    host.inner.borrow_mut().include_stack.pop();

    let values = result?;
    let cached = values.iter().next().cloned().unwrap_or(Value::Nil);
    host.inner.borrow_mut().include_cache.insert(key, cached);
    Ok(values)
}

fn resolve_include_path(host: &crate::host::HostHandle, spec: Value) -> mlua::Result<PathBuf> {
    let raw = match spec {
        Value::String(s) => s.to_str()?.to_owned(),
        Value::UserData(ud) => {
            let url = ud.borrow::<LuaUrl>()?;
            return url.to_path(None);
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "field.include expects a path string or url, got {}",
                other.type_name()
            )))
        }
    };

    let candidate = PathBuf::from(&raw);
    if candidate.is_absolute() && candidate.is_file() {
        return Ok(candidate);
    }

    let inner = host.inner.borrow();
    if let Some(caller) = inner.include_stack.last() {
        let joined = caller.join(&raw);
        if joined.is_file() {
            return Ok(joined);
        }
    }
    if let Some(config) = inner.profile.config_dir.as_ref() {
        let joined = config.join(&raw);
        if joined.is_file() {
            return Ok(joined);
        }
    }
    if candidate.is_file() {
        return Ok(candidate);
    }
    Err(mlua::Error::runtime(format!(
        "field.include could not find `{raw}`"
    )))
}

#[cfg(test)]
mod tests {
    use crate::host::{HostProfile, ScriptHost};

    #[test]
    fn include_relative_and_cache() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("shared.lua"), "return { answer = 42 }").unwrap();

        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: Some(dir.path().to_path_buf()),
        })
        .unwrap();
        let out = host.eval(
            r#"
            local shared = field.include("shared.lua")
            local again = field.include("shared.lua")
            return shared.answer, again.answer
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("42\t42"));
    }

    #[test]
    fn nested_relative_include() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("lib");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("inner.lua"), "return 7").unwrap();
        std::fs::write(
            dir.path().join("outer.lua"),
            r#"return field.include("lib/inner.lua")"#,
        )
        .unwrap();

        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: Some(dir.path().to_path_buf()),
        })
        .unwrap();
        let out = host.eval(r#"return field.include("outer.lua")"#);
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("7"));
    }
}
