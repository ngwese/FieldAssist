// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! `field.fs` — filesystem helpers and find_files.

use std::path::{Path, PathBuf};

use mlua::{Lua, Table, Value};

use crate::url::LuaUrl;

/// Recursively list files under `root` as `/`-separated paths relative to `root`.
pub fn find_files_matching(
    root: &Path,
    mut include: impl FnMut(&str, &str) -> Result<bool, String>,
) -> Result<Vec<String>, String> {
    let meta = std::fs::metadata(root).map_err(|err| format!("{}: {err}", root.display()))?;
    if !meta.is_dir() {
        return Err(format!(
            "find_files expects a directory, got {}",
            root.display()
        ));
    }
    let mut found = Vec::new();
    let mut stack = vec![WalkDir {
        abs: root.to_path_buf(),
        rel: String::new(),
    }];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir.abs) {
            Ok(entries) => entries,
            Err(err) if dir.rel.is_empty() => {
                return Err(format!("{}: {err}", dir.abs.display()));
            }
            Err(_) => continue,
        };
        let mut children: Vec<_> = entries.filter_map(|entry| entry.ok()).collect();
        children.sort_by_key(|entry| entry.file_name());
        for entry in children.into_iter().rev() {
            let name = entry.file_name();
            let Some(basename) = name.to_str() else {
                continue;
            };
            if basename == "." || basename == ".." {
                continue;
            }
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if file_type.is_symlink() {
                continue;
            }
            let rel = join_rel(&dir.rel, basename);
            if file_type.is_dir() {
                stack.push(WalkDir {
                    abs: entry.path(),
                    rel,
                });
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            if include(&dir.rel, basename)? {
                found.push(rel);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// List files, optionally filtering by extension (without dots, case-insensitive).
pub fn find_files(root: &Path, extensions: Option<&[String]>) -> Result<Vec<String>, String> {
    find_files_matching(root, |_, basename| {
        Ok(match extensions {
            None => true,
            Some(exts) => extension_matches(basename, exts),
        })
    })
}

/// Normalize an extension filter token.
pub fn normalize_extension(value: &str) -> String {
    value.trim().trim_start_matches('.').to_ascii_lowercase()
}

fn extension_matches(basename: &str, extensions: &[String]) -> bool {
    let Some(ext) = Path::new(basename).extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    extensions.iter().any(|wanted| wanted == &ext)
}

fn join_rel(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

struct WalkDir {
    abs: PathBuf,
    rel: String,
}

fn path_from_lua(value: Value) -> mlua::Result<PathBuf> {
    match value {
        Value::String(s) => Ok(PathBuf::from(s.to_str()?.as_ref())),
        Value::UserData(ud) => {
            let url = ud.borrow::<LuaUrl>()?;
            url.to_path(None)
        }
        other => Err(mlua::Error::runtime(format!(
            "expected path string or url, got {}",
            other.type_name()
        ))),
    }
}

/// Bind `field.fs`.
pub fn bind_fs(lua: &Lua, field: &Table) -> mlua::Result<()> {
    let fs = lua.create_table()?;
    fs.set(
        "find_files",
        lua.create_function(|lua, (dir, filter): (Value, Value)| {
            let root = path_from_lua(dir)?;
            match filter {
                Value::Nil => {
                    let found = find_files(&root, None).map_err(mlua::Error::runtime)?;
                    list_to_lua(lua, found)
                }
                Value::Table(table) => {
                    let mut exts = Vec::new();
                    for pair in table.sequence_values::<String>() {
                        exts.push(normalize_extension(&pair?));
                    }
                    let found = find_files(&root, Some(&exts)).map_err(mlua::Error::runtime)?;
                    list_to_lua(lua, found)
                }
                Value::Function(pred) => {
                    let found = find_files_matching(&root, |dirname, basename| {
                        pred.call::<bool>((dirname, basename))
                            .map_err(|err| err.to_string())
                    })
                    .map_err(mlua::Error::runtime)?;
                    list_to_lua(lua, found)
                }
                other => Err(mlua::Error::runtime(format!(
                    "find_files filter must be a table, function, or nil, got {}",
                    other.type_name()
                ))),
            }
        })?,
    )?;
    fs.set(
        "mkdir",
        lua.create_function(|_, (path, opts): (Value, Value)| {
            let path = path_from_lua(path)?;
            let recursive = match opts {
                Value::Nil => true,
                Value::Table(t) => t.get::<Option<bool>>("recursive")?.unwrap_or(true),
                _ => true,
            };
            if recursive {
                std::fs::create_dir_all(&path).map_err(|e| mlua::Error::runtime(e.to_string()))?;
            } else {
                std::fs::create_dir(&path).map_err(|e| mlua::Error::runtime(e.to_string()))?;
            }
            Ok(())
        })?,
    )?;
    fs.set(
        "remove",
        lua.create_function(|_, (path, opts): (Value, Value)| {
            let path = path_from_lua(path)?;
            let recursive = match opts {
                Value::Nil => false,
                Value::Table(t) => t.get::<Option<bool>>("recursive")?.unwrap_or(false),
                _ => false,
            };
            let meta = std::fs::metadata(&path).map_err(|e| mlua::Error::runtime(e.to_string()))?;
            if meta.is_dir() {
                if recursive {
                    std::fs::remove_dir_all(&path)
                        .map_err(|e| mlua::Error::runtime(e.to_string()))?;
                } else {
                    std::fs::remove_dir(&path).map_err(|e| mlua::Error::runtime(e.to_string()))?;
                }
            } else {
                std::fs::remove_file(&path).map_err(|e| mlua::Error::runtime(e.to_string()))?;
            }
            Ok(())
        })?,
    )?;
    fs.set(
        "copy",
        lua.create_function(|_, (src, dest, opts): (Value, Value, Value)| {
            let src = path_from_lua(src)?;
            let dest = path_from_lua(dest)?;
            let recursive = match opts {
                Value::Nil => false,
                Value::Table(t) => t.get::<Option<bool>>("recursive")?.unwrap_or(false),
                _ => false,
            };
            let meta = std::fs::metadata(&src).map_err(|e| mlua::Error::runtime(e.to_string()))?;
            if meta.is_dir() {
                if !recursive {
                    return Err(mlua::Error::runtime(
                        "copy of a directory requires { recursive = true }",
                    ));
                }
                copy_dir_recursive(&src, &dest).map_err(mlua::Error::runtime)?;
            } else {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| mlua::Error::runtime(e.to_string()))?;
                }
                std::fs::copy(&src, &dest).map_err(|e| mlua::Error::runtime(e.to_string()))?;
            }
            Ok(())
        })?,
    )?;
    fs.set(
        "exists",
        lua.create_function(|_, path: Value| {
            let path = path_from_lua(path)?;
            Ok(path.exists())
        })?,
    )?;
    fs.set(
        "stat",
        lua.create_function(|lua, path: Value| {
            let path = path_from_lua(path)?;
            let meta = std::fs::metadata(&path).map_err(|e| mlua::Error::runtime(e.to_string()))?;
            let table = lua.create_table()?;
            table.set("size", meta.len() as i64)?;
            table.set("is_dir", meta.is_dir())?;
            table.set("is_file", meta.is_file())?;
            if let Ok(modified) = meta.modified() {
                if let Ok(secs) = modified.duration_since(std::time::UNIX_EPOCH) {
                    table.set("mtime", secs.as_secs_f64())?;
                }
            }
            Ok(table)
        })?,
    )?;
    fs.set(
        "checksum",
        lua.create_function(|_, (path, algo): (Value, Value)| {
            let path = path_from_lua(path)?;
            let algo = match algo {
                Value::Nil => "blake3".to_string(),
                Value::String(s) => s.to_str()?.to_ascii_lowercase(),
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "checksum algorithm must be a string, got {}",
                        other.type_name()
                    )))
                }
            };
            if algo != "blake3" {
                return Err(mlua::Error::runtime(format!(
                    "unsupported checksum algorithm `{algo}`"
                )));
            }
            let bytes = std::fs::read(&path).map_err(|e| mlua::Error::runtime(e.to_string()))?;
            Ok(blake3::hash(&bytes).to_hex().to_string())
        })?,
    )?;
    field.set("fs", fs)?;
    Ok(())
}

fn list_to_lua(lua: &Lua, found: Vec<String>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for (i, path) in found.into_iter().enumerate() {
        table.set(i + 1, path)?;
    }
    Ok(table)
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        let to = dest.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &to)?;
        } else if ty.is_file() {
            std::fs::copy(entry.path(), to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;
    use crate::host::{HostProfile, ScriptHost};

    #[test]
    fn find_files_and_checksum_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("nested")).unwrap();
        std::fs::write(dir.path().join("take.wav"), b"wav").unwrap();
        std::fs::write(dir.path().join("nested").join("more.flac"), b"flac").unwrap();

        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: None,
        })
        .unwrap();
        let path = dir.path().display().to_string().replace('\\', "\\\\");
        let code = format!(
            r#"
            local found = field.fs.find_files("{path}", {{ "wav", "flac" }})
            local sum = field.fs.checksum("{path}/take.wav")
            return table.concat(found, ","), #sum > 0
            "#,
            path = path
        );
        let out = host.eval(&code);
        assert!(out.error.is_none(), "{:?}", out.error);
        assert!(
            out.result
                .as_deref()
                .is_some_and(|r| r.contains("take.wav") && r.contains("true")),
            "{:?}",
            out.result
        );
    }

    #[test]
    fn mkdir_copy_exists() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: None,
        })
        .unwrap();
        let root = dir.path().display().to_string().replace('\\', "\\\\");
        let code = format!(
            r#"
            field.fs.mkdir("{root}/a/b", {{ recursive = true }})
            local src = "{root}/a/b/file.txt"
            local dest = "{root}/out/file.txt"
            -- write via io
            local f = io.open(src, "w"); f:write("hi"); f:close()
            field.fs.copy(src, dest)
            return field.fs.exists(dest), field.fs.stat(dest).size
            "#,
            root = root
        );
        let out = host.eval(&code);
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("true\t2"));
    }
}
