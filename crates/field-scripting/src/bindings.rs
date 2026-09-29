// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Bindings userdata for `field.variables` (scope + values proxy).

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use field_session::DocumentId;
use field_session::SessionId;
use field_variables::{top_level_scope, VariableEntry, VariableTable};
use mlua::{MetaMethod, UserData, UserDataFields, UserDataMethods, UserDataRef, Value};

use crate::host::host_from_lua;

/// How Bindings storage is backed.
#[derive(Clone)]
pub enum BindingsStore {
    /// Detached table (r/w or r/o).
    Detached {
        table: Rc<RefCell<VariableTable>>,
        writable: bool,
    },
    /// Live session-scoped variables.
    Session { detached_id: Option<SessionId> },
    /// Live composition-scoped variables.
    Composition { id: DocumentId },
    /// Live user-scoped variables on the host.
    User,
}

/// Lua Bindings userdata: one scope's leaf name → value map.
#[derive(Clone)]
pub struct LuaBindings {
    scope: String,
    store: BindingsStore,
}

impl LuaBindings {
    /// Detached Bindings with the given scope and table.
    pub fn detached(scope: impl Into<String>, table: VariableTable, writable: bool) -> Self {
        Self {
            scope: scope.into(),
            store: BindingsStore::Detached {
                table: Rc::new(RefCell::new(table)),
                writable,
            },
        }
    }

    /// Empty detached r/w Bindings.
    pub fn create(scope: impl Into<String>) -> Self {
        Self::detached(scope, VariableTable::new(), true)
    }

    /// Live session Bindings.
    pub fn session(detached_id: Option<SessionId>) -> Self {
        Self {
            scope: "session".into(),
            store: BindingsStore::Session { detached_id },
        }
    }

    /// Live composition Bindings.
    pub fn composition(id: DocumentId) -> Self {
        Self {
            scope: "composition".into(),
            store: BindingsStore::Composition { id },
        }
    }

    /// Live user Bindings (host-held) for the top-level `user` scope.
    pub fn user() -> Self {
        Self::user_scope("user")
    }

    /// Live user Bindings for a specific scope path under `user` (e.g.
    /// `"user"` or `"user.ingest"`).
    pub fn user_scope(scope: impl Into<String>) -> Self {
        Self {
            scope: scope.into(),
            store: BindingsStore::User,
        }
    }

    /// Scope string.
    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// Whether writes are allowed.
    pub fn writable(&self) -> bool {
        match &self.store {
            BindingsStore::Detached { writable, .. } => *writable,
            BindingsStore::Session { .. }
            | BindingsStore::Composition { .. }
            | BindingsStore::User => true,
        }
    }

    /// Snapshot as a [`VariableTable`] (entries use this Bindings' scope).
    pub fn to_table(&self, lua: &mlua::Lua) -> mlua::Result<VariableTable> {
        match &self.store {
            BindingsStore::Detached { table, .. } => Ok(table.borrow().clone()),
            BindingsStore::Session { detached_id } => {
                Ok(host_from_lua(lua)?.session_variables(*detached_id))
            }
            BindingsStore::Composition { id } => host_from_lua(lua)?.composition_variables(*id),
            BindingsStore::User => Ok(host_from_lua(lua)?.user_variables()),
        }
    }

    /// Leaf names in insertion order.
    pub fn names(&self, lua: &mlua::Lua) -> mlua::Result<Vec<String>> {
        let table = self.to_table(lua)?;
        Ok(table
            .entries()
            .iter()
            .filter(|e| e.scope == self.scope || e.scope.is_empty())
            .map(|e| e.name.clone())
            .collect())
    }

    fn get_value(&self, lua: &mlua::Lua, name: &str) -> mlua::Result<Option<String>> {
        let table = self.to_table(lua)?;
        Ok(table
            .entries()
            .iter()
            .find(|e| e.name == name && (e.scope == self.scope || e.scope.is_empty()))
            .map(|e| e.value.clone()))
    }

    /// Public leaf lookup (used by resolver fallback).
    pub fn get_value_public(&self, lua: &mlua::Lua, name: &str) -> mlua::Result<Option<String>> {
        self.get_value(lua, name)
    }

    fn set_value(&self, lua: &mlua::Lua, name: &str, value: Option<&str>) -> mlua::Result<()> {
        if !self.writable() {
            return Err(mlua::Error::runtime(format!(
                "bindings for scope `{}` are read-only",
                self.scope
            )));
        }
        match &self.store {
            BindingsStore::Detached { table, .. } => {
                let mut t = table.borrow_mut();
                if let Some(value) = value {
                    t.upsert(VariableEntry::new(&self.scope, name, value));
                } else {
                    t.remove_in_scope(&self.scope, name);
                }
                Ok(())
            }
            BindingsStore::Session { detached_id } => {
                let host = host_from_lua(lua)?;
                let mut t = host.session_variables(*detached_id);
                if let Some(value) = value {
                    t.upsert(VariableEntry::new("session", name, value));
                } else {
                    t.remove_in_scope("session", name);
                }
                host.set_session_variables(*detached_id, t)
            }
            BindingsStore::Composition { id } => {
                let host = host_from_lua(lua)?;
                let mut t = host.composition_variables(*id)?;
                if let Some(value) = value {
                    t.upsert(VariableEntry::new("composition", name, value));
                } else {
                    t.remove_in_scope("composition", name);
                }
                host.set_composition_variables(*id, t)
            }
            BindingsStore::User => {
                let host = host_from_lua(lua)?;
                let mut t = host.user_variables();
                if let Some(value) = value {
                    t.upsert(VariableEntry::new(&self.scope, name, value));
                } else {
                    t.remove_in_scope(&self.scope, name);
                }
                host.set_user_variables(t);
                Ok(())
            }
        }
    }
}

/// Values proxy userdata (`bindings.values`).
#[derive(Clone)]
struct ValuesProxy {
    bindings: LuaBindings,
}

impl UserData for ValuesProxy {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::Index, |lua, this, key: Value| {
            let name = match key {
                Value::String(s) => s.to_str()?.to_owned(),
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "bindings.values key must be a string, got {}",
                        other.type_name()
                    )))
                }
            };
            match this.bindings.get_value(lua, &name)? {
                Some(v) => Ok(Value::String(lua.create_string(v)?)),
                None => Ok(Value::Nil),
            }
        });
        methods.add_meta_method(
            MetaMethod::NewIndex,
            |lua, this, (key, value): (Value, Value)| {
                let name = match key {
                    Value::String(s) => s.to_str()?.to_owned(),
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "bindings.values key must be a string, got {}",
                            other.type_name()
                        )))
                    }
                };
                match value {
                    Value::Nil => this.bindings.set_value(lua, &name, None),
                    Value::String(s) => {
                        this.bindings
                            .set_value(lua, &name, Some(s.to_str()?.as_ref()))
                    }
                    other => Err(mlua::Error::runtime(format!(
                        "bindings.values[{name}] must be a string or nil, got {}",
                        other.type_name()
                    ))),
                }
            },
        );
        methods.add_meta_method(MetaMethod::Pairs, |lua, this, ()| {
            // Return a next-style iterator over leaf names.
            let names = this.bindings.names(lua)?;
            let state = lua.create_table()?;
            state.set("names", names)?;
            state.set("i", 0i64)?;
            let bindings = this.bindings.clone();
            let iter = lua.create_function(move |lua, (st, _prev): (mlua::Table, Value)| {
                let i: i64 = st.get("i")?;
                let names: Vec<String> = st.get("names")?;
                let next = i as usize;
                if next >= names.len() {
                    return Ok((Value::Nil, Value::Nil));
                }
                st.set("i", i + 1)?;
                let name = &names[next];
                let value = bindings.get_value(lua, name)?.unwrap_or_default();
                Ok((
                    Value::String(lua.create_string(name)?),
                    Value::String(lua.create_string(value)?),
                ))
            })?;
            Ok((iter, state, Value::Nil))
        });
    }
}

impl UserData for LuaBindings {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("values", |_, this| {
            Ok(ValuesProxy {
                bindings: this.clone(),
            })
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("scope", |_, this, ()| Ok(this.scope.clone()));
        methods.add_method("names", |lua, this, ()| {
            let names = this.names(lua)?;
            let table = lua.create_table()?;
            for (i, name) in names.into_iter().enumerate() {
                table.set(i + 1, name)?;
            }
            Ok(table)
        });
    }
}

/// Distinct user scope paths present in `table`, always including `"user"`.
///
/// Non-`user` top-level scopes are ignored. Order is `"user"` first, then
/// remaining scopes in first-seen order.
pub fn user_scope_paths(table: &VariableTable) -> Vec<String> {
    let mut scopes = vec!["user".to_string()];
    let mut seen = BTreeSet::from(["user".to_string()]);
    for entry in table.entries() {
        let scope = if entry.scope.is_empty() {
            "user".to_string()
        } else {
            entry.scope.clone()
        };
        if top_level_scope(&scope) != "user" {
            continue;
        }
        if seen.insert(scope.clone()) {
            scopes.push(scope);
        }
    }
    scopes
}

/// One detached Bindings per user scope path in `table` (always includes
/// `"user"`), sharing the full table and filtering by Bindings scope.
pub fn split_user_detached(table: &VariableTable, writable: bool) -> Vec<LuaBindings> {
    user_scope_paths(table)
        .into_iter()
        .map(|scope| LuaBindings::detached(scope, table.clone(), writable))
        .collect()
}

/// Live host user Bindings, one per scope path currently in the user table.
pub fn live_user_bindings(lua: &mlua::Lua) -> mlua::Result<Vec<LuaBindings>> {
    let table = host_from_lua(lua)?.user_variables();
    Ok(user_scope_paths(&table)
        .into_iter()
        .map(LuaBindings::user_scope)
        .collect())
}

/// Split a multi-scope table into one Bindings per distinct scope (ingest order).
pub fn split_readonly_by_scope(table: &VariableTable) -> Vec<LuaBindings> {
    let mut order: Vec<String> = Vec::new();
    let mut by_scope: BTreeMap<String, VariableTable> = BTreeMap::new();
    for entry in table.entries() {
        let scope = if entry.scope.is_empty() {
            "source".to_string()
        } else {
            entry.scope.clone()
        };
        if !by_scope.contains_key(&scope) {
            order.push(scope.clone());
            by_scope.insert(scope.clone(), VariableTable::new());
        }
        by_scope.get_mut(&scope).unwrap().upsert(entry.clone());
    }
    order
        .into_iter()
        .filter_map(|scope| {
            let t = by_scope.remove(&scope)?;
            Some(LuaBindings::detached(scope, t, false))
        })
        .collect()
}

/// Parse Bindings userdata from a Lua value.
pub fn bindings_from_lua(value: Value) -> mlua::Result<LuaBindings> {
    match value {
        Value::UserData(ud) => {
            let b: UserDataRef<LuaBindings> = ud.borrow()?;
            Ok(b.clone())
        }
        other => Err(mlua::Error::runtime(format!(
            "expected bindings userdata, got {}",
            other.type_name()
        ))),
    }
}

/// Accept Bindings userdata or legacy table forms for session/composition assign.
pub fn variables_table_from_value(value: Value, scope: &str) -> mlua::Result<VariableTable> {
    match &value {
        Value::UserData(ud) => {
            if let Ok(b) = ud.borrow::<LuaBindings>() {
                // Clone table from detached store when possible without lua.
                match &b.store {
                    BindingsStore::Detached { table, .. } => {
                        let mut out = table.borrow().clone();
                        for e in out.entries_mut() {
                            if e.scope.is_empty() {
                                e.scope = scope.to_string();
                            }
                        }
                        return Ok(out);
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    crate::util::variables_from_lua(value, scope)
}
