// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Variable entries and tables.

use std::collections::BTreeMap;
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Well-known top-level scope names in default compose order.
pub const TOP_LEVEL_SCOPES: &[&str] = &["source", "user", "session", "composition", "export"];

/// Persistable variable without a scope (scope is implied by the store).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoredVariable {
    /// Leaf name.
    pub name: String,
    /// String value.
    pub value: String,
    /// Optional human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl StoredVariable {
    /// Build a stored variable.
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            description: None,
        }
    }

    /// Attach a description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Convert into a scoped [`VariableEntry`].
    pub fn into_entry(self, scope: impl Into<String>) -> VariableEntry {
        VariableEntry {
            name: self.name,
            value: self.value,
            description: self.description,
            scope: scope.into(),
        }
    }
}

impl From<&VariableEntry> for StoredVariable {
    fn from(entry: &VariableEntry) -> Self {
        Self {
            name: entry.name.clone(),
            value: entry.value.clone(),
            description: entry.description.clone(),
        }
    }
}

/// One variable: string value with optional description and scope path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariableEntry {
    /// Leaf name (text after the last `.` of a qualified id).
    pub name: String,
    /// String value.
    pub value: String,
    /// Optional human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Full scope path (`"user"`, `"source.bwf"`, …). Empty means unresolved /
    /// bare storage within a fixed-scope table.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scope: String,
}

impl VariableEntry {
    /// Build an entry with the given scope path and leaf name.
    pub fn new(
        scope: impl Into<String>,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            description: None,
            scope: scope.into(),
        }
    }

    /// Attach a description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Qualified id `scope.name`, or just `name` when scope is empty.
    pub fn qualified_id(&self) -> String {
        if self.scope.is_empty() {
            self.name.clone()
        } else {
            format!("{}.{}", self.scope, self.name)
        }
    }
}

/// Split a variable identifier into `(scope, name)`.
///
/// - `"title"` → `("", "title")`
/// - `"source.bwf.Originator"` → `("source.bwf", "Originator")`
pub fn split_variable_id(id: &str) -> (&str, &str) {
    match id.rfind('.') {
        Some(idx) => (&id[..idx], &id[idx + 1..]),
        None => ("", id),
    }
}

/// First segment of a scope path (`"source.bwf"` → `"source"`).
pub fn top_level_scope(scope: &str) -> &str {
    scope.split('.').next().unwrap_or(scope)
}

/// A collection of variables, keyed by qualified id when scoped.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VariableTable {
    entries: Vec<VariableEntry>,
}

impl VariableTable {
    /// Empty table.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Wrap an existing entry list.
    pub fn from_entries(entries: Vec<VariableEntry>) -> Self {
        Self { entries }
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Borrow entries in insertion order.
    pub fn entries(&self) -> &[VariableEntry] {
        &self.entries
    }

    /// Mutable access to entries.
    pub fn entries_mut(&mut self) -> &mut Vec<VariableEntry> {
        &mut self.entries
    }

    /// Into owned entries.
    pub fn into_entries(self) -> Vec<VariableEntry> {
        self.entries
    }

    /// Insert or replace by qualified id (scope + name).
    pub fn upsert(&mut self, entry: VariableEntry) {
        let key = entry.qualified_id();
        if let Some(existing) = self.entries.iter_mut().find(|e| e.qualified_id() == key) {
            *existing = entry;
        } else {
            self.entries.push(entry);
        }
    }

    /// Insert under a fixed top-level (or full) scope; leaf name only on `entry.name`.
    pub fn upsert_in_scope(&mut self, scope: &str, mut entry: VariableEntry) {
        entry.scope = scope.to_string();
        self.upsert(entry);
    }

    /// Remove by qualified id; returns whether something was removed.
    pub fn remove(&mut self, qualified_id: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.qualified_id() != qualified_id);
        self.entries.len() != before
    }

    /// Remove by leaf name within a scope.
    pub fn remove_in_scope(&mut self, scope: &str, name: &str) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|e| !(e.scope == scope && e.name == name));
        self.entries.len() != before
    }

    /// Look up by exact qualified id.
    pub fn get_qualified(&self, qualified_id: &str) -> Option<&VariableEntry> {
        self.entries
            .iter()
            .find(|e| e.qualified_id() == qualified_id)
    }

    /// Look up leaf `name` (any scope); returns the first match.
    pub fn get_by_name(&self, name: &str) -> Option<&VariableEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Assign a scope to every entry that has an empty scope (for loading
    /// fixed-scope stores such as `variables.json`).
    pub fn with_default_scope(mut self, scope: &str) -> Self {
        for entry in &mut self.entries {
            if entry.scope.is_empty() {
                entry.scope = scope.to_string();
            }
        }
        self
    }

    /// Map leaf name → value for this table only (last duplicate leaf wins).
    pub fn leaf_map(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        for entry in &self.entries {
            map.insert(entry.name.clone(), entry.value.clone());
        }
        map
    }

    /// Map qualified id → value.
    pub fn qualified_map(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        for entry in &self.entries {
            map.insert(entry.qualified_id(), entry.value.clone());
        }
        map
    }

    /// Serialize as scope-free stored variables (for session / composition / user files).
    pub fn to_stored(&self) -> Vec<StoredVariable> {
        self.entries.iter().map(StoredVariable::from).collect()
    }

    /// Load from stored variables under a fixed scope.
    pub fn from_stored(scope: &str, stored: Vec<StoredVariable>) -> Self {
        let mut table = VariableTable::new();
        for item in stored {
            table.upsert(item.into_entry(scope));
        }
        table
    }
}

impl fmt::Display for VariableEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={:?}", self.qualified_id(), self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_bare_and_scoped() {
        assert_eq!(split_variable_id("title"), ("", "title"));
        assert_eq!(
            split_variable_id("source.bwf.Originator"),
            ("source.bwf", "Originator")
        );
    }

    #[test]
    fn upsert_replaces_same_qualified_id() {
        let mut table = VariableTable::new();
        table.upsert(VariableEntry::new("user", "title", "a"));
        table.upsert(VariableEntry::new("user", "title", "b"));
        assert_eq!(table.len(), 1);
        assert_eq!(table.get_qualified("user.title").unwrap().value, "b");
    }
}
