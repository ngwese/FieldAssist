// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Default display titles for break-out child compositions.

/// Strip a trailing `.facomp` (case-insensitive) from a display name.
fn strip_facomp_suffix(name: &str) -> &str {
    let trimmed = name.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() > 7 {
        let suffix = &trimmed[trimmed.len() - 7..];
        if suffix.eq_ignore_ascii_case(".facomp") {
            return &trimmed[..trimmed.len() - 7];
        }
    }
    trimmed
}

/// Parse `N-` / `N.M-` prefix from a parent display name.
///
/// Returns `(numeric_prefix, base)` when `name` matches
/// `^(\d+(?:\.\d+)*)-(.+)$` after stripping `.facomp`.
fn parse_break_out_prefix(name: &str) -> Option<(String, String)> {
    let stem = strip_facomp_suffix(name);
    let dash = stem.find('-')?;
    if dash == 0 {
        return None;
    }
    let (prefix, rest) = stem.split_at(dash);
    let base = &rest[1..];
    if base.is_empty() {
        return None;
    }
    if !prefix
        .split('.')
        .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    Some((prefix.to_string(), base.to_string()))
}

/// Next unique break-out child name from the parent's display name and sibling names.
///
/// - Parent `Interview` → `1-Interview`, `2-Interview`, …
/// - Parent `1-Interview` → `1.1-Interview`, `1.2-Interview`, …
/// - Parent without an `N-` / `N.M-` prefix is treated as the root case.
pub fn next_break_out_name(parent_name: &str, sibling_names: &[impl AsRef<str>]) -> String {
    let stem = strip_facomp_suffix(parent_name);
    let (child_prefix_base, base) = match parse_break_out_prefix(stem) {
        Some((prefix, base)) => (format!("{prefix}."), base),
        None => (String::new(), stem.to_string()),
    };

    let mut used = Vec::new();
    for sibling in sibling_names {
        let sibling_stem = strip_facomp_suffix(sibling.as_ref());
        let Some((prefix, sib_base)) = parse_break_out_prefix(sibling_stem) else {
            continue;
        };
        if sib_base != base {
            continue;
        }
        if child_prefix_base.is_empty() {
            // Root children: `N-base` where N has no dots.
            if !prefix.contains('.') {
                if let Ok(n) = prefix.parse::<u32>() {
                    used.push(n);
                }
            }
        } else if let Some(suffix) = prefix.strip_prefix(&child_prefix_base) {
            // Nested: `{parentPrefix}.{M}-base`
            if !suffix.is_empty() && !suffix.contains('.') {
                if let Ok(n) = suffix.parse::<u32>() {
                    used.push(n);
                }
            }
        }
    }
    used.sort_unstable();
    let mut next = 1u32;
    for n in used {
        if n == next {
            next += 1;
        } else if n > next {
            break;
        }
    }
    format!("{child_prefix_base}{next}-{base}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_parent_gets_n_prefix() {
        assert_eq!(
            next_break_out_name("Interview", &[] as &[&str]),
            "1-Interview"
        );
        assert_eq!(
            next_break_out_name("Interview", &["1-Interview"]),
            "2-Interview"
        );
        assert_eq!(
            next_break_out_name("My Mix", &["1-My Mix", "3-My Mix"]),
            "2-My Mix"
        );
    }

    #[test]
    fn nested_parent_extends_prefix() {
        assert_eq!(
            next_break_out_name("1-Interview", &[] as &[&str]),
            "1.1-Interview"
        );
        assert_eq!(
            next_break_out_name("1-Interview", &["1.1-Interview"]),
            "1.2-Interview"
        );
        assert_eq!(
            next_break_out_name("1.1-Interview", &[] as &[&str]),
            "1.1.1-Interview"
        );
    }

    #[test]
    fn strips_facomp_suffix() {
        assert_eq!(
            next_break_out_name("Interview.facomp", &[] as &[&str]),
            "1-Interview"
        );
        assert_eq!(
            next_break_out_name("1-Interview.facomp", &["1.1-Interview.facomp"]),
            "1.2-Interview"
        );
    }

    #[test]
    fn ignores_unrelated_sibling_names() {
        assert_eq!(
            next_break_out_name("Interview", &["1-Other", "notes"]),
            "1-Interview"
        );
        assert_eq!(
            next_break_out_name("1-Interview", &["2-Interview", "1.1-Other"]),
            "1.1-Interview"
        );
    }

    #[test]
    fn unmatched_parent_is_root_case() {
        assert_eq!(next_break_out_name("foo-bar", &[] as &[&str]), "1-foo-bar");
        assert_eq!(next_break_out_name("-odd", &[] as &[&str]), "1--odd");
        assert_eq!(next_break_out_name("1.2-", &[] as &[&str]), "1-1.2-");
    }
}
