// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

#![deny(missing_docs)]

//! # field-variables
//!
//! Scoped string variables with layered composition and `${…}` interpolation.
//!
//! Variable identifiers use `"<scope>[.<sub-scope>…].<name>"`. A bare `"<name>"`
//! (no `.`) is a resolved leaf name. The scope of a qualified id is everything
//! before the last `.`.
//!
//! Default compose order (last wins for a given leaf name):
//! `source` → `user` → `session` → `composition` → `export`.

mod complete;
mod compose;
mod interpolate;
mod table;
mod user_file;

pub use complete::{
    AcceptEdit, CompletionContext, CompletionItem, CompletionKind, TabAction, accept_edit,
    completion_context, completion_items, prefix_edit, tab_action,
};
pub use compose::{DEFAULT_SCOPE_ORDER, compose, compose_with_order};
pub use interpolate::{InterpolateError, MAX_INTERPOLATE_DEPTH, interpolate, interpolate_strict};
pub use table::{
    StoredVariable, TOP_LEVEL_SCOPES, VariableEntry, VariableTable, split_variable_id,
    top_level_scope,
};
pub use user_file::{
    USER_SCOPE, UserVariablesFile, VARIABLES_FORMAT_VERSION, VARIABLES_KIND,
    parse_user_variable_name, user_variable_display_name,
};

/// Re-export serde for callers that store tables.
pub use serde;
