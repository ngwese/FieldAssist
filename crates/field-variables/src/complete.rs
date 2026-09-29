// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! `${…}` path-segment completion for template text fields.

use std::collections::BTreeMap;

use crate::table::VariableTable;

/// Kind of a completion candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompletionKind {
    /// Next scope path segment (accept appends `.`).
    Scope,
    /// Leaf variable name (accept may append `}`).
    Variable,
}

/// One row in a completion popdown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    /// Text inserted for this segment (scope segment or leaf name).
    pub label: String,
    /// [`CompletionKind::Scope`] or [`CompletionKind::Variable`].
    pub kind: CompletionKind,
    /// Resolved value for variable rows; empty for scopes.
    pub value: String,
}

/// Location of the editable path segment under the caret inside `${…}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionContext {
    /// Byte index of `${`.
    pub expr_start: usize,
    /// Byte index after `${` (start of the expression body).
    pub body_start: usize,
    /// Byte index of `}` if closed, else end of the expression body (EOF).
    pub expr_end: usize,
    /// Whether a closing `}` is present.
    pub closed: bool,
    /// Scope path typed before the current segment (empty at root).
    pub parent_scope: String,
    /// Byte range of the current path segment within the full template.
    pub segment_range: std::ops::Range<usize>,
    /// Prefix of the current segment before the caret.
    pub prefix: String,
}

/// What Tab should do given the current filtered candidates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabAction {
    /// Accept this single match.
    Accept(CompletionItem),
    /// Extend the typed prefix by this common suffix (no focus change).
    InsertPrefix(String),
    /// Shared prefix is already typed; move focus into the selection list.
    FocusList,
    /// No candidates / no active expression.
    None,
}

/// Text to write over [`CompletionContext::segment_range`] when accepting an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcceptEdit {
    /// Replacement for the current segment (may include trailing `.` or `}`).
    pub replacement: String,
    /// Caret offset after the edit, relative to the start of `segment_range`.
    pub caret_after: usize,
}

/// Locate the `${…}` expression and path segment under `caret` (UTF-8 byte offset).
///
/// Returns `None` when the caret is outside an expression.
pub fn completion_context(template: &str, caret: usize) -> Option<CompletionContext> {
    let caret = caret.min(template.len());
    let before = &template[..caret];
    let open = before.rfind("${")?;
    let body_start = open + 2;
    let after_open = &template[body_start..];
    let (body_len, closed) = match after_open.find('}') {
        Some(end) => {
            // Caret must be at or before the closing brace.
            if caret > body_start + end {
                return None;
            }
            (end, true)
        }
        None => (after_open.len(), false),
    };
    let expr_end = body_start + body_len;
    let body = &template[body_start..expr_end];
    let caret_in_body = caret.saturating_sub(body_start).min(body.len());

    // Segment boundaries: previous `.` (or start) … next `.` (or end).
    let seg_start_in_body = body[..caret_in_body].rfind('.').map(|i| i + 1).unwrap_or(0);
    let seg_end_in_body = body[caret_in_body..]
        .find('.')
        .map(|i| caret_in_body + i)
        .unwrap_or(body.len());

    let parent_scope = if seg_start_in_body == 0 {
        String::new()
    } else {
        body[..seg_start_in_body.saturating_sub(1)].to_string()
    };
    let segment = &body[seg_start_in_body..seg_end_in_body];
    let prefix_len = caret_in_body
        .saturating_sub(seg_start_in_body)
        .min(segment.len());
    let prefix = segment[..prefix_len].to_string();

    Some(CompletionContext {
        expr_start: open,
        body_start,
        expr_end,
        closed,
        parent_scope,
        segment_range: (body_start + seg_start_in_body)..(body_start + seg_end_in_body),
        prefix,
    })
}

/// Candidates for the current segment, filtered by [`CompletionContext::prefix`].
pub fn completion_items(ctx: &CompletionContext, vars: &VariableTable) -> Vec<CompletionItem> {
    let mut scopes: BTreeMap<String, ()> = BTreeMap::new();
    let mut variables: BTreeMap<String, String> = BTreeMap::new();
    let parent = ctx.parent_scope.as_str();
    let prefix = ctx.prefix.as_str();

    for entry in vars.entries() {
        if parent.is_empty() {
            // Root: next top-level / nested scope segments, plus every leaf
            // (bare `${name}` resolves via get_by_name).
            if !entry.scope.is_empty() {
                if let Some(seg) = first_segment(&entry.scope) {
                    if seg.starts_with(prefix) {
                        scopes.insert(seg.to_string(), ());
                    }
                }
            }
            if entry.name.starts_with(prefix) {
                variables
                    .entry(entry.name.clone())
                    .or_insert_with(|| entry.value.clone());
            }
        } else if entry.scope == parent {
            if entry.name.starts_with(prefix) {
                variables
                    .entry(entry.name.clone())
                    .or_insert_with(|| entry.value.clone());
            }
        } else if let Some(rest) = entry.scope.strip_prefix(parent) {
            if let Some(rest) = rest.strip_prefix('.') {
                if let Some(seg) = first_segment(rest) {
                    if seg.starts_with(prefix) {
                        scopes.insert(seg.to_string(), ());
                    }
                }
            }
        }
    }

    // Virtual `env` scope is always available at the root (process environment).
    if parent.is_empty() && "env".starts_with(prefix) {
        scopes.insert("env".to_string(), ());
    }

    let mut items = Vec::with_capacity(scopes.len() + variables.len());
    for label in scopes.into_keys() {
        // Prefer scope when a leaf shares the same label at this level.
        variables.remove(&label);
        items.push(CompletionItem {
            label,
            kind: CompletionKind::Scope,
            value: String::new(),
        });
    }
    for (label, value) in variables {
        items.push(CompletionItem {
            label,
            kind: CompletionKind::Variable,
            value,
        });
    }
    items
}

/// Decide what Tab should do for the filtered list and typed prefix.
pub fn tab_action(items: &[CompletionItem], prefix: &str) -> TabAction {
    match items {
        [] => TabAction::None,
        [only] => TabAction::Accept(only.clone()),
        many => {
            let shared = common_prefix(many.iter().map(|i| i.label.as_str()));
            if shared.len() > prefix.len() {
                TabAction::InsertPrefix(shared[prefix.len()..].to_string())
            } else {
                TabAction::FocusList
            }
        }
    }
}

/// Build the text that replaces the current segment when accepting `item`.
pub fn accept_edit(ctx: &CompletionContext, template: &str, item: &CompletionItem) -> AcceptEdit {
    let after_seg = template.get(ctx.segment_range.end..).unwrap_or("");
    match item.kind {
        CompletionKind::Scope => {
            let needs_dot = !after_seg.starts_with('.');
            let replacement = if needs_dot {
                format!("{}.", item.label)
            } else {
                item.label.clone()
            };
            let caret_after = replacement.len();
            AcceptEdit {
                replacement,
                caret_after,
            }
        }
        CompletionKind::Variable => {
            // Append `}` only when the expression is still open and this
            // segment is the last content of the body (nothing after it).
            let needs_brace = !ctx.closed && after_seg.is_empty();
            let replacement = if needs_brace {
                format!("{}}}", item.label)
            } else {
                item.label.clone()
            };
            let caret_after = if needs_brace {
                replacement.len()
            } else {
                item.label.len()
            };
            AcceptEdit {
                replacement,
                caret_after,
            }
        }
    }
}

/// Apply a shared-prefix Tab insert: extend the typed prefix only.
///
/// The returned replacement covers [`CompletionContext::prefix`] plus `insert`.
/// Apply it over [`CompletionContext::prefix_range`], not the full segment.
pub fn prefix_edit(ctx: &CompletionContext, insert: &str) -> AcceptEdit {
    let mut replacement = ctx.prefix.clone();
    replacement.push_str(insert);
    AcceptEdit {
        caret_after: replacement.len(),
        replacement,
    }
}

impl CompletionContext {
    /// Byte range of the typed prefix (segment start through caret).
    pub fn prefix_range(&self) -> std::ops::Range<usize> {
        let start = self.segment_range.start;
        start..(start + self.prefix.len())
    }
}

fn first_segment(path: &str) -> Option<&str> {
    let seg = path.split('.').next()?;
    if seg.is_empty() {
        None
    } else {
        Some(seg)
    }
}

fn common_prefix<'a>(mut labels: impl Iterator<Item = &'a str>) -> String {
    let Some(first) = labels.next() else {
        return String::new();
    };
    let mut prefix = first.to_string();
    for label in labels {
        while !label.starts_with(&prefix) {
            prefix.pop();
            if prefix.is_empty() {
                return prefix;
            }
        }
    }
    prefix
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compose, VariableEntry, VariableTable};

    fn sample_table() -> VariableTable {
        let mut source = VariableTable::new();
        source.upsert(VariableEntry::new("source", "stem", "take"));
        source.upsert(VariableEntry::new("source", "basename", "take01"));
        source.upsert(VariableEntry::new("source.bwf", "Originator", "BBC"));
        source.upsert(VariableEntry::new("source.bwf", "Description", "desc"));
        let mut user = VariableTable::new();
        user.upsert(VariableEntry::new("user", "title", "My Title"));
        // User overrides Originator leaf; composed table keeps user winner.
        user.upsert(VariableEntry::new("user", "Originator", "Studio"));
        compose(&[source, user])
    }

    fn labels(items: &[CompletionItem]) -> Vec<&str> {
        items.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn outside_expression_is_none() {
        assert!(completion_context("hello", 3).is_none());
        assert!(completion_context("pre ${title} post", 2).is_none());
        // After closing brace.
        let t = "${title}x";
        assert!(completion_context(t, t.len()).is_none());
    }

    #[test]
    fn second_expression_uses_local_context() {
        let t = "${title}_${sou";
        let caret = t.len();
        let ctx = completion_context(t, caret).unwrap();
        assert_eq!(ctx.parent_scope, "");
        assert_eq!(ctx.prefix, "sou");
        assert!(!ctx.closed);
        assert_eq!(&t[ctx.segment_range.clone()], "sou");
    }

    #[test]
    fn root_lists_scopes_and_bare_leaves() {
        let vars = sample_table();
        let t = "${";
        let ctx = completion_context(t, t.len()).unwrap();
        let items = completion_items(&ctx, &vars);
        let labs = labels(&items);
        assert!(labs.contains(&"source"));
        assert!(labs.contains(&"user"));
        assert!(labs.contains(&"env"));
        assert!(labs.contains(&"title"));
        assert!(labs.contains(&"stem"));
        assert!(labs.contains(&"basename"));
        // Originator is the user winner (bare leaf).
        assert!(labs.contains(&"Originator"));
        // Scopes come before variables.
        let source_ix = labs.iter().position(|l| *l == "source").unwrap();
        let title_ix = labs.iter().position(|l| *l == "title").unwrap();
        assert!(source_ix < title_ix);
    }

    #[test]
    fn root_offers_env_scope_even_on_empty_table() {
        let vars = VariableTable::new();
        let t = "${e";
        let ctx = completion_context(t, t.len()).unwrap();
        let items = completion_items(&ctx, &vars);
        assert!(labels(&items).contains(&"env"));
        // Do not enumerate process env var names under env.
        let t2 = "${env.";
        let ctx2 = completion_context(t2, t2.len()).unwrap();
        assert_eq!(ctx2.parent_scope, "env");
        assert!(completion_items(&ctx2, &vars).is_empty());
    }

    #[test]
    fn source_dot_lists_bwf_and_source_leaves() {
        let vars = sample_table();
        let t = "${source.";
        let ctx = completion_context(t, t.len()).unwrap();
        assert_eq!(ctx.parent_scope, "source");
        assert_eq!(ctx.prefix, "");
        let items = completion_items(&ctx, &vars);
        let labs = labels(&items);
        assert!(labs.contains(&"bwf"));
        assert!(labs.contains(&"stem"));
        assert!(labs.contains(&"basename"));
        // Originator lives under source.bwf, not source — only via composed
        // last-wins it is user.Originator, so it must not appear here.
        assert!(!labs.contains(&"Originator"));
    }

    #[test]
    fn prefix_filters_and_shared_prefix_tab() {
        let vars = sample_table();
        let t = "${source.b";
        let ctx = completion_context(t, t.len()).unwrap();
        let items = completion_items(&ctx, &vars);
        let labs = labels(&items);
        assert!(labs.contains(&"bwf"));
        assert!(labs.contains(&"basename"));
        assert!(!labs.contains(&"stem"));

        match tab_action(&items, "b") {
            TabAction::FocusList => {}
            other => panic!("expected FocusList for bwf|basename, got {other:?}"),
        }

        // Longer shared prefix: ba… collapses to unique basename.
        let t2 = "${source.ba";
        let ctx2 = completion_context(t2, t2.len()).unwrap();
        let items2 = completion_items(&ctx2, &vars);
        assert_eq!(labels(&items2), vec!["basename"]);
        assert!(matches!(
            tab_action(&items2, "ba"),
            TabAction::Accept(CompletionItem {
                label,
                kind: CompletionKind::Variable,
                ..
            }) if label == "basename"
        ));
    }

    #[test]
    fn tab_inserts_common_prefix_when_longer() {
        let items = vec![
            CompletionItem {
                label: "basename".into(),
                kind: CompletionKind::Variable,
                value: "x".into(),
            },
            CompletionItem {
                label: "basepath".into(),
                kind: CompletionKind::Variable,
                value: "y".into(),
            },
        ];
        assert_eq!(
            tab_action(&items, "b"),
            TabAction::InsertPrefix("ase".into())
        );
        assert_eq!(tab_action(&items, "base"), TabAction::FocusList);
    }

    #[test]
    fn accept_scope_appends_dot() {
        let t = "${sou";
        let ctx = completion_context(t, t.len()).unwrap();
        let item = CompletionItem {
            label: "source".into(),
            kind: CompletionKind::Scope,
            value: String::new(),
        };
        let edit = accept_edit(&ctx, t, &item);
        assert_eq!(edit.replacement, "source.");
        assert_eq!(edit.caret_after, "source.".len());
    }

    #[test]
    fn accept_scope_skips_existing_dot() {
        let t = "${sou.bwf";
        // Caret in "sou" segment.
        let caret = "${sou".len();
        let ctx = completion_context(t, caret).unwrap();
        let item = CompletionItem {
            label: "source".into(),
            kind: CompletionKind::Scope,
            value: String::new(),
        };
        let edit = accept_edit(&ctx, t, &item);
        assert_eq!(edit.replacement, "source");
    }

    #[test]
    fn accept_variable_closes_unclosed_expression() {
        let t = "${title";
        let ctx = completion_context(t, t.len()).unwrap();
        let item = CompletionItem {
            label: "title".into(),
            kind: CompletionKind::Variable,
            value: "My Title".into(),
        };
        let edit = accept_edit(&ctx, t, &item);
        assert_eq!(edit.replacement, "title}");
    }

    #[test]
    fn accept_variable_keeps_closed_brace() {
        let t = "${tit}";
        let caret = "${tit".len();
        let ctx = completion_context(t, caret).unwrap();
        let item = CompletionItem {
            label: "title".into(),
            kind: CompletionKind::Variable,
            value: "My Title".into(),
        };
        let edit = accept_edit(&ctx, t, &item);
        assert_eq!(edit.replacement, "title");
    }

    #[test]
    fn resolved_table_omits_overridden_scoped_id() {
        let vars = sample_table();
        // After compose, Originator is user-scoped; source.bwf.Originator is gone.
        assert!(vars.get_qualified("source.bwf.Originator").is_none());
        assert_eq!(
            vars.get_by_name("Originator").map(|e| e.scope.as_str()),
            Some("user")
        );

        let t = "${source.bwf.";
        let ctx = completion_context(t, t.len()).unwrap();
        let items = completion_items(&ctx, &vars);
        let labs = labels(&items);
        // Description still under source.bwf; Originator was overridden away.
        assert!(labs.contains(&"Description"));
        assert!(!labs.contains(&"Originator"));
    }
}
