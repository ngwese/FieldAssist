// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! `${var_expression}` string interpolation.

use thiserror::Error;

use crate::table::{split_variable_id, VariableTable};

/// Failure while interpolating with missing variables forbidden.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum InterpolateError {
    /// `${expr}` could not be resolved.
    #[error("unresolved variable `${0}`")]
    Unresolved(String),
    /// Malformed `${` without closing `}`.
    #[error("unclosed variable interpolation starting at byte index {0}")]
    Unclosed(usize),
}

/// Interpolate `template`, leaving unresolved `${…}` references unchanged.
///
/// Unclosed `${` is left as literal text from that point.
pub fn interpolate(template: &str, vars: &VariableTable) -> String {
    match interpolate_inner(template, vars, false) {
        Ok(s) => s,
        Err(InterpolateError::Unclosed(start)) => {
            // Soft mode: copy remainder literally.
            let mut out = interpolate_inner(&template[..start], vars, false).unwrap_or_default();
            out.push_str(&template[start..]);
            out
        }
        Err(_) => template.to_string(),
    }
}

/// Interpolate `template`, erroring on unresolved references or unclosed `${`.
pub fn interpolate_strict(
    template: &str,
    vars: &VariableTable,
) -> Result<String, InterpolateError> {
    interpolate_inner(template, vars, true)
}

fn interpolate_inner(
    template: &str,
    vars: &VariableTable,
    strict: bool,
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
            Some(value) => out.push_str(&value),
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

fn resolve(expr: &str, vars: &VariableTable) -> Option<String> {
    let expr = expr.trim();
    if expr.is_empty() {
        return None;
    }
    let (scope, name) = split_variable_id(expr);
    if scope.is_empty() {
        vars.get_by_name(name).map(|e| e.value.clone())
    } else {
        vars.get_qualified(expr)
            .map(|e| e.value.clone())
            .or_else(|| {
                vars.entries()
                    .iter()
                    .find(|e| e.scope == scope && e.name == name)
                    .map(|e| e.value.clone())
            })
    }
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
}
