// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! User-scoped variables (`variables.json` next to `settings.json`).
//!
//! File format and parse helpers live in [`field_variables`]. This module wraps
//! them with the FieldAssist config path and the GPUI global store.

use field_variables::VariableTable;
use gpui_kit::{App, Global};

pub use field_variables::{
    parse_user_variable_name, user_variable_display_name, UserVariablesFile, USER_SCOPE,
};

/// Path to `variables.json` beside `settings.json`, if the config dir resolves.
pub fn path() -> Option<std::path::PathBuf> {
    UserVariablesFile::path_in(crate::commands::user_config_dir().as_deref())
}

/// Load from disk, or defaults when missing/invalid.
pub fn load() -> UserVariablesFile {
    match path() {
        Some(p) => UserVariablesFile::load_from_path(&p),
        None => UserVariablesFile::default(),
    }
}

/// Write pretty JSON to the config directory (creating it if needed).
pub fn save(file: &UserVariablesFile) -> Result<(), String> {
    file.save_to_dir(crate::commands::user_config_dir().as_deref())
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
    store_mut(cx).file = load();
}

/// Persist the in-memory user variables.
#[allow(dead_code)]
pub fn save_store(cx: &App) -> Result<(), String> {
    save(&store(cx).file)
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
    save(&file)?;
    store_mut(cx).file = file;
    Ok(table)
}
