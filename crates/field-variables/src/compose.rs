// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Layered variable composition.

use std::collections::{BTreeMap, BTreeSet};

use crate::table::{top_level_scope, VariableEntry, VariableTable, TOP_LEVEL_SCOPES};

/// Default top-level scope order: earlier layers lose to later ones.
pub const DEFAULT_SCOPE_ORDER: &[&str] = TOP_LEVEL_SCOPES;

/// Compose tables in [`DEFAULT_SCOPE_ORDER`].
///
/// Each input table may contain any scopes; entries are grouped by top-level
/// scope, then merged in order so the **last** definition of a given leaf
/// `name` wins. The composed table stores the winning entry (including its
/// original full scope path).
pub fn compose(tables: &[VariableTable]) -> VariableTable {
    compose_with_order(tables, DEFAULT_SCOPE_ORDER)
}

/// Compose with an explicit top-level scope order (last wins per leaf name).
pub fn compose_with_order(tables: &[VariableTable], order: &[&str]) -> VariableTable {
    let mut by_top: BTreeMap<String, Vec<VariableEntry>> = BTreeMap::new();
    for &name in order {
        by_top.insert(name.to_string(), Vec::new());
    }
    let mut unknown: Vec<VariableEntry> = Vec::new();

    for table in tables {
        for entry in table.entries() {
            let top = top_level_scope(&entry.scope);
            if order.contains(&top) {
                by_top.get_mut(top).unwrap().push(entry.clone());
            } else {
                unknown.push(entry.clone());
            }
        }
    }

    // Last write per leaf name wins across ordered scopes.
    let mut winners: BTreeMap<String, VariableEntry> = BTreeMap::new();
    for &scope_name in order {
        for entry in by_top.get(scope_name).into_iter().flatten() {
            winners.insert(entry.name.clone(), entry.clone());
        }
    }
    for entry in &unknown {
        winners.insert(entry.name.clone(), entry.clone());
    }

    // Emit in scope order for stability.
    let mut out = VariableTable::new();
    let mut emitted = BTreeSet::new();
    for &scope_name in order {
        for entry in by_top.get(scope_name).into_iter().flatten() {
            if let Some(winner) = winners.get(&entry.name) {
                if winner.qualified_id() == entry.qualified_id()
                    && emitted.insert(entry.name.clone())
                {
                    out.upsert(winner.clone());
                }
            }
        }
    }
    for entry in &unknown {
        if emitted.insert(entry.name.clone()) {
            if let Some(winner) = winners.get(&entry.name) {
                out.upsert(winner.clone());
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VariableEntry;

    fn table(scope: &str, pairs: &[(&str, &str)]) -> VariableTable {
        let mut t = VariableTable::new();
        for (name, value) in pairs {
            t.upsert(VariableEntry::new(scope, *name, *value));
        }
        t
    }

    #[test]
    fn last_scope_wins_for_leaf_name() {
        let source = table("source", &[("title", "from-source")]);
        let user = table("user", &[("title", "from-user")]);
        let session = table("session", &[("title", "from-session")]);
        let composed = compose(&[source, user, session]);
        let entry = composed.get_by_name("title").unwrap();
        assert_eq!(entry.value, "from-session");
        assert_eq!(entry.scope, "session");
    }

    #[test]
    fn export_overrides_composition() {
        let composition = table("composition", &[("title", "comp")]);
        let export = table("export", &[("title", "exp")]);
        let composed = compose(&[composition, export]);
        assert_eq!(composed.get_by_name("title").unwrap().value, "exp");
    }

    #[test]
    fn distinct_names_all_present() {
        let source = table("source.bwf", &[("Originator", "BBC")]);
        let user = table("user", &[("project", "Show")]);
        let composed = compose(&[source, user]);
        assert_eq!(composed.len(), 2);
        assert_eq!(
            composed
                .get_qualified("source.bwf.Originator")
                .unwrap()
                .value,
            "BBC"
        );
        assert_eq!(
            composed.get_qualified("user.project").unwrap().value,
            "Show"
        );
    }

    #[test]
    fn source_subscopes_same_leaf_later_wins() {
        let mut a = VariableTable::new();
        a.upsert(VariableEntry::new("source.riff", "INAM", "riff-title"));
        let mut b = VariableTable::new();
        b.upsert(VariableEntry::new("source.bwf", "INAM", "bwf-title"));
        let composed = compose(&[a, b]);
        assert_eq!(composed.get_by_name("INAM").unwrap().value, "bwf-title");
    }
}
