// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! User-scoped variables (`variables.json` next to `settings.json`).

use std::path::PathBuf;

use field_variables::{StoredVariable, VariableTable};
use gpui_kit::{App, Global};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// On-disk kind marker for `variables.json`.
pub const VARIABLES_KIND: &str = "variables";
/// Current `variables.json` format version.
pub const VARIABLES_FORMAT_VERSION: u32 = 1;

/// Versioned envelope for user-scoped variables.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct UserVariablesFile {
    /// File kind marker.
    pub kind: String,
    /// Format version.
    pub format_version: u32,
    /// User-scoped variable entries (scope implied: `user`).
    pub variables: Vec<StoredVariable>,
}

impl Default for UserVariablesFile {
    fn default() -> Self {
        Self {
            kind: VARIABLES_KIND.into(),
            format_version: VARIABLES_FORMAT_VERSION,
            variables: Vec::new(),
        }
    }
}

impl UserVariablesFile {
    /// Path to `variables.json` beside `settings.json`, if the config dir resolves.
    pub fn path() -> Option<PathBuf> {
        crate::commands::user_config_dir().map(|dir| dir.join("variables.json"))
    }

    /// Load from disk, or defaults when missing/invalid.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<UserVariablesFile>(&text) {
                Ok(file) if file.kind == VARIABLES_KIND => file,
                Ok(file) => {
                    eprintln!(
                        "FieldAssist: ignoring variables.json with unexpected kind {:?} ({})",
                        file.kind,
                        path.display()
                    );
                    Self::default()
                }
                Err(err) => {
                    eprintln!(
                        "FieldAssist: ignoring invalid variables.json ({}): {err}",
                        path.display()
                    );
                    Self::default()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(err) => {
                eprintln!(
                    "FieldAssist: could not read variables.json ({}): {err}",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Write pretty JSON to the config directory (creating it if needed).
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or_else(|| "config directory is unavailable".to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| format!("{err:#}"))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|err| format!("{err:#}"))?;
        std::fs::write(&path, text + "\n").map_err(|err| format!("{err:#}"))
    }

    /// Convert to a scoped [`VariableTable`].
    pub fn to_table(&self) -> VariableTable {
        VariableTable::from_stored("user", self.variables.clone())
    }

    /// Replace from a scoped table (scope stripped on save).
    pub fn from_table(table: &VariableTable) -> Self {
        Self {
            kind: VARIABLES_KIND.into(),
            format_version: VARIABLES_FORMAT_VERSION,
            variables: table.to_stored(),
        }
    }
}

/// In-memory user variables store (independent of [`crate::settings::AppSettings`]).
#[derive(Clone, Debug, Default)]
pub struct UserVariablesStore {
    pub file: UserVariablesFile,
}

impl Global for UserVariablesStore {}

/// Ensure a global store exists.
pub fn ensure_store(cx: &mut App) {
    if cx.try_global::<UserVariablesStore>().is_none() {
        cx.set_global(UserVariablesStore::default());
    }
}

/// Borrow the global store.
pub fn store(cx: &App) -> &UserVariablesStore {
    cx.global::<UserVariablesStore>()
}

/// Mutable borrow, creating defaults if needed.
pub fn store_mut(cx: &mut App) -> &mut UserVariablesStore {
    ensure_store(cx);
    cx.global_mut::<UserVariablesStore>()
}

/// Reload user variables from disk into the global store.
pub fn reload_from_disk(cx: &mut App) {
    let file = UserVariablesFile::load();
    store_mut(cx).file = file;
}

/// Persist the in-memory user variables.
#[allow(dead_code)]
pub fn save_store(cx: &App) -> Result<(), String> {
    store(cx).file.save()
}

/// Replace variables, persist, and return the table.
#[allow(dead_code)]
pub fn update_and_save(
    cx: &mut App,
    f: impl FnOnce(&mut VariableTable),
) -> Result<VariableTable, String> {
    let mut table = store(cx).file.to_table();
    f(&mut table);
    let file = UserVariablesFile::from_table(&table);
    file.save()?;
    store_mut(cx).file = file;
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use field_variables::VariableEntry;

    #[test]
    fn round_trip_table() {
        let mut table = VariableTable::new();
        table.upsert(
            VariableEntry::new("user", "artist", "Greg").with_description("default artist"),
        );
        let file = UserVariablesFile::from_table(&table);
        let json = serde_json::to_string(&file).unwrap();
        let back: UserVariablesFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.variables.len(), 1);
        assert_eq!(back.variables[0].name, "artist");
        assert_eq!(back.to_table().get_by_name("artist").unwrap().value, "Greg");
    }
}
