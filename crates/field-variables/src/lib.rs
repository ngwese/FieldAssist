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

pub use complete::{
    accept_edit, completion_context, completion_items, prefix_edit, tab_action, AcceptEdit,
    CompletionContext, CompletionItem, CompletionKind, TabAction,
};
pub use compose::{compose, compose_with_order, DEFAULT_SCOPE_ORDER};
pub use interpolate::{interpolate, interpolate_strict, InterpolateError};
pub use table::{
    split_variable_id, top_level_scope, StoredVariable, VariableEntry, VariableTable,
    TOP_LEVEL_SCOPES,
};

/// Re-export serde for callers that store tables.
pub use serde;
