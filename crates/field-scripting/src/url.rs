// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! `field.url` — location userdata over the `url` crate and field-core file URLs.

use std::path::{Path, PathBuf};

use field_core::{encode_file_url, resolve_file_url};
use mlua::{FromLua, Lua, Table, UserData, UserDataFields, UserDataMethods, Value};
use url::Url;

/// Lua location handle.
#[derive(Clone, Debug)]
pub struct LuaUrl {
    /// Stored serialized form (file://, memory://, relative, or absolute path).
    raw: String,
}

impl LuaUrl {
    /// Parse a native path, `file://` URL, relative URL, or `memory://` URL.
    pub fn parse(input: &str) -> mlua::Result<Self> {
        let input = input.trim();
        if input.is_empty() {
            return Err(mlua::Error::runtime("empty URL"));
        }
        Ok(Self {
            raw: input.to_owned(),
        })
    }

    /// From a filesystem path (absolute `file://` when possible).
    pub fn from_path(path: &Path) -> Self {
        Self {
            raw: encode_file_url(path, None),
        }
    }

    /// Resolve to a filesystem path when applicable.
    pub fn to_path(&self, base: Option<&Path>) -> mlua::Result<PathBuf> {
        resolve_file_url(&self.raw, base).map_err(|err| mlua::Error::runtime(err.to_string()))
    }

    fn as_url(&self) -> Option<Url> {
        if self.raw.starts_with("memory://") {
            return None;
        }
        if self.raw.starts_with("file://") {
            return Url::parse(&self.raw).ok();
        }
        Url::parse(&self.raw).ok()
    }

    fn native_path(&self) -> Option<PathBuf> {
        self.to_path(None).ok().filter(|p| {
            let s = p.to_string_lossy();
            !s.starts_with("memory://")
        })
    }
}

impl FromLua for LuaUrl {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::UserData(ud) => ud.borrow::<Self>().map(|u| u.clone()),
            Value::String(s) => Self::parse(s.to_str()?.as_ref()),
            other => Err(mlua::Error::runtime(format!(
                "expected url userdata or string, got {}",
                other.type_name()
            ))),
        }
    }
}

impl UserData for LuaUrl {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("scheme", |_, this| {
            if this.raw.starts_with("memory://") {
                return Ok("memory".to_string());
            }
            if this.raw.starts_with("file://") || Path::new(&this.raw).is_absolute() {
                return Ok("file".to_string());
            }
            Ok(this
                .as_url()
                .map(|u| u.scheme().to_string())
                .unwrap_or_else(|| "file".into()))
        });
        fields.add_field_method_get("host", |_, this| {
            Ok(this.as_url().and_then(|u| u.host_str().map(str::to_string)))
        });
        fields.add_field_method_get("path", |_, this| {
            if let Some(p) = this.native_path() {
                return Ok(p.to_string_lossy().replace('\\', "/"));
            }
            Ok(this
                .as_url()
                .map(|u| u.path().to_string())
                .unwrap_or_else(|| this.raw.clone()))
        });
        fields.add_field_method_get("query", |_, this| {
            Ok(this.as_url().and_then(|u| u.query().map(str::to_string)))
        });
        fields.add_field_method_get("native", |_, this| {
            Ok(this
                .native_path()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| this.raw.clone()))
        });
        fields.add_field_method_get("normalized", |_, this| {
            if this.raw.starts_with("memory://") || this.raw.starts_with("file://") {
                return Ok(this.raw.clone());
            }
            if let Ok(path) = this.to_path(None) {
                return Ok(encode_file_url(&path, None));
            }
            Ok(this.raw.clone())
        });
        fields.add_field_method_get("parent", |_, this| {
            let path = this
                .native_path()
                .ok_or_else(|| mlua::Error::runtime("parent requires a filesystem URL"))?;
            let parent = path
                .parent()
                .ok_or_else(|| mlua::Error::runtime("URL has no parent"))?;
            Ok(LuaUrl::from_path(parent))
        });
        fields.add_field_method_get("basename", |_, this| {
            let path = this
                .native_path()
                .ok_or_else(|| mlua::Error::runtime("basename requires a filesystem URL"))?;
            Ok(path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default())
        });
        fields.add_field_method_get("stem", |_, this| {
            let path = this
                .native_path()
                .ok_or_else(|| mlua::Error::runtime("stem requires a filesystem URL"))?;
            Ok(path
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default())
        });
        fields.add_field_method_get("extension", |_, this| {
            let path = this
                .native_path()
                .ok_or_else(|| mlua::Error::runtime("extension requires a filesystem URL"))?;
            Ok(path
                .extension()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("join", |_, this, parts: mlua::MultiValue| {
            let mut path = this
                .native_path()
                .ok_or_else(|| mlua::Error::runtime("join requires a filesystem URL"))?;
            for part in parts {
                let seg = match part {
                    Value::String(s) => s.to_str()?.to_owned(),
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "join segment must be a string, got {}",
                            other.type_name()
                        )))
                    }
                };
                path.push(seg.trim_start_matches(['/', '\\']));
            }
            Ok(LuaUrl::from_path(&path))
        });
        methods.add_method("with_extension", |_, this, ext: String| {
            let path = this
                .native_path()
                .ok_or_else(|| mlua::Error::runtime("with_extension requires a filesystem URL"))?;
            let ext = ext.trim_start_matches('.');
            Ok(LuaUrl::from_path(&path.with_extension(ext)))
        });
        methods.add_method("relative_to", |lua, this, base: Value| {
            let base = LuaUrl::from_lua(base, lua)?;
            let path = this.to_path(None)?;
            let base_path = base.to_path(None)?;
            let rel = path
                .strip_prefix(&base_path)
                .map_err(|_| mlua::Error::runtime("path is not under base"))?;
            Ok(rel.to_string_lossy().replace('\\', "/"))
        });
        methods.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| {
            Ok(this.raw.clone())
        });
    }
}

/// Bind `field.url` module.
pub fn bind_url(lua: &Lua, field: &Table) -> mlua::Result<()> {
    let module = lua.create_table()?;
    module.set(
        "parse",
        lua.create_function(|_, input: String| LuaUrl::parse(&input))?,
    )?;
    module.set(
        "from_path",
        lua.create_function(|_, path: String| Ok(LuaUrl::from_path(Path::new(&path))))?,
    )?;
    module.set(
        "from_file_path",
        lua.create_function(|_, path: String| Ok(LuaUrl::from_path(Path::new(&path))))?,
    )?;
    field.set("url", module)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{HostProfile, ScriptHost};

    #[test]
    fn from_path_and_components() {
        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: None,
        })
        .unwrap();
        let out = host.eval(
            r#"
            local u = field.url.from_path("/tmp/takes/a.wav")
            return u.basename, u.stem, u.extension, u.scheme
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("a.wav\ta\twav\tfile"));
    }

    #[test]
    fn join_and_with_extension() {
        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: None,
        })
        .unwrap();
        let out = host.eval(
            r#"
            local u = field.url.from_path("/tmp/root")
            local j = u:join("nested", "take.wav")
            local f = j:with_extension("flac")
            return j.basename, f.extension
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("take.wav\tflac"));
    }

    #[test]
    fn relative_to_base() {
        let u = LuaUrl::from_path(Path::new("/sessions/takes/a.wav"));
        let base = LuaUrl::from_path(Path::new("/sessions"));
        let path = u.to_path(None).unwrap();
        let base_path = base.to_path(None).unwrap();
        let rel = path.strip_prefix(&base_path).unwrap();
        assert_eq!(rel.to_string_lossy().replace('\\', "/"), "takes/a.wav");
    }
}
