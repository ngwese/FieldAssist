// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Discovery of `resolver_*.lua` / `workflow_*.lua` along the scripting search path.

use std::path::{Path, PathBuf};

/// Prefix for user resolver scripts (`resolver_*.lua`).
pub const RESOLVER_PREFIX: &str = "resolver_";
/// Prefix for user workflow scripts (`workflow_*.lua`).
pub const WORKFLOW_PREFIX: &str = "workflow_";

/// Directories searched for resolver/workflow scripts: config dir first, then
/// `scripting.search_path` from `{config}/settings.json`.
///
/// Blank entries and duplicates of the config directory are skipped. Missing
/// directories remain in the list so callers can skip them when listing files.
pub fn script_search_dirs(config: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(config) = config {
        dirs.push(config.to_path_buf());
        let settings = field_settings::load_from_dir(Some(config));
        for entry in settings.scripting.search_path {
            let path = PathBuf::from(entry);
            if paths_equal(&path, config) {
                continue;
            }
            if dirs.iter().any(|d| paths_equal(d, &path)) {
                continue;
            }
            dirs.push(path);
        }
    }
    dirs
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
            p.is_file()
                && p.extension().and_then(|e| e.to_str()) == Some("lua")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|name| name.starts_with(prefix))
        })
        .collect::<Vec<_>>();
    files.sort_by(|a, b| {
        a.file_name()
            .unwrap_or_default()
            .cmp(b.file_name().unwrap_or_default())
    });
    files
}

/// Collect matching scripts from every search directory (config first).
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
    use field_settings::{save_to_dir, AppSettings};

    #[test]
    fn search_dirs_include_extras_from_settings() {
        let config = tempfile::tempdir().unwrap();
        let extra = tempfile::tempdir().unwrap();
        let mut settings = AppSettings::default();
        settings.scripting.search_path = vec![extra.path().display().to_string()];
        save_to_dir(&settings, Some(config.path())).unwrap();

        let dirs = script_search_dirs(Some(config.path()));
        assert_eq!(dirs.len(), 2);
        assert_eq!(dirs[0], config.path());
        assert_eq!(dirs[1], extra.path());
    }

    #[test]
    fn collect_matching_lua_walks_search_path() {
        let config = tempfile::tempdir().unwrap();
        let extra = tempfile::tempdir().unwrap();
        std::fs::write(config.path().join("resolver_a.lua"), "-- a").unwrap();
        std::fs::write(extra.path().join("resolver_b.lua"), "-- b").unwrap();
        let mut settings = AppSettings::default();
        settings.scripting.search_path = vec![extra.path().display().to_string()];
        save_to_dir(&settings, Some(config.path())).unwrap();

        let files = collect_matching_lua(Some(config.path()), RESOLVER_PREFIX);
        assert_eq!(files.len(), 2);
        assert!(files[0].ends_with("resolver_a.lua"));
        assert!(files[1].ends_with("resolver_b.lua"));
    }
}
