// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! `variables.json` envelope (user-scoped variables beside `settings.json`).

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::table::{split_variable_id, StoredVariable, VariableEntry, VariableTable};

/// On-disk kind marker for `variables.json`.
pub const VARIABLES_KIND: &str = "variables";
/// Current `variables.json` format version.
pub const VARIABLES_FORMAT_VERSION: u32 = 1;

/// Top-level scope for user variables.
pub const USER_SCOPE: &str = "user";

/// Versioned envelope for user-scoped variables.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct UserVariablesFile {
    /// File kind marker.
    pub kind: String,
    /// Format version.
    pub format_version: u32,
    /// User-scoped variable entries (scope implied: `user`; dots in `name`
    /// introduce sub-scopes under `user`).
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
    /// `{config_dir}/variables.json` when `config_dir` is `Some`.
    pub fn path_in(config_dir: Option<&Path>) -> Option<PathBuf> {
        config_dir.map(|dir| dir.join("variables.json"))
    }

    /// Load from `{config_dir}/variables.json`, or defaults when missing/invalid.
    pub fn load_from_dir(config_dir: Option<&Path>) -> Self {
        let Some(path) = Self::path_in(config_dir) else {
            return Self::default();
        };
        Self::load_from_path(&path)
    }

    /// Load from an explicit path, or defaults when missing/invalid.
    pub fn load_from_path(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
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

    /// Write pretty JSON to `path` (creating parent dirs if needed).
    pub fn save_to_path(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| format!("{err:#}"))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|err| format!("{err:#}"))?;
        std::fs::write(path, text + "\n").map_err(|err| format!("{err:#}"))
    }

    /// Write pretty JSON to `{config_dir}/variables.json`.
    pub fn save_to_dir(&self, config_dir: Option<&Path>) -> Result<(), String> {
        let path = Self::path_in(config_dir)
            .ok_or_else(|| "config directory is unavailable".to_string())?;
        self.save_to_path(&path)
    }

    /// Convert to a scoped [`VariableTable`].
    ///
    /// Stored names are paths under implicit `user`: `artist` → scope `user`,
    /// `ingest.root_dir` → scope `user.ingest` / leaf `root_dir`.
    pub fn to_table(&self) -> VariableTable {
        let mut table = VariableTable::new();
        for item in &self.variables {
            let Some((scope, leaf)) = parse_user_variable_name(&item.name) else {
                continue;
            };
            let mut entry = VariableEntry::new(scope, leaf, item.value.clone());
            if let Some(description) = &item.description {
                entry = entry.with_description(description.clone());
            }
            table.upsert(entry);
        }
        table
    }

    /// Replace from a scoped table (path under `user` written as stored `name`).
    pub fn from_table(table: &VariableTable) -> Self {
        let mut variables = Vec::with_capacity(table.len());
        for entry in table.entries() {
            let Some(stored_name) = user_variable_display_name(&entry.scope, &entry.name) else {
                continue;
            };
            let mut stored = StoredVariable::new(stored_name, entry.value.clone());
            if let Some(description) = &entry.description {
                stored = stored.with_description(description.clone());
            }
            variables.push(stored);
        }
        Self {
            kind: VARIABLES_KIND.into(),
            format_version: VARIABLES_FORMAT_VERSION,
            variables,
        }
    }
}

/// Parse a User Variables editor / `variables.json` name into `(scope, leaf)`.
///
/// - `"artist"` → `("user", "artist")`
/// - `"ingest.root_dir"` → `("user.ingest", "root_dir")`
/// - `"user.ingest.root_dir"` → `("user.ingest", "root_dir")` (leading `user.`
///   is not doubled)
///
/// Returns `None` when empty or any segment is empty (`ingest.`, `.root`,
/// `a..b`).
pub fn parse_user_variable_name(raw: &str) -> Option<(String, String)> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    if text.split('.').any(|seg| seg.is_empty()) {
        return None;
    }
    let relative = text
        .strip_prefix("user.")
        .filter(|rest| !rest.is_empty())
        .unwrap_or(text);
    if relative.split('.').any(|seg| seg.is_empty()) {
        return None;
    }
    let (sub, leaf) = split_variable_id(relative);
    if leaf.is_empty() {
        return None;
    }
    let scope = if sub.is_empty() {
        USER_SCOPE.to_string()
    } else {
        format!("{USER_SCOPE}.{sub}")
    };
    Some((scope, leaf.to_string()))
}

/// Path under `user` for display / persistence (`ingest.root_dir`), or just the
/// leaf when scope is exactly `user`.
///
/// Returns `None` when `scope` is not under the `user` top-level.
pub fn user_variable_display_name(scope: &str, leaf: &str) -> Option<String> {
    if leaf.is_empty() || leaf.contains('.') {
        return None;
    }
    if scope == USER_SCOPE {
        return Some(leaf.to_string());
    }
    let Some(rest) = scope.strip_prefix("user.") else {
        return None;
    };
    if rest.is_empty() || rest.split('.').any(|seg| seg.is_empty()) {
        return None;
    }
    Some(format!("{rest}.{leaf}"))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn round_trip_subscope() {
        let mut table = VariableTable::new();
        table.upsert(VariableEntry::new("user.ingest", "root_dir", "/data"));
        let file = UserVariablesFile::from_table(&table);
        assert_eq!(file.variables.len(), 1);
        assert_eq!(file.variables[0].name, "ingest.root_dir");
        let back = file.to_table();
        let entry = back.get_qualified("user.ingest.root_dir").unwrap();
        assert_eq!(entry.scope, "user.ingest");
        assert_eq!(entry.name, "root_dir");
        assert_eq!(entry.value, "/data");
    }

    #[test]
    fn parse_plain_and_dotted() {
        assert_eq!(
            parse_user_variable_name("artist"),
            Some(("user".into(), "artist".into()))
        );
        assert_eq!(
            parse_user_variable_name("ingest.root_dir"),
            Some(("user.ingest".into(), "root_dir".into()))
        );
        assert_eq!(
            parse_user_variable_name("user.ingest.root_dir"),
            Some(("user.ingest".into(), "root_dir".into()))
        );
        assert_eq!(
            parse_user_variable_name("a.b.c"),
            Some(("user.a.b".into(), "c".into()))
        );
    }

    #[test]
    fn parse_rejects_empty_segments() {
        assert_eq!(parse_user_variable_name(""), None);
        assert_eq!(parse_user_variable_name("   "), None);
        assert_eq!(parse_user_variable_name("ingest."), None);
        assert_eq!(parse_user_variable_name(".root"), None);
        assert_eq!(parse_user_variable_name("a..b"), None);
        assert_eq!(parse_user_variable_name("user."), None);
    }

    #[test]
    fn display_name_under_user() {
        assert_eq!(
            user_variable_display_name("user", "artist").as_deref(),
            Some("artist")
        );
        assert_eq!(
            user_variable_display_name("user.ingest", "root_dir").as_deref(),
            Some("ingest.root_dir")
        );
        assert_eq!(user_variable_display_name("session", "x"), None);
        assert_eq!(user_variable_display_name("user", "a.b"), None);
    }

    #[test]
    fn load_from_dir_reads_variables_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("variables.json");
        std::fs::write(
            &path,
            r#"{
  "kind": "variables",
  "format_version": 1,
  "variables": [
    { "name": "artist", "value": "Ada" },
    { "name": "ingest.root_dir", "value": "/data" }
  ]
}
"#,
        )
        .unwrap();
        let table = UserVariablesFile::load_from_dir(Some(dir.path())).to_table();
        assert_eq!(table.get_by_name("artist").unwrap().value, "Ada");
        assert_eq!(
            table.get_qualified("user.ingest.root_dir").unwrap().value,
            "/data"
        );
    }
}
