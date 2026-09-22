// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Restrict `package.path` / `package.cpath` and C-module loading by default.

use std::path::Path;

use mlua::{Function, Lua, Table, Value};

/// Snapshots used to restore system paths and native loaders on opt-in.
pub struct PackagePolicy {
    system_path: String,
    system_cpath: String,
    native_loadlib: Value,
    native_searcher_3: Value,
    native_searcher_4: Option<Value>,
    system_package_paths_enabled: bool,
    native_modules_enabled: bool,
}

impl PackagePolicy {
    /// Whether [`Self::enable_system_package_paths`] has already run.
    pub fn system_package_paths_enabled(&self) -> bool {
        self.system_package_paths_enabled
    }

    /// Whether [`Self::enable_native_modules`] has already run.
    pub fn native_modules_enabled(&self) -> bool {
        self.native_modules_enabled
    }
}

/// Install locked-down package paths and stub C loaders.
///
/// Default `package.path` / `package.cpath` are the FieldAssist config directory
/// (when provided) plus cwd-relative templates. System-wide Lua templates from
/// the bundled runtime (and `LUA_PATH*` / `LUA_CPATH*` foreign roots) are saved
/// for [`PackagePolicy::enable_system_package_paths`].
pub fn install_package_policy(lua: &Lua, config_dir: Option<&Path>) -> mlua::Result<PackagePolicy> {
    let package: Table = lua.globals().get("package")?;
    let initial_path: String = package.get("path")?;
    let initial_cpath: String = package.get("cpath")?;

    let system_path = system_templates_only(&initial_path, config_dir);
    let system_cpath = system_templates_only(&initial_cpath, config_dir);

    package.set("path", local_lua_path(config_dir))?;
    package.set("cpath", local_c_path(config_dir))?;

    let native_loadlib: Value = package.get("loadlib")?;
    let searchers: Table = package.get("searchers")?;
    let native_searcher_3: Value = searchers.raw_get(3)?;
    let native_searcher_4: Option<Value> = if searchers.raw_len() >= 4 {
        Some(searchers.raw_get(4)?)
    } else {
        None
    };

    stub_c_modules(lua, &package, &searchers)?;

    Ok(PackagePolicy {
        system_path,
        system_cpath,
        native_loadlib,
        native_searcher_3,
        native_searcher_4,
        system_package_paths_enabled: false,
        native_modules_enabled: false,
    })
}

impl PackagePolicy {
    /// Append saved system-wide `package.path` / `package.cpath` templates.
    ///
    /// Idempotent. Does not enable C-module loading.
    pub fn enable_system_package_paths(&mut self, lua: &Lua) -> mlua::Result<()> {
        if self.system_package_paths_enabled {
            return Ok(());
        }
        let package: Table = lua.globals().get("package")?;
        append_templates(&package, "path", &self.system_path)?;
        append_templates(&package, "cpath", &self.system_cpath)?;
        self.system_package_paths_enabled = true;
        Ok(())
    }

    /// Restore C searchers and `package.loadlib`.
    ///
    /// Idempotent. Does not add system package paths.
    pub fn enable_native_modules(&mut self, lua: &Lua) -> mlua::Result<()> {
        if self.native_modules_enabled {
            return Ok(());
        }
        let package: Table = lua.globals().get("package")?;
        package.set("loadlib", self.native_loadlib.clone())?;
        let searchers: Table = package.get("searchers")?;
        searchers.raw_set(3, self.native_searcher_3.clone())?;
        if let Some(searcher_4) = self.native_searcher_4.clone() {
            if searchers.raw_len() >= 4 {
                searchers.raw_set(4, searcher_4)?;
            } else {
                searchers.raw_insert(4, searcher_4)?;
            }
        }
        self.native_modules_enabled = true;
        Ok(())
    }
}

fn stub_c_modules(lua: &Lua, package: &Table, searchers: &Table) -> mlua::Result<()> {
    let disabled_loadlib: Function = lua.create_function(|_, ()| -> mlua::Result<()> {
        Err(mlua::Error::runtime(
            "package.loadlib is disabled; call field.scripting.enable_native_modules() first",
        ))
    })?;
    package.set("loadlib", disabled_loadlib)?;

    let stub: Function = lua.create_function(|_, ()| {
        Ok("\n\tcan't load C modules until field.scripting.enable_native_modules()")
    })?;
    searchers.raw_set(3, stub.clone())?;
    if searchers.raw_len() >= 4 {
        searchers.raw_set(4, stub)?;
    }
    Ok(())
}

fn append_templates(package: &Table, key: &str, extra: &str) -> mlua::Result<()> {
    if extra.is_empty() {
        return Ok(());
    }
    let current: String = package.get(key)?;
    if current.is_empty() {
        package.set(key, extra)?;
    } else {
        package.set(key, format!("{current};{extra}"))?;
    }
    Ok(())
}

fn system_templates_only(path: &str, config_dir: Option<&Path>) -> String {
    path.split(';')
        .filter(|t| !t.is_empty())
        .filter(|template| !is_default_local_template(template, config_dir))
        .map(str::to_owned)
        .collect::<Vec<_>>()
        .join(";")
}

fn is_default_local_template(template: &str, config_dir: Option<&Path>) -> bool {
    if template.starts_with("./") || template.starts_with(".\\") {
        return true;
    }
    let Some(config) = config_dir else {
        return false;
    };
    let config = config.to_string_lossy();
    let config = config.trim_end_matches(['/', '\\']);
    if config.is_empty() {
        return false;
    }
    template.starts_with(config)
        && (template.as_bytes().get(config.len()) == Some(&b'/')
            || template.as_bytes().get(config.len()) == Some(&b'\\')
            || template.len() == config.len())
}

fn local_lua_path(config_dir: Option<&Path>) -> String {
    let mut parts = Vec::new();
    if let Some(config) = config_dir {
        let base = config.display().to_string();
        parts.push(format!("{base}/?.lua"));
        parts.push(format!("{base}/?/init.lua"));
    }
    parts.push("./?.lua".to_owned());
    parts.push("./?/init.lua".to_owned());
    parts.join(";")
}

fn local_c_path(config_dir: Option<&Path>) -> String {
    let mut parts = Vec::new();
    #[cfg(target_os = "windows")]
    let ext = "dll";
    #[cfg(not(target_os = "windows"))]
    let ext = "so";
    if let Some(config) = config_dir {
        let base = config.display().to_string();
        parts.push(format!("{base}/?.{ext}"));
    }
    parts.push(format!("./?.{ext}"));
    parts.join(";")
}

#[cfg(test)]
mod tests {
    use super::{install_package_policy, local_lua_path};
    use mlua::Lua;
    use std::path::PathBuf;

    #[test]
    fn default_paths_are_config_and_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let lua = unsafe { Lua::unsafe_new() };
        let policy = install_package_policy(&lua, Some(dir.path())).unwrap();
        assert!(!policy.system_package_paths_enabled());
        assert!(!policy.native_modules_enabled());

        let path: String = lua
            .globals()
            .get::<mlua::Table>("package")
            .unwrap()
            .get("path")
            .unwrap();
        let config = dir.path().display().to_string();
        assert!(path.contains(&format!("{config}/?.lua")), "{path}");
        assert!(path.contains("./?.lua"), "{path}");
        assert!(!path.contains("/usr/local"), "{path}");
    }

    #[test]
    fn enable_system_paths_is_idempotent() {
        let lua = unsafe { Lua::unsafe_new() };
        let mut policy = install_package_policy(&lua, None).unwrap();
        let before: String = lua
            .globals()
            .get::<mlua::Table>("package")
            .unwrap()
            .get("path")
            .unwrap();
        policy.enable_system_package_paths(&lua).unwrap();
        let once: String = lua
            .globals()
            .get::<mlua::Table>("package")
            .unwrap()
            .get("path")
            .unwrap();
        policy.enable_system_package_paths(&lua).unwrap();
        let twice: String = lua
            .globals()
            .get::<mlua::Table>("package")
            .unwrap()
            .get("path")
            .unwrap();
        assert_eq!(once, twice);
        if !policy.system_path.is_empty() {
            assert!(once.len() > before.len(), "once={once} before={before}");
            assert!(once.contains("/usr/local") || once.contains("!.\\") || once.contains("!\\"));
        }
    }

    #[test]
    fn native_modules_stubbed_until_enabled() {
        let lua = unsafe { Lua::unsafe_new() };
        let mut policy = install_package_policy(&lua, None).unwrap();
        let err = lua
            .load(r#"require("definitely_missing_native_mod_xyz")"#)
            .exec()
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("can't load C modules until field.scripting.enable_native_modules()")
                || err.contains("module 'definitely_missing_native_mod_xyz' not found"),
            "{err}"
        );
        assert!(
            err.contains("can't load C modules until field.scripting.enable_native_modules()"),
            "{err}"
        );

        policy.enable_native_modules(&lua).unwrap();
        policy.enable_native_modules(&lua).unwrap();
        let err2 = lua
            .load(r#"require("definitely_missing_native_mod_xyz")"#)
            .exec()
            .unwrap_err()
            .to_string();
        assert!(
            !err2.contains("can't load C modules until field.scripting.enable_native_modules()"),
            "{err2}"
        );
    }

    #[test]
    fn require_finds_module_in_config_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mymod.lua"), "return { n = 7 }").unwrap();
        let lua = unsafe { Lua::unsafe_new() };
        let _policy = install_package_policy(&lua, Some(dir.path())).unwrap();
        let n: i64 = lua.load("return require('mymod').n").eval().unwrap();
        assert_eq!(n, 7);
    }

    #[test]
    fn local_lua_path_includes_config() {
        let path = local_lua_path(Some(PathBuf::from("/tmp/cfg").as_path()));
        assert!(path.contains("/tmp/cfg/?.lua"));
        assert!(path.contains("./?.lua"));
    }
}
