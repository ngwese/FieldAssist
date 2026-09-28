// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Load and save `settings.json` beside `init.lua`.

use std::path::{Path, PathBuf};

use crate::groups::{AppSettings, SETTINGS_FORMAT_VERSION, SETTINGS_KIND};

/// `{config_dir}/settings.json` when `config_dir` is `Some`.
pub fn path_in(config_dir: Option<&Path>) -> Option<PathBuf> {
    config_dir.map(|dir| dir.join("settings.json"))
}

/// Load from `{config_dir}/settings.json`, or defaults when missing/invalid.
pub fn load_from_dir(config_dir: Option<&Path>) -> AppSettings {
    let Some(path) = path_in(config_dir) else {
        return AppSettings::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<AppSettings>(&text) {
            Ok(mut settings) if settings.kind == SETTINGS_KIND => {
                settings.normalize(config_dir);
                settings
            }
            Ok(settings) => {
                eprintln!(
                    "FieldAssist: ignoring settings.json with unexpected kind {:?} ({})",
                    settings.kind,
                    path.display()
                );
                AppSettings::default()
            }
            Err(err) => {
                eprintln!(
                    "FieldAssist: ignoring invalid settings.json ({}): {err}",
                    path.display()
                );
                AppSettings::default()
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => AppSettings::default(),
        Err(err) => {
            eprintln!(
                "FieldAssist: could not read settings.json ({}): {err}",
                path.display()
            );
            AppSettings::default()
        }
    }
}

/// Write pretty JSON under `config_dir` (creating it if needed).
pub fn save_to_dir(settings: &AppSettings, config_dir: Option<&Path>) -> Result<(), String> {
    let path = path_in(config_dir).ok_or_else(|| "config directory is unavailable".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("{err:#}"))?;
    }
    let mut out = settings.clone();
    out.kind = SETTINGS_KIND.into();
    out.format_version = SETTINGS_FORMAT_VERSION;
    out.normalize(config_dir);
    let text = serde_json::to_string_pretty(&out).map_err(|err| format!("{err:#}"))?;
    std::fs::write(&path, text + "\n").map_err(|err| format!("{err:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::groups::{ScriptingSettings, DOCK_HIDDEN, SETTINGS_KIND};

    #[test]
    fn load_missing_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let s = load_from_dir(Some(dir.path()));
        assert_eq!(s.kind, SETTINGS_KIND);
        assert!(s.scripting.search_path.is_empty());
        assert!(!s.scripting.enable_native_modules);
    }

    #[test]
    fn round_trip_preserves_scripting_flags() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = AppSettings::default();
        s.scripting.search_path = vec![dir.path().join("extra").display().to_string()];
        s.scripting.enable_system_package_paths = true;
        s.scripting.enable_native_modules = true;
        save_to_dir(&s, Some(dir.path())).unwrap();
        let back = load_from_dir(Some(dir.path()));
        assert!(back.scripting.enable_system_package_paths);
        assert!(back.scripting.enable_native_modules);
        assert_eq!(back.scripting.search_path.len(), 1);
    }

    #[test]
    fn scripting_normalize_drops_blanks() {
        let mut s = ScriptingSettings {
            search_path: vec!["  ".into(), "C:/ok".into(), "".into(), "  D:/also  ".into()],
            ..Default::default()
        };
        s.normalize(None);
        assert_eq!(
            s.search_path,
            vec!["C:/ok".to_string(), "D:/also".to_string()]
        );
    }

    #[test]
    fn legacy_bool_dock_prefs_deserialize() {
        let json = r#"{
            "view": { "explorer": true, "detail": true, "script": false }
        }"#;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.view.detail, crate::groups::DETAIL_DOCK_TRUE_TAB);
        assert_eq!(s.view.script, DOCK_HIDDEN);
    }
}
