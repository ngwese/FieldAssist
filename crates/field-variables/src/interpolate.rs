// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! `${var_expression}` string interpolation.

use thiserror::Error;

use crate::table::{split_variable_id, VariableTable};

/// Maximum nested expansions of `${…}` inside resolved values.
pub const MAX_INTERPOLATE_DEPTH: u32 = 32;

/// Failure while interpolating with missing variables forbidden.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum InterpolateError {
    /// `${expr}` could not be resolved.
    #[error("unresolved variable `${0}`")]
    Unresolved(String),
    /// Malformed `${` without closing `}`.
    #[error("unclosed variable interpolation starting at byte index {0}")]
    Unclosed(usize),
    /// Nested `${…}` expansion exceeded [`MAX_INTERPOLATE_DEPTH`].
    #[error("variable interpolation exceeded recursion limit of {MAX_INTERPOLATE_DEPTH}")]
    RecursionLimit,
}

/// Interpolate `template`, leaving unresolved `${…}` references unchanged.
///
/// Resolved values that themselves contain `${…}` are expanded recursively up
/// to [`MAX_INTERPOLATE_DEPTH`] levels. Unclosed `${` is left as literal text
/// from that point. Soft mode stops expanding when the recursion limit is hit
/// (inserts the unexpanded value).
pub fn interpolate(template: &str, vars: &VariableTable) -> String {
    match interpolate_inner(template, vars, false, 0) {
        Ok(s) => s,
        Err(InterpolateError::Unclosed(start)) => {
            // Soft mode: copy remainder literally.
            let mut out = interpolate_inner(&template[..start], vars, false, 0).unwrap_or_default();
            out.push_str(&template[start..]);
            out
        }
        Err(_) => template.to_string(),
    }
}

/// Interpolate `template`, erroring on unresolved references, unclosed `${`,
/// or recursion beyond [`MAX_INTERPOLATE_DEPTH`].
///
/// Resolved values that contain `${…}` are expanded recursively.
pub fn interpolate_strict(
    template: &str,
    vars: &VariableTable,
) -> Result<String, InterpolateError> {
    interpolate_inner(template, vars, true, 0)
}

fn interpolate_inner(
    template: &str,
    vars: &VariableTable,
    strict: bool,
    depth: u32,
) -> Result<String, InterpolateError> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    let mut offset = 0usize;
    while let Some(rel) = rest.find("${") {
        out.push_str(&rest[..rel]);
        let abs = offset + rel;
        let after = &rest[rel + 2..];
        let Some(end) = after.find('}') else {
            return Err(InterpolateError::Unclosed(abs));
        };
        let expr = &after[..end];
        match resolve(expr, vars) {
            Some(value) => {
                let expanded = expand_value(&value, vars, strict, depth)?;
                out.push_str(&expanded);
            }
            None if strict => {
                return Err(InterpolateError::Unresolved(expr.to_string()));
            }
            None => {
                out.push_str("${");
                out.push_str(expr);
                out.push('}');
            }
        }
        let consumed = rel + 2 + end + 1;
        offset += consumed;
        rest = &rest[consumed..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Expand `${…}` inside a resolved value, respecting the recursion limit.
fn expand_value(
    value: &str,
    vars: &VariableTable,
    strict: bool,
    depth: u32,
) -> Result<String, InterpolateError> {
    if !value.contains("${") {
        return Ok(value.to_string());
    }
    let next = depth + 1;
    if next > MAX_INTERPOLATE_DEPTH {
        if strict {
            return Err(InterpolateError::RecursionLimit);
        }
        // Soft: stop nesting; keep the raw value.
        return Ok(value.to_string());
    }
    interpolate_inner(value, vars, strict, next)
}

fn resolve(expr: &str, vars: &VariableTable) -> Option<String> {
    let expr = expr.trim();
    if expr.is_empty() {
        return None;
    }
    let (scope, name) = split_variable_id(expr);
    if scope.is_empty() {
        return vars.get_by_name(name).map(|e| e.value.clone());
    }
    // Virtual scope: process environment (not materialized into the table).
    if scope == "env" {
        return std::env::var(name).ok();
    }
    vars.get_qualified(expr)
        .map(|e| e.value.clone())
        .or_else(|| {
            vars.entries()
                .iter()
                .find(|e| e.scope == scope && e.name == name)
                .map(|e| e.value.clone())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compose, VariableEntry, VariableTable};

    fn composed() -> VariableTable {
        let mut source = VariableTable::new();
        source.upsert(VariableEntry::new("source", "basename", "take01"));
        source.upsert(VariableEntry::new("source.bwf", "Originator", "BBC"));
        let mut user = VariableTable::new();
        user.upsert(VariableEntry::new("user", "title", "My Title"));
        compose(&[source, user])
    }

    #[test]
    fn bare_and_scoped() {
        let vars = composed();
        assert_eq!(
            interpolate("${title}_${basename}.wav", &vars),
            "My Title_take01.wav"
        );
        assert_eq!(interpolate("by ${source.bwf.Originator}", &vars), "by BBC");
    }

    #[test]
    fn soft_leaves_unresolved() {
        let vars = VariableTable::new();
        assert_eq!(interpolate("x=${missing}", &vars), "x=${missing}");
    }

    #[test]
    fn strict_errors_on_missing() {
        let vars = VariableTable::new();
        assert!(matches!(
            interpolate_strict("${missing}", &vars),
            Err(InterpolateError::Unresolved(_))
        ));
    }

    #[test]
    fn strict_errors_on_unclosed() {
        let vars = VariableTable::new();
        assert!(matches!(
            interpolate_strict("hello ${foo", &vars),
            Err(InterpolateError::Unclosed(_))
        ));
    }

    #[test]
    fn env_scope_reads_process_environment() {
        let key = "FIELDASSIST_INTERPOLATE_ENV_TEST";
        let value = "from-env-interpolate";
        // SAFETY: test-only unique key; no parallel test shares this name.
        unsafe { std::env::set_var(key, value) };
        let vars = VariableTable::new();
        assert_eq!(
            interpolate(&format!("path=${{env.{key}}}"), &vars),
            format!("path={value}")
        );
        assert_eq!(
            interpolate_strict(&format!("${{env.{key}}}"), &vars).unwrap(),
            value
        );
        unsafe { std::env::remove_var(key) };
        assert_eq!(
            interpolate(&format!("x=${{env.{key}}}"), &vars),
            format!("x=${{env.{key}}}")
        );
        assert!(matches!(
            interpolate_strict(&format!("${{env.{key}}}"), &vars),
            Err(InterpolateError::Unresolved(_))
        ));
    }

    #[test]
    fn recursive_expands_nested_values() {
        let mut vars = VariableTable::new();
        vars.upsert(VariableEntry::new("user", "root", "/data"));
        vars.upsert(VariableEntry::new("user", "name", "take01"));
        vars.upsert(VariableEntry::new("user", "path", "${root}/${name}"));
        assert_eq!(interpolate("${path}.wav", &vars), "/data/take01.wav");
        assert_eq!(
            interpolate_strict("${path}.wav", &vars).unwrap(),
            "/data/take01.wav"
        );
    }

    #[test]
    fn recursive_chain_within_limit() {
        let mut vars = VariableTable::new();
        // depth 0 template → a → b → … → leaf (32 nested values = depth 32).
        vars.upsert(VariableEntry::new("user", "v0", "done"));
        for i in 1..=MAX_INTERPOLATE_DEPTH {
            vars.upsert(VariableEntry::new(
                "user",
                format!("v{i}"),
                format!("${{v{}}}", i - 1),
            ));
        }
        let top = format!("v{MAX_INTERPOLATE_DEPTH}");
        assert_eq!(
            interpolate_strict(&format!("${{{top}}}"), &vars).unwrap(),
            "done"
        );
    }

    #[test]
    fn recursive_limit_strict_errors() {
        let mut vars = VariableTable::new();
        vars.upsert(VariableEntry::new("user", "v0", "done"));
        for i in 1..=(MAX_INTERPOLATE_DEPTH + 1) {
            vars.upsert(VariableEntry::new(
                "user",
                format!("v{i}"),
                format!("${{v{}}}", i - 1),
            ));
        }
        let top = format!("v{}", MAX_INTERPOLATE_DEPTH + 1);
        assert_eq!(
            interpolate_strict(&format!("${{{top}}}"), &vars),
            Err(InterpolateError::RecursionLimit)
        );
    }

    #[test]
    fn recursive_limit_soft_stops() {
        let mut vars = VariableTable::new();
        // Cycle: soft mode should stop at the limit rather than hang.
        vars.upsert(VariableEntry::new("user", "a", "${b}"));
        vars.upsert(VariableEntry::new("user", "b", "${a}"));
        let out = interpolate("${a}", &vars);
        assert!(out.contains("${"), "{out}");
    }
}
