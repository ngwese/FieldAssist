// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! `field.variables` — Bindings, resolvers, and site resolution.
//!
//! `composition.variables` / `session.variables` are **single-scope** Bindings
//! (writable store). The Variables pane and export templates use the
//! **composition site** resolution (`source.*` + user + session + composition).
//! Call [`resolve_composition_variable`] / `composition:resolve_variable` to
//! look up a leaf (or scoped name) the same way the pane does.

use field_audio_io::{probe_source_variables, TechnicalSourceFields};
use field_composition::Composition;
use field_session::DocumentId;
use field_variables::{VariableEntry, VariableTable};
use mlua::{MultiValue, Table, UserData, UserDataFields, Value};
use std::path::Path;

use crate::bindings::{
    bindings_from_lua, live_user_bindings, split_readonly_by_scope, split_user_detached,
    LuaBindings,
};
use crate::host::host_from_lua;
use crate::prototype::{base_properties, create_prototype_table};

/// Registered resolver prototype.
pub struct ResolverDef {
    /// Resolver name (`:name()`).
    pub name: String,
    /// Prototype table.
    pub prototype: Table,
}

/// Base properties userdata for resolver prototypes.
#[derive(Clone, Debug)]
pub struct ResolverBaseProperties {
    /// Registered name.
    pub name: String,
}

impl UserData for ResolverBaseProperties {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |_, this| Ok(this.name.clone()));
    }
}

/// Create a resolver prototype table.
pub fn resolver_create_prototype(lua: &mlua::Lua, properties: Table) -> mlua::Result<Table> {
    let name: String = match properties.get::<Value>("name")? {
        Value::String(s) => {
            let n = s.to_str()?.to_owned();
            if n.is_empty() {
                return Err(mlua::Error::runtime("resolver name is required"));
            }
            n
        }
        Value::Nil => return Err(mlua::Error::runtime("resolver name is required")),
        other => {
            return Err(mlua::Error::runtime(format!(
                "resolver name must be a string, got {}",
                other.type_name()
            )))
        }
    };
    let proto = create_prototype_table(lua)?;
    proto.set(
        "__base_properties",
        ResolverBaseProperties { name: name.clone() },
    )?;
    proto.set(
        "name",
        lua.create_function(|_, this: Table| {
            Ok(base_properties::<ResolverBaseProperties>(&this)?.name)
        })?,
    )?;
    Ok(proto)
}

fn resolver_from_prototype(prototype: Table) -> mlua::Result<ResolverDef> {
    let meta = base_properties::<ResolverBaseProperties>(&prototype)?;
    if meta.name.is_empty() {
        return Err(mlua::Error::runtime("resolver prototype is missing a name"));
    }
    Ok(ResolverDef {
        name: meta.name,
        prototype,
    })
}

/// Construct a short-lived resolver instance and call `:init(bindings)`.
pub fn resolver_new_instance(
    lua: &mlua::Lua,
    prototype: Table,
    bindings: &[LuaBindings],
) -> mlua::Result<Table> {
    let instance = lua.create_table()?;
    instance.set_metatable(Some(prototype))?;
    let list = lua.create_table()?;
    for (i, b) in bindings.iter().enumerate() {
        list.set(i + 1, b.clone())?;
    }
    match instance.get::<Value>("init")? {
        Value::Nil => {}
        Value::Function(init) => {
            init.call::<()>((instance.clone(), list))?;
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "init must be a function, got {}",
                other.type_name()
            )))
        }
    }
    Ok(instance)
}

/// Resolve a bindings list with the active resolver into a [`VariableTable`].
///
/// Falls back to Rust last-wins compose when no resolver is registered.
pub fn resolve_with_active(
    lua: &mlua::Lua,
    bindings: &[LuaBindings],
) -> mlua::Result<VariableTable> {
    let host = host_from_lua(lua)?;
    let (prototype, has_resolver) = {
        let inner = host.inner.borrow();
        let name = inner.active_resolver.clone();
        match inner.resolvers.get(&name) {
            Some(def) => (Some(def.prototype.clone()), true),
            None => (None, false),
        }
    };
    if !has_resolver {
        return Ok(compose_bindings_fallback(lua, bindings)?);
    }
    let Some(prototype) = prototype else {
        return Ok(compose_bindings_fallback(lua, bindings)?);
    };
    let instance = resolver_new_instance(lua, prototype, bindings)?;
    let names = call_names(&instance)?;
    let mut out = VariableTable::new();
    for name in names {
        match call_resolve(lua, &instance, None, &name)? {
            Some((value, scope)) => {
                out.upsert(VariableEntry::new(scope, name, value));
            }
            None => {}
        }
    }
    Ok(out)
}

/// Exact scoped resolve via the active resolver (for `${scope.name}` templates).
#[allow(dead_code)]
pub fn resolve_qualified_with_active(
    lua: &mlua::Lua,
    bindings: &[LuaBindings],
    scope: &str,
    name: &str,
) -> mlua::Result<Option<String>> {
    Ok(resolve_one_with_active(lua, bindings, Some(scope), name)?.map(|(v, _)| v))
}

/// Resolve one leaf with the active resolver (`scope == None` → composed lookup).
pub fn resolve_one_with_active(
    lua: &mlua::Lua,
    bindings: &[LuaBindings],
    scope: Option<&str>,
    name: &str,
) -> mlua::Result<Option<(String, String)>> {
    let host = host_from_lua(lua)?;
    let prototype = {
        let inner = host.inner.borrow();
        let active = inner.active_resolver.clone();
        inner.resolvers.get(&active).map(|d| d.prototype.clone())
    };
    let Some(prototype) = prototype else {
        return Ok(resolve_one_fallback(lua, bindings, scope, name)?);
    };
    let instance = resolver_new_instance(lua, prototype, bindings)?;
    call_resolve(lua, &instance, scope, name)
}

fn resolve_one_fallback(
    lua: &mlua::Lua,
    bindings: &[LuaBindings],
    scope: Option<&str>,
    name: &str,
) -> mlua::Result<Option<(String, String)>> {
    for b in bindings.iter().rev() {
        if let Some(want) = scope {
            if b.scope() != want {
                continue;
            }
        }
        if let Some(v) = b.get_value_public(lua, name)? {
            return Ok(Some((v, b.scope().to_string())));
        }
    }
    Ok(None)
}

fn call_names(instance: &Table) -> mlua::Result<Vec<String>> {
    match instance.get::<Value>("names")? {
        Value::Function(func) => {
            let value: Value = func.call(instance.clone())?;
            match value {
                Value::Table(t) => {
                    let mut names = Vec::new();
                    for i in 1..=t.raw_len() {
                        match t.get::<Value>(i)? {
                            Value::String(s) => names.push(s.to_str()?.to_owned()),
                            Value::Nil => {}
                            other => {
                                return Err(mlua::Error::runtime(format!(
                                    "names() entries must be strings, got {}",
                                    other.type_name()
                                )))
                            }
                        }
                    }
                    Ok(names)
                }
                other => Err(mlua::Error::runtime(format!(
                    "names() must return a table, got {}",
                    other.type_name()
                ))),
            }
        }
        Value::Nil => Ok(Vec::new()),
        other => Err(mlua::Error::runtime(format!(
            "names must be a function, got {}",
            other.type_name()
        ))),
    }
}

fn call_resolve(
    lua: &mlua::Lua,
    instance: &Table,
    scope: Option<&str>,
    name: &str,
) -> mlua::Result<Option<(String, String)>> {
    match instance.get::<Value>("resolve")? {
        Value::Function(func) => {
            let scope_val = match scope {
                Some(s) => Value::String(lua.create_string(s)?),
                None => Value::Nil,
            };
            let results: MultiValue = func.call((instance.clone(), scope_val, name))?;
            let mut iter = results.into_iter();
            let value = match iter.next() {
                None | Some(Value::Nil) => return Ok(None),
                Some(Value::String(s)) => s.to_str()?.to_owned(),
                Some(other) => {
                    return Err(mlua::Error::runtime(format!(
                        "resolve() value must be a string, got {}",
                        other.type_name()
                    )))
                }
            };
            let resolved_scope = match iter.next() {
                None | Some(Value::Nil) => String::new(),
                Some(Value::String(s)) => s.to_str()?.to_owned(),
                Some(other) => {
                    return Err(mlua::Error::runtime(format!(
                        "resolve() scope must be a string, got {}",
                        other.type_name()
                    )))
                }
            };
            Ok(Some((value, resolved_scope)))
        }
        Value::Nil => Ok(None),
        other => Err(mlua::Error::runtime(format!(
            "resolve must be a function, got {}",
            other.type_name()
        ))),
    }
}

fn compose_bindings_fallback(
    lua: &mlua::Lua,
    bindings: &[LuaBindings],
) -> mlua::Result<VariableTable> {
    use field_variables::compose;
    let mut tables = Vec::new();
    for b in bindings {
        tables.push(b.to_table(lua)?);
    }
    Ok(compose(&tables))
}

/// Session site: user (including user.* sub-scopes), session.
pub fn session_site_bindings(
    lua: &mlua::Lua,
    detached_id: Option<field_session::SessionId>,
) -> mlua::Result<Vec<LuaBindings>> {
    let mut list = live_user_bindings(lua)?;
    list.push(LuaBindings::session(detached_id));
    Ok(list)
}

/// Composition site: source.* (r/o), user (including user.*), session, composition.
pub fn composition_site_bindings(
    lua: &mlua::Lua,
    source: &VariableTable,
    composition_id: Option<field_session::DocumentId>,
    session_id: Option<field_session::SessionId>,
) -> mlua::Result<Vec<LuaBindings>> {
    let mut list = split_readonly_by_scope(source);
    list.extend(live_user_bindings(lua)?);
    list.push(LuaBindings::session(session_id));
    if let Some(id) = composition_id {
        list.push(LuaBindings::composition(id));
    } else {
        // Detached composition table not live — pass empty r/w composition.
        list.push(LuaBindings::detached(
            "composition",
            VariableTable::new(),
            true,
        ));
    }
    Ok(list)
}

/// Export site: composition site + export profile bindings.
pub fn export_site_bindings(
    lua: &mlua::Lua,
    source: &VariableTable,
    composition_id: Option<field_session::DocumentId>,
    session_id: Option<field_session::SessionId>,
    export: &VariableTable,
) -> mlua::Result<Vec<LuaBindings>> {
    let mut list = composition_site_bindings(lua, source, composition_id, session_id)?;
    list.push(LuaBindings::detached("export", export.clone(), true));
    Ok(list)
}

/// Probe primary media into `source` / `source.*` variables (Variables pane / export).
pub fn composition_source_variables(composition: &Composition) -> VariableTable {
    let Some(media) = composition.primary_media() else {
        return VariableTable::new();
    };
    let tech = TechnicalSourceFields {
        basename: &media.basename,
        sample_rate: media.sample_rate,
        channel_count: media.channel_count,
        frame_count: media.frame_count,
        bits_per_sample: media.bits_per_sample,
        container_format: &media.container_format,
        codec: &media.codec,
    };
    if media.path.as_os_str().is_empty() {
        let mut table = VariableTable::new();
        table.upsert(VariableEntry::new(
            "source",
            "basename",
            media.basename.clone(),
        ));
        let stem = Path::new(&media.basename)
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        table.upsert(VariableEntry::new("source", "stem", stem));
        table.upsert(VariableEntry::new(
            "source",
            "sample_rate",
            media.sample_rate.to_string(),
        ));
        return table;
    }
    probe_source_variables(&media.path, Some(&tech))
}

/// Composition-scope table for compose / resolve: stored variables plus derived
/// leaves such as `channel_layout` (layout export `code`, or empty when unset).
///
/// Prefer [`composition_layer_variables_for_host`] when a layout registry is
/// available so `${channel_layout}` uses the filename-safe layout `code`.
pub fn composition_layer_variables(composition: &Composition) -> VariableTable {
    composition_layer_variables_with_channel_layout(
        composition,
        composition.channel_layout().unwrap_or(""),
    )
}

/// Like [`composition_layer_variables`], but resolve `channel_layout` through
/// the host layout registry (`code`, falling back to the stored layout name).
pub fn composition_layer_variables_for_host(
    host: &crate::host::HostHandle,
    composition: &Composition,
) -> VariableTable {
    let token = match composition.channel_layout() {
        Some(name) => host.layout_export_code(name),
        None => String::new(),
    };
    composition_layer_variables_with_channel_layout(composition, token)
}

fn composition_layer_variables_with_channel_layout(
    composition: &Composition,
    channel_layout: impl Into<String>,
) -> VariableTable {
    let mut table = composition.variables().clone();
    table.upsert(VariableEntry::new(
        "composition",
        "channel_layout",
        channel_layout.into(),
    ));
    table
}

/// Build a composition layer using an explicit `channel_layout` token (typically
/// the layout export `code`).
pub fn composition_layer_with_channel_layout_token(
    composition: &Composition,
    channel_layout: impl Into<String>,
) -> VariableTable {
    composition_layer_variables_with_channel_layout(composition, channel_layout)
}

/// Detached composition-site Bindings for an open document (Variables pane order).
pub fn composition_site_detached(
    lua: &mlua::Lua,
    id: DocumentId,
) -> mlua::Result<Vec<LuaBindings>> {
    let host = host_from_lua(lua)?;
    let (source, layout_name, composition_store) =
        crate::composition::with_document(lua, id, |doc| {
            let composition = doc.composition.read().unwrap();
            Ok((
                composition_source_variables(&composition),
                composition.channel_layout().map(str::to_string),
                composition.variables().clone(),
            ))
        })?;
    let token = match layout_name.as_deref() {
        Some(name) => host.layout_export_code(name),
        None => String::new(),
    };
    let mut composition_vars = composition_store;
    composition_vars.upsert(VariableEntry::new("composition", "channel_layout", token));
    let user = host.user_variables();
    let session = host.session_variables(None);
    let mut list = split_readonly_by_scope(&source);
    list.extend(split_user_detached(&user, true));
    list.push(LuaBindings::detached("session", session, true));
    list.push(LuaBindings::detached("composition", composition_vars, true));
    Ok(list)
}

/// Resolve one variable at the composition site (`scope == None` → composed leaf).
///
/// Returns `(value, resolved_scope)`, or `None` when unresolved.
pub fn resolve_composition_variable(
    lua: &mlua::Lua,
    id: DocumentId,
    scope: Option<&str>,
    name: &str,
) -> mlua::Result<Option<(String, String)>> {
    let bindings = composition_site_detached(lua, id)?;
    resolve_one_with_active(lua, &bindings, scope, name)
}

/// Install `field.variables`.
pub fn bind_variables_module(lua: &mlua::Lua, field: &Table) -> mlua::Result<()> {
    let variables = lua.create_table()?;
    variables.set(
        "create_bindings",
        lua.create_function(|_, scope: String| Ok(LuaBindings::create(scope)))?,
    )?;
    variables.set(
        "create_resolver",
        lua.create_function(|lua, properties: Table| resolver_create_prototype(lua, properties))?,
    )?;
    variables.set(
        "declare_resolver",
        lua.create_function(|lua, prototype: Table| {
            let def = resolver_from_prototype(prototype)?;
            let host = host_from_lua(lua)?;
            let mut inner = host.inner.borrow_mut();
            let name = def.name.clone();
            inner.resolvers.insert(name.clone(), def);
            if inner.active_resolver.is_empty() {
                inner.active_resolver = name;
            }
            Ok(())
        })?,
    )?;
    variables.set(
        "set_resolver",
        lua.create_function(|lua, name: String| {
            let host = host_from_lua(lua)?;
            let mut inner = host.inner.borrow_mut();
            if !inner.resolvers.contains_key(&name) {
                return Err(mlua::Error::runtime(format!("unknown resolver `{name}`")));
            }
            inner.active_resolver = name;
            Ok(())
        })?,
    )?;
    variables.set(
        "resolve",
        lua.create_function(|lua, list: Table| {
            let mut bindings = Vec::new();
            for i in 1..=list.raw_len() {
                bindings.push(bindings_from_lua(list.get(i)?)?);
            }
            let table = resolve_with_active(lua, &bindings)?;
            crate::util::variables_to_lua(lua, &table)
        })?,
    )?;
    // Convenience: live user bindings.
    variables.set(
        "user",
        lua.create_function(|_, ()| Ok(LuaBindings::user()))?,
    )?;
    variables.set(
        "load_resolvers",
        lua.create_function(|lua, ()| {
            let host = host_from_lua(lua)?;
            let config = host.inner.borrow().profile.config_dir.clone();
            crate::host::load_matching_scripts(
                lua,
                &host,
                config.as_deref(),
                crate::script_search::RESOLVER_PREFIX,
            )
            .map_err(mlua::Error::runtime)
        })?,
    )?;
    field.set("variables", variables)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::bindings::LuaBindings;
    use crate::host::{HostProfile, ScriptHost};
    use field_variables::{VariableEntry, VariableTable};

    fn host_with_resolver() -> ScriptHost {
        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: None,
        })
        .unwrap();
        host.load_init_from(None).unwrap();
        host
    }

    #[test]
    fn default_resolver_sees_user_subscopes() {
        use crate::bindings::split_user_detached;

        let mut host = host_with_resolver();
        let mut user = VariableTable::new();
        user.upsert(VariableEntry::new("user.ingest", "root_dir", "/data"));
        let bindings = split_user_detached(&user, true);
        host.lua()
            .globals()
            .set("bindings", {
                let t = host.lua().create_table().unwrap();
                for (i, b) in bindings.into_iter().enumerate() {
                    t.set(i + 1, b).unwrap();
                }
                t
            })
            .unwrap();
        let out = host.eval(
            r#"
            local rows = field.variables.resolve(bindings)
            local exact
            for _, b in ipairs(bindings) do
              if b:scope() == "user.ingest" then
                exact = b.values.root_dir
              end
            end
            return rows.root_dir, exact
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("/data\t/data"));
    }

    #[test]
    fn default_resolver_reads_env_scope() {
        let key = "FIELDASSIST_RESOLVER_ENV_TEST";
        let value = "from-env-resolver";
        // SAFETY: test-only unique key; no parallel test shares this name.
        unsafe { std::env::set_var(key, value) };
        let host = host_with_resolver();
        let bindings = [LuaBindings::create("user")];
        let got =
            crate::variables::resolve_one_with_active(host.lua(), &bindings, Some("env"), key)
                .unwrap();
        let missing = crate::variables::resolve_one_with_active(
            host.lua(),
            &bindings,
            Some("env"),
            "FIELDASSIST_ENV_MISSING_XYZ",
        )
        .unwrap();
        unsafe { std::env::remove_var(key) };
        assert_eq!(got, Some((value.to_string(), "env".to_string())));
        assert_eq!(missing, None);
    }

    #[test]
    fn create_bindings_rw_round_trip() {
        let mut host = host_with_resolver();
        let out = host.eval(
            r#"
            local b = field.variables.create_bindings("user")
            b.values.title = "Hello"
            return b:scope(), b.values.title, b:names()[1]
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("user\tHello\ttitle"));
    }

    #[test]
    fn readonly_bindings_reject_write() {
        let mut table = VariableTable::new();
        table.upsert(VariableEntry::new("source.bwf", "Originator", "BBC"));
        let b = LuaBindings::detached("source.bwf", table, false);
        let mut host = host_with_resolver();
        host.lua().globals().set("ro", b).unwrap();
        let out = host.eval(
            r#"
            local ok, err = pcall(function() ro.values.Originator = "X" end)
            return ok, tostring(err)
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        let result = out.result.unwrap();
        assert!(result.starts_with("false"), "{result}");
        assert!(result.contains("read-only"), "{result}");
    }

    #[test]
    fn declare_default_overrides_and_set_resolver() {
        let mut host = host_with_resolver();
        let out = host.eval(
            r#"
            local R = field.variables.create_resolver({ name = "default" })
            function R:init(bindings) self._b = bindings end
            function R:names() return { "ping" } end
            function R:resolve(scope, name)
              if name == "ping" then return "pong", "computed" end
              return nil, nil
            end
            field.variables.declare_resolver(R)
            field.variables.set_resolver("default")
            local b = field.variables.create_bindings("user")
            local rows = field.variables.resolve({ b })
            return rows.ping or rows[1].value
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("pong"));
    }

    #[test]
    fn set_resolver_unknown_errors() {
        let mut host = host_with_resolver();
        let out = host.eval(r#"field.variables.set_resolver("missing")"#);
        assert!(out.error.is_some());
        assert!(
            out.error.as_deref().unwrap().contains("unknown resolver"),
            "{:?}",
            out.error
        );
    }

    #[test]
    fn default_resolver_last_wins_per_site() {
        let mut host = host_with_resolver();
        let out = host.eval(
            r#"
            local user = field.variables.create_bindings("user")
            user.values.title = "from-user"
            local session = field.variables.create_bindings("session")
            session.values.title = "from-session"
            local composition = field.variables.create_bindings("composition")
            composition.values.title = "from-composition"
            local export = field.variables.create_bindings("export")
            export.values.title = "from-export"

            local session_site = field.variables.resolve({ user, session })
            local composition_site = field.variables.resolve({ user, session, composition })
            local export_site = field.variables.resolve({ user, session, composition, export })
            return session_site.title, composition_site.title, export_site.title
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(
            out.result.as_deref(),
            Some("from-session\tfrom-composition\tfrom-export")
        );
    }

    #[test]
    fn custom_resolver_today_and_operator() {
        let mut host = host_with_resolver();
        let out = host.eval(
            r#"
            local R = field.variables.create_resolver({ name = "custom" })
            function R:init(bindings) self._b = bindings end
            function R:names()
              local seen, out = {}, {}
              for _, b in ipairs(self._b) do
                for _, n in ipairs(b:names()) do
                  if not seen[n] then seen[n] = true; out[#out+1] = n end
                end
              end
              out[#out+1] = "today"
              out[#out+1] = "operator"
              return out
            end
            function R:resolve(scope, name)
              if name == "today" then return "2099-01-02", "computed" end
              if name == "operator" then
                for _, b in ipairs(self._b) do
                  if b:scope() == "source.ixml" and b.values.NOTE then
                    return b.values.NOTE, "source.ixml"
                  end
                end
                for _, b in ipairs(self._b) do
                  if b.values.artist then return b.values.artist, b:scope() end
                end
                return nil, nil
              end
              for i = #self._b, 1, -1 do
                local b = self._b[i]
                if scope == nil or b:scope() == scope then
                  local v = b.values[name]
                  if v ~= nil then return v, b:scope() end
                end
              end
              return nil, nil
            end
            field.variables.declare_resolver(R)
            field.variables.set_resolver("custom")

            local ixml = field.variables.create_bindings("source.ixml")
            ixml.values.NOTE = "Greg"
            local user = field.variables.create_bindings("user")
            local rows = field.variables.resolve({ ixml, user })
            return rows.today, rows.operator
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("2099-01-02\tGreg"));
    }

    #[test]
    fn resolve_variable_includes_source_not_on_composition_bindings() {
        use crate::backend::{BackendHandle, HeadlessBackend};
        use crate::host::HostProfile;
        use crate::world::HeadlessWorld;
        use std::cell::RefCell;
        use std::f32::consts::TAU;
        use std::io::Write;
        use std::rc::Rc;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("resolve_comp.wav");
        {
            let frames = 64u32;
            let sample_rate = 44_100u32;
            let channels: u16 = 2;
            let bits: u16 = 16;
            let block_align = channels * bits / 8;
            let byte_rate = sample_rate * u32::from(block_align);
            let data_len = frames * u32::from(block_align);
            let mut out = std::fs::File::create(&path).unwrap();
            out.write_all(b"RIFF").unwrap();
            out.write_all(&(36 + data_len).to_le_bytes()).unwrap();
            out.write_all(b"WAVE").unwrap();
            out.write_all(b"fmt ").unwrap();
            out.write_all(&16u32.to_le_bytes()).unwrap();
            out.write_all(&1u16.to_le_bytes()).unwrap();
            out.write_all(&channels.to_le_bytes()).unwrap();
            out.write_all(&sample_rate.to_le_bytes()).unwrap();
            out.write_all(&byte_rate.to_le_bytes()).unwrap();
            out.write_all(&block_align.to_le_bytes()).unwrap();
            out.write_all(&bits.to_le_bytes()).unwrap();
            out.write_all(b"data").unwrap();
            out.write_all(&data_len.to_le_bytes()).unwrap();
            for i in 0..frames {
                let t = i as f32 / sample_rate as f32;
                let sample = (0.5 * (TAU * 440.0 * t).sin() * i16::MAX as f32) as i16;
                for _ in 0..channels {
                    out.write_all(&sample.to_le_bytes()).unwrap();
                }
            }
        }

        let world = Rc::new(RefCell::new(HeadlessWorld::new()));
        let backend: BackendHandle =
            Rc::new(RefCell::new(HeadlessBackend::from_world_rc(world.clone())));
        let mut host = ScriptHost::with_backend(
            HostProfile {
                name: "field-batch",
                config_dir: None,
            },
            backend,
        )
        .unwrap();
        host.load_init_from(None).unwrap();
        world.borrow_mut().open_path(&path).expect("open");

        let out = host.eval(
            r#"
            local c = field.session.focused().composition
            c.variables.values.studio = "mine"
            local store_only = c.variables.values.basename
            local basename, basename_scope = c:resolve_variable("basename")
            local studio, studio_scope = c:resolve_variable("studio")
            local scoped = c:resolve_variable("source", "basename")
            return tostring(store_only), basename, basename_scope, studio, studio_scope, scoped
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(
            out.result.as_deref(),
            Some("nil\tresolve_comp.wav\tsource\tmine\tcomposition\tresolve_comp.wav")
        );
    }

    #[test]
    fn composition_layer_includes_channel_layout() {
        use super::composition_layer_variables;
        use field_composition::Composition;
        use std::collections::BTreeMap;

        let mut comp = Composition::new(48_000, 2);
        let empty = composition_layer_variables(&comp);
        assert_eq!(
            empty
                .get_by_name("channel_layout")
                .map(|e| e.value.as_str()),
            Some("")
        );
        let mut labels = BTreeMap::new();
        labels.insert(0, "L".into());
        labels.insert(1, "R".into());
        comp.apply_channel_layout(Some("Stereo".into()), labels);
        let layered = composition_layer_variables(&comp);
        assert_eq!(
            layered
                .get_by_name("channel_layout")
                .map(|e| e.value.as_str()),
            Some("Stereo")
        );
        assert_eq!(
            layered
                .get_by_name("channel_layout")
                .map(|e| e.scope.as_str()),
            Some("composition")
        );
    }
}
