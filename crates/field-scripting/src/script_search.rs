// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Discovery of `resolver_*.lua` / `workflow_*.lua` along the scripting search path.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Prefix for user resolver scripts (`resolver_*.lua`).
pub const RESOLVER_PREFIX: &str = "resolver_";
/// Prefix for user workflow scripts (`workflow_*.lua`).
pub const WORKFLOW_PREFIX: &str = "workflow_";

#[derive(Debug, Default, Deserialize)]
struct SettingsFile {
    #[serde(default)]
    scripting: ScriptingSection,
}

#[derive(Debug, Default, Deserialize)]
struct ScriptingSection {
    #[serde(default)]
    search_path: Vec<String>,
}

/// Directories searched for resolver/workflow scripts: config dir first, then
/// `scripting.search_path` from `{config}/settings.json`.
///
/// Blank entries and duplicates of the config directory are skipped. Missing
/// directories remain in the list so callers can skip them when listing files.
pub fn script_search_dirs(config: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(config) = config {
        dirs.push(config.to_path_buf());
        if let Some(extras) = read_extra_search_path(config) {
            for entry in extras {
                let trimmed = entry.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let path = PathBuf::from(trimmed);
                if paths_equal(&path, config) {
                    continue;
                }
                if dirs.iter().any(|d| paths_equal(d, &path)) {
                    continue;
                }
                dirs.push(path);
            }
        }
    }
    dirs
}

fn read_extra_search_path(config: &Path) -> Option<Vec<String>> {
    let path = config.join("settings.json");
    let text = std::fs::read_to_string(path).ok()?;
    let file: SettingsFile = serde_json::from_str(&text).ok()?;
    Some(file.scripting.search_path)
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Non-recursive listing of `{prefix}*.lua` in `dir`, sorted by file name.
pub fn matching_lua_files(dir: &Path, prefix: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(prefix) && n.ends_with(".lua"))
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

/// Flatten [`script_search_dirs`] + [`matching_lua_files`] for `prefix`.
pub fn collect_matching_lua(config: Option<&Path>, prefix: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in script_search_dirs(config) {
        out.extend(matching_lua_files(&dir, prefix));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn matching_lua_files_filters_and_sorts() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("workflow_b.lua"), "-- b").unwrap();
        fs::write(dir.path().join("workflow_a.lua"), "-- a").unwrap();
        fs::write(dir.path().join("other.lua"), "-- skip").unwrap();
        fs::write(dir.path().join("resolver_x.lua"), "-- skip").unwrap();
        let files = matching_lua_files(dir.path(), WORKFLOW_PREFIX);
        let names: Vec<_> = files
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
            .collect();
        assert_eq!(names, ["workflow_a.lua", "workflow_b.lua"]);
    }

    #[test]
    fn script_search_dirs_config_first_then_extras() {
        let config = tempfile::tempdir().unwrap();
        let extra = tempfile::tempdir().unwrap();
        let settings = format!(
            r#"{{
                "kind": "settings",
                "format_version": 1,
                "scripting": {{
                    "search_path": ["{}", "  ", "{}"]
                }}
            }}"#,
            extra.path().display().to_string().replace('\\', "/"),
            config.path().display().to_string().replace('\\', "/")
        );
        fs::write(config.path().join("settings.json"), settings).unwrap();

        let dirs = script_search_dirs(Some(config.path()));
        assert_eq!(dirs.len(), 2);
        assert_eq!(dirs[0], config.path());
        assert_eq!(dirs[1], extra.path());
    }

    #[test]
    fn collect_matching_walks_dirs_in_order() {
        let config = tempfile::tempdir().unwrap();
        let extra = tempfile::tempdir().unwrap();
        fs::write(config.path().join("workflow_first.lua"), "return 1").unwrap();
        fs::write(extra.path().join("workflow_second.lua"), "return 2").unwrap();
        let settings = format!(
            r#"{{
                "scripting": {{ "search_path": ["{}"] }}
            }}"#,
            extra.path().display().to_string().replace('\\', "/")
        );
        fs::write(config.path().join("settings.json"), settings).unwrap();

        let files = collect_matching_lua(Some(config.path()), WORKFLOW_PREFIX);
        let names: Vec<_> = files
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
            .collect();
        assert_eq!(names, ["workflow_first.lua", "workflow_second.lua"]);
    }

    #[test]
    fn missing_extra_dir_is_skipped_when_listing() {
        let config = tempfile::tempdir().unwrap();
        let missing = config.path().join("nope");
        let settings = format!(
            r#"{{
                "scripting": {{ "search_path": ["{}"] }}
            }}"#,
            missing.display().to_string().replace('\\', "/")
        );
        fs::write(config.path().join("settings.json"), settings).unwrap();
        fs::write(config.path().join("resolver_only.lua"), "return 1").unwrap();
        let files = collect_matching_lua(Some(config.path()), RESOLVER_PREFIX);
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("resolver_only.lua"));
    }
}
