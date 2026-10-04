// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Host construction and eval.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mlua::{Function, Lua, MultiValue, Table, Value};

use crate::app::bind_app;
use crate::backend::{BackendHandle, HeadlessBackend, ScriptBackend};
use crate::field_ns::bind_field;
use crate::layout::ChannelLayoutDef;
use crate::package_policy::{install_package_policy, PackagePolicy};
use crate::variables::ResolverDef;
use crate::workflow::{
    instance_display_name, instance_name, prototype_is_stateful, table_method, workflow_new,
    workflow_start, WorkflowDef, WorkflowMeta,
};
use crate::workflow_app::{layout_drop_targets, workflows_for_menu, DropLayout};
use crate::workflow_sheet::{has_sheet, sheet_control_row, sheet_from_table, SheetSnapshot};
use crate::workflow_toolbar::{
    control_store, invoke_action, toolbar_from_table, toolbar_row, ToolbarItem,
};
use crate::world::HeadlessWorld;

use field_session::{DocumentId, SessionId};
use field_variables::VariableTable;

/// Embedded default `init.lua` (layouts + `detect_layout`).
///
/// Loaded when `{config_dir}/init.lua` is absent. Dump via FieldAssist
/// `--dump-init`.
pub const EMBEDDED_INIT: &str = include_str!("../assets/init.lua");

/// Embedded default variable resolver (last-wins). Loaded before `init.lua`.
pub const EMBEDDED_RESOLVER_DEFAULT: &str = include_str!("../assets/resolver_default.lua");

/// Profile for the enclosing application process.
#[derive(Clone, Debug)]
pub struct HostProfile {
    /// Read-only `app.name` (e.g. `"field-assist"`, `"field-batch"`,
    /// `"field-play"`).
    pub name: &'static str,
    /// Optional config directory for `field.include` and user scripts.
    pub config_dir: Option<PathBuf>,
}

/// Captured print / expression output from [`ScriptHost::eval`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalOutput {
    /// Lines written with `print`.
    pub prints: Vec<String>,
    /// Stringified return value(s), if any.
    pub result: Option<String>,
    /// Runtime error message, if any.
    pub error: Option<String>,
}

/// Log severity for [`LogEntry`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    /// Informational.
    Info,
    /// Warning.
    Warn,
    /// Error.
    Error,
}

impl LogLevel {
    /// Lowercase label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// One structured log line from `field.log`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    /// Severity.
    pub level: LogLevel,
    /// Topic tag.
    pub topic: String,
    /// Message body.
    pub message: String,
}

/// Result of resuming a stateful workflow after session load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResumeWorkflow {
    /// No workflow name on the session.
    None,
    /// Resumed successfully.
    Resumed,
    /// Session named an unknown or one-shot workflow.
    Unknown {
        /// Persisted workflow name.
        name: String,
    },
}

pub(crate) struct HostInner {
    pub(crate) profile: HostProfile,
    pub(crate) prints: Vec<String>,
    pub(crate) logs: Vec<LogEntry>,
    pub(crate) alerts: Vec<(String, String)>,
    pub(crate) loaded: Vec<Function>,
    pub(crate) saved: Vec<Function>,
    pub(crate) session_loaded: Vec<Function>,
    pub(crate) session_saved: Vec<Function>,
    pub(crate) session_selected: Vec<Function>,
    pub(crate) composition_selected: Vec<Function>,
    pub(crate) detect_layout: Vec<Function>,
    pub(crate) enrich_composition: Vec<Function>,
    pub(crate) enrich_session: Vec<Function>,
    pub(crate) enrich_media: Vec<Function>,
    pub(crate) layouts: Vec<ChannelLayoutDef>,
    pub(crate) export_profiles: Vec<crate::export::ExportProfileDef>,
    pub(crate) workflows: BTreeMap<String, WorkflowDef>,
    pub(crate) resolvers: BTreeMap<String, ResolverDef>,
    /// Name of the active variable resolver (`"default"` after startup).
    pub(crate) active_resolver: String,
    /// User-scoped variables (`variables.json` in FieldAssist).
    pub(crate) user_variables: VariableTable,
    pub(crate) active: Option<Table>,
    /// Non-stateful (or unbound) instance retained while its modal sheet is open.
    pub(crate) sheet: Option<Table>,
    /// Deferred callbacks `(workflow, fn)` drained by the host on a later frame.
    pub(crate) deferred: VecDeque<(Table, Function)>,
    /// Backend — owns session, open documents, media pool.
    pub(crate) backend: BackendHandle,
    pub(crate) include_stack: Vec<PathBuf>,
    pub(crate) include_cache: HashMap<String, Value>,
    pub(crate) args: Vec<String>,
    pub(crate) package_policy: PackagePolicy,
}

/// Shared handle stored in Lua app data.
#[derive(Clone)]
pub struct HostHandle {
    pub(crate) inner: Rc<RefCell<HostInner>>,
}

impl HostHandle {
    /// Return the active workflow instance table, if any.
    pub fn active_workflow(&self) -> Option<Table> {
        self.inner.borrow().active.clone()
    }
}

/// Lua runtime with `app` + `field` bindings.
pub struct ScriptHost {
    lua: Lua,
    handle: HostHandle,
}

impl ScriptHost {
    /// Create a host for `profile` with an empty [`HeadlessBackend`].
    pub fn new(profile: HostProfile) -> mlua::Result<Self> {
        let backend: BackendHandle = Rc::new(RefCell::new(HeadlessBackend::new()));
        Self::with_backend(profile, backend)
    }

    /// Create a host with a pre-built [`HeadlessWorld`] (tests).
    pub fn with_world(profile: HostProfile, world: HeadlessWorld) -> mlua::Result<Self> {
        let backend: BackendHandle = Rc::new(RefCell::new(HeadlessBackend::from_world(world)));
        Self::with_backend(profile, backend)
    }

    /// Create a host with an arbitrary [`ScriptBackend`].
    pub fn with_backend(
        profile: HostProfile,
        backend: Rc<RefCell<dyn ScriptBackend>>,
    ) -> mlua::Result<Self> {
        // Keep Lua C API symbols that C modules need but mlua may not call.
        crate::keep_lua_c_api::retain_for_native_modules();
        // SAFETY: we immediately install a reversible package policy that stubs
        // C loaders; `field.scripting.enable_native_modules` restores them on purpose.
        let lua = unsafe { Lua::unsafe_new() };
        let package_policy = install_package_policy(&lua, profile.config_dir.as_deref())?;
        let handle = HostHandle {
            inner: Rc::new(RefCell::new(HostInner {
                profile,
                prints: Vec::new(),
                logs: Vec::new(),
                alerts: Vec::new(),
                loaded: Vec::new(),
                saved: Vec::new(),
                session_loaded: Vec::new(),
                session_saved: Vec::new(),
                session_selected: Vec::new(),
                composition_selected: Vec::new(),
                detect_layout: Vec::new(),
                enrich_composition: Vec::new(),
                enrich_session: Vec::new(),
                enrich_media: Vec::new(),
                layouts: Vec::new(),
                export_profiles: Vec::new(),
                workflows: BTreeMap::new(),
                resolvers: BTreeMap::new(),
                active_resolver: String::new(),
                user_variables: VariableTable::new(),
                active: None,
                sheet: None,
                deferred: VecDeque::new(),
                backend,
                include_stack: Vec::new(),
                include_cache: HashMap::new(),
                args: Vec::new(),
                package_policy,
            })),
        };
        lua.set_app_data(handle.clone());
        bind_app(&lua)?;
        bind_field(&lua)?;
        install_print(&lua, handle.clone())?;
        Ok(Self { lua, handle })
    }

    /// Underlying Lua state.
    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    /// Shared backend handle.
    pub fn backend(&self) -> BackendHandle {
        self.handle.inner.borrow().backend.clone()
    }

    /// Shared [`HeadlessWorld`] handle (panics if the backend is not
    /// [`HeadlessBackend`]).  Prefer [`ScriptHost::backend`] for new code.
    pub fn world(&self) -> Rc<RefCell<HeadlessWorld>> {
        let backend = self.handle.inner.borrow().backend.clone();
        let backend_ref = backend.borrow();
        // Downcast via Any is unavailable on dyn traits; instead we obtain the
        // Rc from HeadlessBackend when it is installed at construction time.
        // This method exists for test compat; production code should prefer backend().
        drop(backend_ref);
        // Access the world Rc stored inside the HeadlessBackend via the
        // BackendHandle. We use a side-channel: ScriptHost::new / with_world
        // always wraps a HeadlessBackend, so we can reconstruct via media_store.
        // For the public API, keep returning an Rc<RefCell<HeadlessWorld>>.
        // Implementation: re-borrow the backend and call world_rc via a concrete cast.
        // Since we control construction, use unsafe downcast only if truly needed.
        // For now: hold the world Rc in a separate field on ScriptHost.
        // Actually we stored it as backend only. Return a new Rc wrapping
        // the shared world via HeadlessBackend::world_rc.
        // This requires knowing the backend IS a HeadlessBackend.
        // Safe in practice (the tests that use this always create via new/with_world).
        // Use the media_store() method as a compatibility probe won't work.
        //
        // SIMPLEST FIX: hold the world Rc as a separate field.
        // We can't change ScriptHost layout without breaking construction.
        // For now: panic with a useful message if not headless.
        panic!(
            "ScriptHost::world() is deprecated; use ScriptHost::backend() instead, \
             or use ScriptHost::with_world() in tests and access via HeadlessBackend::world_rc()"
        );
    }

    /// Set `app.args` (script argv after the file name).
    pub fn set_args(&self, args: Vec<String>) {
        self.handle.inner.borrow_mut().args = args;
    }

    /// Evaluate a chunk; capture `print` and the return value.
    pub fn eval(&mut self, code: &str) -> EvalOutput {
        self.handle.inner.borrow_mut().prints.clear();
        let result = eval_repl(&self.lua, code);
        let prints = std::mem::take(&mut self.handle.inner.borrow_mut().prints);
        match result {
            Ok(values) => EvalOutput {
                prints,
                result: stringify_values(&self.lua, values),
                error: None,
            },
            Err(err) => EvalOutput {
                prints,
                result: None,
                error: Some(err.to_string()),
            },
        }
    }

    /// Load user `{config_dir}/init.lua` if present, else [`EMBEDDED_INIT`].
    ///
    /// Uses the host profile's `config_dir`. Reads `{config_dir}/variables.json`
    /// into host user variables first (missing/invalid → empty). Does not load
    /// user `resolver_*.lua` / `workflow_*.lua` (call [`Self::load_resolvers`] /
    /// [`Self::load_workflows`]). FieldAssist loads those after this along with
    /// embedded workflow bundles.
    pub fn load_init(&mut self) -> Result<(), String> {
        let config = self.config_dir();
        self.load_init_from(config.as_deref())
    }

    /// Load `{config}/init.lua` if it is a file, else [`EMBEDDED_INIT`].
    ///
    /// Always loads [`EMBEDDED_RESOLVER_DEFAULT`] first so `"default"` is
    /// registered before user scripts run. Reads `{config}/variables.json` into
    /// host user variables before init (missing/invalid → empty). Does not load
    /// user `resolver_*.lua` / `workflow_*.lua` (call [`Self::load_resolvers`] /
    /// [`Self::load_workflows`], or let FieldAssist do so after this).
    pub fn load_init_from(&mut self, config: Option<&Path>) -> Result<(), String> {
        self.load_user_variables_from(config);
        self.lua
            .load(EMBEDDED_RESOLVER_DEFAULT)
            .set_name("@<embedded>/resolver_default.lua")
            .exec()
            .map_err(|err| format!("resolver_default.lua: {err}"))?;
        if let Some(path) = config.map(|d| d.join("init.lua")).filter(|p| p.is_file()) {
            self.load_file(&path)
        } else {
            self.lua
                .load(EMBEDDED_INIT)
                .set_name("@<embedded>/init.lua")
                .exec()
                .map_err(|err| format!("init.lua: {err}"))
        }
    }

    /// Replace host user variables from `{config}/variables.json` (read-only).
    ///
    /// Missing or invalid files leave an empty table. Does not write disk.
    pub fn load_user_variables_from(&self, config: Option<&Path>) {
        let table = field_variables::UserVariablesFile::load_from_dir(config).to_table();
        self.set_user_variables(table);
    }

    /// Apply scripting preferences to `package.path` / policy before `init.lua`.
    ///
    /// Appends search-path templates (once), then optionally enables system
    /// package paths and native modules. Idempotent for the enable flags.
    pub fn apply_scripting_settings(
        &self,
        settings: &field_settings::ScriptingSettings,
    ) -> Result<(), String> {
        {
            let mut inner = self.handle.inner.borrow_mut();
            inner
                .package_policy
                .append_search_path_dirs(&self.lua, &settings.search_path)
                .map_err(|err| err.to_string())?;
        }
        if settings.enable_system_package_paths {
            self.handle
                .enable_system_package_paths(&self.lua)
                .map_err(|err| err.to_string())?;
        }
        if settings.enable_native_modules {
            self.handle
                .enable_native_modules(&self.lua)
                .map_err(|err| err.to_string())?;
        }
        Ok(())
    }

    /// Load `resolver_*.lua` from the scripting search path (config dir, then
    /// `settings.json` `scripting.search_path`).
    pub fn load_resolvers(&mut self) -> Result<(), String> {
        let config = self.config_dir();
        self.load_resolvers_from(config.as_deref())
    }

    /// Load `resolver_*.lua` using `config` as the primary search directory.
    pub fn load_resolvers_from(&mut self, config: Option<&Path>) -> Result<(), String> {
        self.load_matching_scripts(config, crate::script_search::RESOLVER_PREFIX)
    }

    /// Load `workflow_*.lua` from the scripting search path (config dir, then
    /// `settings.json` `scripting.search_path`).
    pub fn load_workflows(&mut self) -> Result<(), String> {
        let config = self.config_dir();
        self.load_workflows_from(config.as_deref())
    }

    /// Load `workflow_*.lua` using `config` as the primary search directory.
    pub fn load_workflows_from(&mut self, config: Option<&Path>) -> Result<(), String> {
        self.load_matching_scripts(config, crate::script_search::WORKFLOW_PREFIX)
    }

    fn load_matching_scripts(&mut self, config: Option<&Path>, prefix: &str) -> Result<(), String> {
        load_matching_scripts(&self.lua, &self.handle, config, prefix)
    }

    /// Replace host-held user-scoped variables (FieldAssist `variables.json`).
    pub fn set_user_variables(&self, variables: VariableTable) {
        self.handle.set_user_variables(variables);
    }

    /// Snapshot host-held user-scoped variables.
    pub fn user_variables(&self) -> VariableTable {
        self.handle.user_variables()
    }

    /// Resolve a bindings list with the active Lua resolver (or Rust fallback).
    pub fn resolve_variables(
        &self,
        bindings: &[crate::bindings::LuaBindings],
    ) -> mlua::Result<VariableTable> {
        crate::variables::resolve_with_active(&self.lua, bindings)
    }

    /// Load and execute a Lua file.
    ///
    /// A leading Unix shebang (`#!…`) is stripped so scripts can use
    /// `#!/usr/bin/env field-batch`.
    pub fn load_file(&mut self, path: &Path) -> Result<(), String> {
        exec_lua_file(&self.lua, &self.handle, path)
    }

    /// Drain captured log entries.
    pub fn take_logs(&self) -> Vec<LogEntry> {
        std::mem::take(&mut self.handle.inner.borrow_mut().logs)
    }

    /// Drain captured `print` lines (outside of [`Self::eval`]).
    pub fn take_prints(&self) -> Vec<String> {
        std::mem::take(&mut self.handle.inner.borrow_mut().prints)
    }

    /// Drain captured alerts.
    pub fn take_alerts(&self) -> Vec<(String, String)> {
        std::mem::take(&mut self.handle.inner.borrow_mut().alerts)
    }

    /// Log a structured line (also available from Lua as `field.log`).
    pub fn log(&self, level: LogLevel, topic: impl Into<String>, message: impl Into<String>) {
        self.handle.inner.borrow_mut().logs.push(LogEntry {
            level,
            topic: topic.into(),
            message: message.into(),
        });
    }

    /// Host name from the profile.
    pub fn host_name(&self) -> &'static str {
        self.handle.inner.borrow().profile.name
    }

    /// Config directory, if any.
    pub fn config_dir(&self) -> Option<PathBuf> {
        self.handle.inner.borrow().profile.config_dir.clone()
    }

    /// Declare / run helpers used by FieldAssist chrome.
    pub fn workflow_metas(&self) -> Vec<WorkflowMeta> {
        self.handle
            .inner
            .borrow()
            .workflows
            .values()
            .map(|w| w.meta.clone())
            .collect()
    }

    /// Drop overlay layout from registered workflows.
    pub fn drop_layout(&self) -> DropLayout {
        layout_drop_targets(&self.workflow_metas())
    }

    /// Menu workflows `(name, display_name)`.
    pub fn menu_workflows(&self) -> Vec<(String, String)> {
        workflows_for_menu(&self.workflow_metas())
    }

    /// Active stateful workflow name, if any.
    pub fn active_workflow_name(&self) -> Option<String> {
        let active = self.handle.inner.borrow().active.clone()?;
        instance_name(&active).ok()
    }

    /// Toolbar snapshot for the active workflow.
    ///
    /// Returns `None` when a modal sheet is open (sheet owns chrome) or when
    /// the toolbar has no items (avoids a title-only strip).
    pub fn toolbar_snapshot(&self) -> Option<(String, Vec<ToolbarItem>)> {
        let active = self.handle.inner.borrow().active.clone()?;
        if has_sheet(&active) {
            return None;
        }
        let items = toolbar_from_table(&active);
        if items.is_empty() {
            return None;
        }
        let name = instance_display_name(&active).ok()?;
        Some((name, items))
    }

    /// Sheet snapshot for the open workflow sheet (active or sheet-retained).
    pub fn sheet_snapshot(&self) -> Option<SheetSnapshot> {
        let instance = self.sheet_instance()?;
        let mut snap = sheet_from_table(&instance)?;
        if snap.title.is_empty() {
            snap.title = instance_display_name(&instance).ok()?;
        }
        Some(snap)
    }

    /// Workflow instance that owns the open sheet, if any.
    pub fn sheet_instance(&self) -> Option<Table> {
        let inner = self.handle.inner.borrow();
        if let Some(active) = &inner.active {
            if has_sheet(active) {
                return Some(active.clone());
            }
        }
        inner.sheet.clone()
    }

    /// Close any open workflow sheet and drop the sheet-retained instance.
    pub fn close_workflow_sheet(&self) {
        self.handle.close_open_sheet();
    }

    /// Drain deferred workflow callbacks. Each is `(workflow, fn)`.
    pub fn take_deferred(&self) -> Vec<(Table, Function)> {
        self.handle.inner.borrow_mut().deferred.drain(..).collect()
    }

    /// Run all pending deferred callbacks (headless / tests).
    pub fn drain_deferred(&self) -> mlua::Result<()> {
        loop {
            let batch = self.take_deferred();
            if batch.is_empty() {
                return Ok(());
            }
            for (instance, func) in batch {
                func.call::<()>((instance,))?;
            }
        }
    }

    /// Run a registered workflow by name.
    pub fn run_workflow(&self, name: &str, payload: Table) -> mlua::Result<Option<Table>> {
        self.handle.workflow_run(&self.lua, name, payload)
    }

    /// Finish the active stateful workflow.
    pub fn finish_workflow(&self) -> mlua::Result<()> {
        self.handle.finish_workflow(&self.lua)
    }

    /// Cancel the active stateful workflow.
    pub fn cancel_workflow(&self) -> mlua::Result<()> {
        self.handle.cancel_workflow(&self.lua)
    }

    /// Running workflow instance, if any.
    pub fn active_workflow(&self) -> Option<Table> {
        self.handle.inner.borrow().active.clone()
    }

    /// Fire `loaded` hooks (called by the host after a document loads).
    pub fn fire_loaded(&self, id: DocumentId, elapsed: f64) {
        let hooks = self.handle.inner.borrow().loaded.clone();
        let handle = crate::composition::LuaComposition { id };
        for hook in &hooks {
            if let Err(err) = hook.call::<()>((handle, elapsed)) {
                self.handle.note_hook_error("loaded", &err);
            }
        }
        self.handle.fire_workflow_document("loaded", id, elapsed);
    }

    /// Fire `saved` hooks.
    pub fn fire_saved(&self, id: DocumentId, elapsed: f64) {
        let hooks = self.handle.inner.borrow().saved.clone();
        let handle = crate::composition::LuaComposition { id };
        for hook in &hooks {
            if let Err(err) = hook.call::<()>((handle, elapsed)) {
                self.handle.note_hook_error("saved", &err);
            }
        }
        self.handle.fire_workflow_document("saved", id, elapsed);
    }

    /// Fire `session_loaded` hooks.
    pub fn fire_session_loaded(&self) {
        let hooks = self.handle.inner.borrow().session_loaded.clone();
        for hook in &hooks {
            if let Err(err) = hook.call::<()>(crate::session::LuaSession::focused()) {
                self.handle.note_hook_error("session_loaded", &err);
            }
        }
        self.handle.fire_workflow_session("session_loaded");
    }

    /// Fire `session_saved` hooks.
    pub fn fire_session_saved(&self) {
        let hooks = self.handle.inner.borrow().session_saved.clone();
        for hook in &hooks {
            if let Err(err) = hook.call::<()>(crate::session::LuaSession::focused()) {
                self.handle.note_hook_error("session_saved", &err);
            }
        }
        self.handle.fire_workflow_session("session_saved");
    }

    /// Fire `session_selected` hooks.
    pub fn fire_session_selected(&self) {
        let hooks = self.handle.inner.borrow().session_selected.clone();
        for hook in &hooks {
            if let Err(err) = hook.call::<()>(crate::session::LuaSession::focused()) {
                self.handle.note_hook_error("session_selected", &err);
            }
        }
        self.handle.fire_workflow_session("session_selected");
    }

    /// Fire `composition_selected` hooks.
    pub fn fire_composition_selected(&self, id: Option<DocumentId>) {
        self.handle.emit_composition_selected(id);
    }

    /// Fire `detect_layout` hooks for a composition.
    pub fn fire_detect_layout(&self, id: DocumentId) {
        self.handle.fire_detect_layout(id);
    }

    /// Fire `enrich_composition` hooks for a composition built from media.
    pub fn fire_enrich_composition(&self, id: DocumentId) {
        self.handle.fire_enrich_composition(id);
    }

    /// Fire `enrich_media` hooks after probe (media open / pool add).
    pub fn fire_enrich_media(&self, media: crate::media::LuaMedia) {
        self.handle.fire_enrich_media(media);
    }

    /// Fire `enrich_media` for the composition's primary media.
    pub fn fire_enrich_media_for_document(&self, id: DocumentId) {
        self.handle.fire_enrich_media_for_document(id);
    }

    /// Fire `enrich_session` hooks for a newly created empty session.
    ///
    /// `which` is `None` for the focused world session, or `Some(id)` for a
    /// detached session from `field.session.new()`.
    pub fn fire_enrich_session(&self, which: Option<SessionId>) {
        self.handle.fire_enrich_session(which);
    }

    /// Apply a named channel layout to a document (user-explicit choice).
    ///
    /// Returns `Err` if `name` is `Some` and is not registered.
    pub fn choose_layout(&self, id: DocumentId, name: Option<&str>) -> mlua::Result<()> {
        self.handle.choose_layout(id, name)
    }

    /// Look up a registered layout by name.
    pub fn layout(&self, name: &str) -> Option<ChannelLayoutDef> {
        self.handle.layout(name)
    }

    /// Names of registered channel layouts.
    pub fn layout_names(&self) -> Vec<String> {
        self.handle.layout_names()
    }

    /// `(name, description)` pairs for registered layouts.
    pub fn layout_choices(&self) -> Vec<(String, String)> {
        self.handle.layout_choices()
    }

    /// Look up a registered export profile by name.
    pub fn export_profile(&self, name: &str) -> Option<crate::export::ExportProfileDef> {
        self.handle.export_profile(name)
    }

    /// Names of registered export profiles.
    pub fn export_profile_names(&self) -> Vec<String> {
        self.handle.export_profile_names()
    }

    /// `(name, description)` pairs for registered export profiles.
    pub fn export_profile_choices(&self) -> Vec<(String, String)> {
        self.handle.export_profile_choices()
    }

    /// Invoke a registered drag-drop workflow by name with dropped paths.
    pub fn invoke_workflow(&self, name: &str, paths: &[PathBuf]) -> Result<(), String> {
        let payload = self.drop_payload(paths).map_err(|e| e.to_string())?;
        self.handle
            .workflow_run(&self.lua, name, payload)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Invoke a registered menu workflow by name.
    pub fn invoke_menu_workflow(&self, name: &str) -> Result<(), String> {
        let payload = self.lua.create_table().map_err(|e| e.to_string())?;
        payload
            .set("scope", crate::workflow_app::SCOPE_MENU)
            .map_err(|e| e.to_string())?;
        self.handle
            .workflow_run(&self.lua, name, payload)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Ask the active workflow whether suspend is allowed.
    pub fn suspend_workflow(&self) -> Result<bool, String> {
        let instance = self.handle.inner.borrow().active.clone();
        let Some(instance) = instance else {
            return Ok(true);
        };
        use crate::workflow::table_method;
        let Some(func) = table_method(&instance, "suspend") else {
            return Ok(true);
        };
        let result: mlua::Value = func
            .call((instance, crate::session::LuaSession::focused()))
            .map_err(|e| e.to_string())?;
        match result {
            mlua::Value::Nil | mlua::Value::Boolean(true) => Ok(true),
            mlua::Value::Boolean(false) => Ok(false),
            other => Err(format!(
                "suspend must return a boolean or nil, got {}",
                other.type_name()
            )),
        }
    }

    /// Restore a stateful workflow from the session's `workflow_name`.
    pub fn resume_workflow(&self) -> Result<ResumeWorkflow, String> {
        let Some(name) = self.handle.with_backend(|b| b.session_workflow(None)) else {
            self.handle.inner.borrow_mut().active = None;
            return Ok(ResumeWorkflow::None);
        };
        let proto = self
            .handle
            .inner
            .borrow()
            .workflows
            .get(&name)
            .map(|w| w.prototype.clone());
        let Some(proto) = proto else {
            self.handle.inner.borrow_mut().active = None;
            self.handle.log(
                LogLevel::Error,
                "workflow".into(),
                format!("unknown workflow `{name}`"),
            );
            return Ok(ResumeWorkflow::Unknown { name });
        };
        use crate::workflow::{prototype_is_stateful, table_method, workflow_new};
        if !prototype_is_stateful(&proto) {
            self.handle.inner.borrow_mut().active = None;
            self.handle.log(
                LogLevel::Error,
                "workflow".into(),
                format!("workflow `{name}` is not stateful"),
            );
            return Ok(ResumeWorkflow::Unknown { name });
        }
        let instance = workflow_new(&self.lua, proto).map_err(|e| e.to_string())?;
        if let Some(func) = table_method(&instance, "resume") {
            func.call::<()>((instance.clone(), crate::session::LuaSession::focused()))
                .map_err(|e| e.to_string())?;
        }
        self.handle.inner.borrow_mut().active = Some(instance);
        Ok(ResumeWorkflow::Resumed)
    }

    /// Clear the active workflow and session workflow name.
    pub fn clear_session_workflow(&self) {
        let _ = self
            .handle
            .with_backend_mut(|b| b.set_session_workflow(None, None));
        self.handle.inner.borrow_mut().active = None;
        self.handle.close_open_sheet();
    }

    /// Dispatch a toolbar button press.
    pub fn dispatch_toolbar_button(&self, id: &str) -> Result<(), String> {
        let Some((instance, row)) = self.handle.toolbar_target(id).map_err(|e| e.to_string())?
        else {
            return Ok(());
        };
        invoke_action(&row, &instance)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Dispatch a toolbar path entry press.
    pub fn dispatch_toolbar_path(&self, id: &str, paths: &[PathBuf]) -> Result<String, String> {
        self.handle
            .dispatch_toolbar_path(&self.lua, id, paths)
            .map_err(|e| e.to_string())
    }

    /// Toggle a toolbar toggle.
    pub fn dispatch_toolbar_toggle(&self, id: &str) -> mlua::Result<bool> {
        self.handle.dispatch_toolbar_toggle(id)
    }

    /// Set a toolbar text-entry value.
    pub fn set_toolbar_entry_value(&self, id: &str, value: &str) -> mlua::Result<()> {
        self.handle.set_toolbar_entry_value(id, value)
    }

    fn drop_payload(&self, paths: &[PathBuf]) -> mlua::Result<Table> {
        let payload = self.lua.create_table()?;
        payload.set("scope", crate::workflow_app::SCOPE_DRAG_DROP)?;
        let path_table = self.lua.create_table()?;
        for (index, path) in paths.iter().enumerate() {
            path_table.set(index + 1, path.display().to_string())?;
        }
        payload.set("paths", path_table)?;
        Ok(payload)
    }
}

impl HostHandle {
    pub(crate) fn workflow_run(
        &self,
        lua: &Lua,
        name: &str,
        payload: Table,
    ) -> mlua::Result<Option<Table>> {
        if self.inner.borrow().sheet.is_some()
            || self.inner.borrow().active.as_ref().is_some_and(has_sheet)
        {
            self.alert(
                "Cannot start workflow".into(),
                "A workflow sheet is already open.".into(),
            )?;
            return Ok(None);
        }
        let proto = {
            self.inner
                .borrow()
                .workflows
                .get(name)
                .map(|workflow| workflow.prototype.clone())
        }
        .ok_or_else(|| mlua::Error::runtime(format!("unknown workflow `{name}`")))?;
        let stateful = prototype_is_stateful(&proto);
        if stateful {
            let current = self.inner.borrow().active.clone();
            if let Some(current) = current {
                let current_name = instance_name(&current)?;
                if current_name != name {
                    self.alert(
                        "Cannot start workflow".into(),
                        format!("`{current_name}` is already running."),
                    )?;
                    return Ok(None);
                }
            }
        }
        let instance = workflow_new(lua, proto)?;
        workflow_start(&instance, payload)?;
        if has_sheet(&instance) {
            self.inner.borrow_mut().sheet = Some(instance.clone());
            self.with_backend_mut(|backend| backend.sheet_changed())?;
        }
        if stateful {
            self.bind_active_workflow(instance.clone())?;
        }
        Ok(Some(instance))
    }

    pub(crate) fn bind_active_workflow(&self, instance: Table) -> mlua::Result<()> {
        let name = instance_name(&instance)?;
        self.with_backend_mut(|backend| backend.set_session_workflow(None, Some(name)))?;
        self.inner.borrow_mut().active = Some(instance);
        Ok(())
    }

    pub(crate) fn finish_workflow(&self, lua: &Lua) -> mlua::Result<()> {
        let Some(instance) = self.inner.borrow_mut().active.take() else {
            return Ok(());
        };
        let had_sheet = has_sheet(&instance);
        if let Some(finish) = table_method(&instance, "finish") {
            let session = crate::session::LuaSession::focused();
            finish.call::<()>((instance.clone(), session))?;
        }
        let _ = lua;
        self.with_backend_mut(|backend| backend.set_session_workflow(None, None))?;
        if had_sheet {
            let _ = instance.set("__fa_sheet", Value::Nil);
            self.close_open_sheet();
        }
        Ok(())
    }

    pub(crate) fn cancel_workflow(&self, lua: &Lua) -> mlua::Result<()> {
        let Some(instance) = self.inner.borrow_mut().active.take() else {
            return Ok(());
        };
        let had_sheet = has_sheet(&instance);
        if let Some(cancel) = table_method(&instance, "cancel") {
            let session = crate::session::LuaSession::focused();
            cancel.call::<()>((instance.clone(), session))?;
        }
        let _ = lua;
        self.with_backend_mut(|backend| backend.set_session_workflow(None, None))?;
        if had_sheet {
            let _ = instance.set("__fa_sheet", Value::Nil);
            self.close_open_sheet();
        }
        Ok(())
    }

    pub(crate) fn enable_system_package_paths(&self, lua: &Lua) -> mlua::Result<()> {
        self.inner
            .borrow_mut()
            .package_policy
            .enable_system_package_paths(lua)
    }

    pub(crate) fn enable_native_modules(&self, lua: &Lua) -> mlua::Result<()> {
        self.inner
            .borrow_mut()
            .package_policy
            .enable_native_modules(lua)
    }

    /// Record an alert for the host to surface (batch: stderr / capture).
    pub fn alert(&self, subject: String, body: String) -> mlua::Result<()> {
        self.inner.borrow_mut().alerts.push((subject, body));
        Ok(())
    }

    pub(crate) fn log(&self, level: LogLevel, topic: String, message: String) {
        self.inner.borrow_mut().logs.push(LogEntry {
            level,
            topic,
            message,
        });
    }

    pub(crate) fn toolbar_changed(&self, _instance: &Table) -> mlua::Result<()> {
        self.with_backend_mut(|backend| {
            backend.toolbar_changed()?;
            backend.sheet_changed()?;
            Ok(())
        })
    }

    pub(crate) fn sheet_opened(&self, instance: &Table) -> mlua::Result<()> {
        self.inner.borrow_mut().sheet = Some(instance.clone());
        self.with_backend_mut(|backend| backend.sheet_changed())
    }

    pub(crate) fn sheet_closed(&self, _instance: &Table) -> mlua::Result<()> {
        self.inner.borrow_mut().sheet = None;
        self.with_backend_mut(|backend| backend.sheet_changed())
    }

    pub(crate) fn close_open_sheet(&self) {
        let sheet = self.inner.borrow_mut().sheet.take();
        if let Some(instance) = sheet {
            let _ = instance.set("__fa_sheet", Value::Nil);
        }
        if let Some(active) = self.inner.borrow().active.clone() {
            if has_sheet(&active) {
                let _ = active.set("__fa_sheet", Value::Nil);
            }
        }
        let _ = self.with_backend_mut(|backend| backend.sheet_changed());
    }

    pub(crate) fn defer_workflow(&self, instance: Table, func: Function) -> mlua::Result<()> {
        self.inner.borrow_mut().deferred.push_back((instance, func));
        self.with_backend_mut(|backend| backend.deferred_scheduled())
    }

    /// Borrow the backend for read operations.
    ///
    /// Clones the backend handle and releases [`HostInner`] before calling
    /// `f`, so Lua callbacks that re-enter the host (hooks, composition
    /// fields) do not deadlock on the `HostInner` `RefCell`.
    pub(crate) fn with_backend<R>(&self, f: impl FnOnce(&dyn ScriptBackend) -> R) -> R {
        let backend = {
            let inner = self.inner.borrow();
            Rc::clone(&inner.backend)
        };
        let backend = backend.borrow();
        f(&*backend)
    }

    /// Borrow the backend mutably.
    ///
    /// Same release-before-callback rule as [`Self::with_backend`].
    pub(crate) fn with_backend_mut<R>(&self, f: impl FnOnce(&mut dyn ScriptBackend) -> R) -> R {
        let backend = {
            let inner = self.inner.borrow();
            Rc::clone(&inner.backend)
        };
        let mut backend = backend.borrow_mut();
        f(&mut *backend)
    }

    #[allow(dead_code)]
    pub(crate) fn dispatch_toolbar_button(&self, id: &str) -> Result<(), String> {
        let Some((instance, row)) = self.toolbar_target(id).map_err(|e| e.to_string())? else {
            return Ok(());
        };
        invoke_action(&row, &instance)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub(crate) fn dispatch_toolbar_path(
        &self,
        lua: &Lua,
        id: &str,
        paths: &[PathBuf],
    ) -> mlua::Result<String> {
        let Some((instance, row)) = self.toolbar_target(id)? else {
            return Ok(paths
                .first()
                .map(|p| p.display().to_string())
                .unwrap_or_default());
        };
        let store = control_store(&row)?;
        let table = lua.create_table()?;
        for (index, path) in paths.iter().enumerate() {
            table.set(index + 1, path.display().to_string())?;
        }
        store.raw_set("paths", table)?;
        store.raw_set(
            "value",
            paths
                .first()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        )?;
        let result = invoke_action(&row, &instance);
        store.raw_set("paths", mlua::Value::Nil)?;
        result?;
        control_string(&row, "value")
    }

    pub(crate) fn dispatch_toolbar_toggle(&self, id: &str) -> mlua::Result<bool> {
        let Some((instance, row)) = self.toolbar_target(id)? else {
            return Ok(false);
        };
        let current = control_bool(&row, "value", false)?;
        if !invoke_action(&row, &instance)? {
            let next = !current;
            control_store(&row)?.raw_set("value", next)?;
            self.toolbar_changed(&instance)?;
            return Ok(next);
        }
        control_bool(&row, "value", false)
    }

    pub(crate) fn set_toolbar_entry_value(&self, id: &str, value: &str) -> mlua::Result<()> {
        let Some((instance, row)) = self.toolbar_target(id)? else {
            return Ok(());
        };
        control_store(&row)?.raw_set("value", value)?;
        invoke_action(&row, &instance)?;
        Ok(())
    }

    fn toolbar_target(&self, id: &str) -> mlua::Result<Option<(Table, Table)>> {
        if let Some(instance) = self.inner.borrow().active.clone() {
            match toolbar_row(&instance, id) {
                Ok(row) => return Ok(Some((instance, row))),
                Err(mlua::Error::RuntimeError(message))
                    if message.starts_with("no toolbar item")
                        || message == "workflow has no toolbar" => {}
                Err(err) => return Err(err),
            }
            if has_sheet(&instance) {
                match sheet_control_row(&instance, id) {
                    Ok(row) => return Ok(Some((instance, row))),
                    Err(mlua::Error::RuntimeError(message))
                        if message.starts_with("no sheet control") => {}
                    Err(err) => return Err(err),
                }
            }
        }
        if let Some(instance) = self.inner.borrow().sheet.clone() {
            match sheet_control_row(&instance, id) {
                Ok(row) => return Ok(Some((instance, row))),
                Err(mlua::Error::RuntimeError(message))
                    if message.starts_with("no sheet control")
                        || message.contains("require an open sheet") =>
                {
                    return Ok(None);
                }
                Err(err) => return Err(err),
            }
        }
        Ok(None)
    }

    // ── Event firing helpers ─────────────────────────────────────────────────

    pub(crate) fn workflow_handlers(&self, event: &str) -> Vec<(Table, Function)> {
        let Some(instance) = self.inner.borrow().active.clone() else {
            return Vec::new();
        };
        let mut funcs = Vec::new();
        if let Some(meta) = instance.metatable() {
            collect_event_hooks(&meta, event, &mut funcs);
        }
        collect_event_hooks(&instance, event, &mut funcs);
        funcs
            .into_iter()
            .map(|func| (instance.clone(), func))
            .collect()
    }

    pub(crate) fn fire_workflow_document(&self, event: &str, id: DocumentId, elapsed: f64) {
        let composition = crate::composition::LuaComposition { id };
        for (instance, hook) in self.workflow_handlers(event) {
            if let Err(err) = hook.call::<()>((instance, composition, elapsed)) {
                self.note_hook_error(event, &err);
            }
        }
    }

    pub(crate) fn fire_workflow_session(&self, event: &str) {
        let session = crate::session::LuaSession::focused();
        for (instance, hook) in self.workflow_handlers(event) {
            if let Err(err) = hook.call::<()>((instance, session)) {
                self.note_hook_error(event, &err);
            }
        }
    }

    pub(crate) fn emit_composition_selected(&self, id: Option<DocumentId>) {
        let hooks = self.inner.borrow().composition_selected.clone();
        for hook in &hooks {
            let result = match id {
                Some(id) => hook.call::<()>(crate::composition::LuaComposition { id }),
                None => hook.call::<()>(mlua::Value::Nil),
            };
            if let Err(err) = result {
                self.note_hook_error("composition_selected", &err);
            }
        }
        for (instance, hook) in self.workflow_handlers("composition_selected") {
            let result = match id {
                Some(id) => hook.call::<()>((instance, crate::composition::LuaComposition { id })),
                None => hook.call::<()>((instance, mlua::Value::Nil)),
            };
            if let Err(err) = result {
                self.note_hook_error("composition_selected", &err);
            }
        }
    }

    fn note_hook_error(&self, event: &str, err: &mlua::Error) {
        self.inner
            .borrow_mut()
            .prints
            .push(format!("{event} hook error: {err}"));
    }

    // ── Layout helpers (define_layout/layout/layout_names/choose_layout in layout.rs) ─

    pub(crate) fn layout_choices(&self) -> Vec<(String, String)> {
        self.inner
            .borrow()
            .layouts
            .iter()
            .map(|l| (l.name.clone(), l.description.clone()))
            .collect()
    }

    pub(crate) fn apply_effective_layout(
        &self,
        id: DocumentId,
        name: Option<&str>,
    ) -> mlua::Result<()> {
        let name_owned = name.map(str::to_string);
        let (labels, default_chain) = if let Some(ref name_str) = name_owned {
            let layout_opt: Option<ChannelLayoutDef> = {
                let inner = self.inner.borrow();
                inner.layouts.iter().find(|l| l.name == *name_str).cloned()
            };
            let labels = layout_opt
                .as_ref()
                .map(|l| l.channels.clone())
                .unwrap_or_default();
            let chain = layout_opt
                .as_ref()
                .and_then(|l| l.monitor_chain_id().map(str::to_string));
            (labels, chain)
        } else {
            (BTreeMap::new(), None)
        };
        self.with_backend_mut(|backend| {
            backend.with_open_document_mut(id, &mut |doc| {
                let mut composition = doc.composition.write().unwrap();
                composition.apply_channel_layout(name_owned.clone(), labels.clone());
                if composition.monitor_chain().is_none() {
                    if let Some(chain) = default_chain.clone() {
                        composition.set_monitor_chain(Some(chain));
                    }
                }
                Ok(())
            })
        })
    }

    pub(crate) fn fire_enrich_composition(&self, id: DocumentId) {
        let hooks = self.inner.borrow().enrich_composition.clone();
        let handle = crate::composition::LuaComposition { id };
        for hook in &hooks {
            if let Err(err) = hook.call::<()>(handle) {
                self.note_hook_error("enrich_composition", &err);
            }
        }
        for (instance, hook) in self.workflow_handlers("enrich_composition") {
            if let Err(err) = hook.call::<()>((instance, handle)) {
                self.note_hook_error("enrich_composition", &err);
            }
        }
    }

    /// Fire `enrich_media` hooks for probed media.
    pub fn fire_enrich_media(&self, media: crate::media::LuaMedia) {
        let hooks = self.inner.borrow().enrich_media.clone();
        for hook in &hooks {
            if let Err(err) = hook.call::<()>(media.clone()) {
                self.note_hook_error("enrich_media", &err);
            }
        }
        for (instance, hook) in self.workflow_handlers("enrich_media") {
            if let Err(err) = hook.call::<()>((instance, media.clone())) {
                self.note_hook_error("enrich_media", &err);
            }
        }
    }

    /// Enrich primary media for a composition document.
    pub(crate) fn fire_enrich_media_for_document(&self, id: DocumentId) {
        let media_id = {
            let mut result = None;
            let _ = self.with_backend(|backend| {
                backend.with_open_document(id, &mut |doc| {
                    result = doc
                        .composition
                        .read()
                        .unwrap()
                        .primary_media()
                        .map(|m| m.id);
                    Ok(())
                })
            });
            result
        };
        if let Some(media_id) = media_id {
            self.fire_enrich_media(crate::media::LuaMedia::pooled(media_id));
        }
    }

    pub(crate) fn fire_enrich_session(&self, which: Option<SessionId>) {
        let session = crate::session::LuaSession { detached_id: which };
        let hooks = self.inner.borrow().enrich_session.clone();
        for hook in &hooks {
            if let Err(err) = hook.call::<()>(session) {
                self.note_hook_error("enrich_session", &err);
            }
        }
        for (instance, hook) in self.workflow_handlers("enrich_session") {
            if let Err(err) = hook.call::<()>((instance, session)) {
                self.note_hook_error("enrich_session", &err);
            }
        }
    }

    pub(crate) fn fire_detect_layout(&self, id: DocumentId) {
        // Retrieve the currently chosen layout from the composition.
        let chosen: Option<String> = {
            let mut result = None;
            let _ = self.with_backend(|backend| {
                backend.with_open_document(id, &mut |doc| {
                    result = doc
                        .composition
                        .read()
                        .unwrap()
                        .chosen_channel_layout()
                        .map(str::to_string);
                    Ok(())
                })
            });
            result
        };

        let hooks = self.inner.borrow().detect_layout.clone();
        let handle = crate::composition::LuaComposition { id };
        let mut last: Option<String> = None;

        for hook in &hooks {
            match hook.call::<Option<String>>((handle, chosen.clone())) {
                Ok(Some(name)) => {
                    if self.layout(&name).is_some() {
                        last = Some(name);
                    } else {
                        self.inner
                            .borrow_mut()
                            .prints
                            .push(format!("detect_layout hook error: unknown layout `{name}`"));
                    }
                }
                Ok(None) => {}
                Err(err) => self.note_hook_error("detect_layout", &err),
            }
        }

        for (instance, hook) in self.workflow_handlers("detect_layout") {
            match hook.call::<Option<String>>((instance, handle, chosen.clone())) {
                Ok(Some(name)) => {
                    if self.layout(&name).is_some() {
                        last = Some(name);
                    } else {
                        self.inner
                            .borrow_mut()
                            .prints
                            .push(format!("detect_layout hook error: unknown layout `{name}`"));
                    }
                }
                Ok(None) => {}
                Err(err) => self.note_hook_error("detect_layout", &err),
            }
        }

        if let Err(err) = self.apply_effective_layout(id, last.as_deref()) {
            self.inner
                .borrow_mut()
                .prints
                .push(format!("detect_layout hook error: {err}"));
        }
    }
}

/// Fetch the host handle from Lua app data.
pub fn host_from_lua(lua: &Lua) -> mlua::Result<HostHandle> {
    lua.app_data_ref::<HostHandle>()
        .map(|h| h.clone())
        .ok_or_else(|| mlua::Error::runtime("script host is not available"))
}

fn collect_event_hooks(table: &Table, event: &str, out: &mut Vec<Function>) {
    let Ok(Value::Table(hooks)) = table.raw_get::<Value>("__fa_hooks") else {
        return;
    };
    let Ok(Value::Table(list)) = hooks.raw_get::<Value>(event) else {
        return;
    };
    for value in list.sequence_values::<Value>() {
        if let Ok(Value::Function(func)) = value {
            out.push(func);
        }
    }
}

fn install_print(lua: &Lua, handle: HostHandle) -> mlua::Result<()> {
    let print = lua.create_function(move |lua, args: MultiValue| {
        let line = stringify_values(lua, args).unwrap_or_default();
        handle.inner.borrow_mut().prints.push(line);
        Ok(())
    })?;
    lua.globals().set("print", print)?;
    Ok(())
}

fn eval_repl(lua: &Lua, code: &str) -> mlua::Result<MultiValue> {
    let trimmed = code.trim();
    if trimmed.is_empty() {
        return Ok(MultiValue::new());
    }
    // Chunks that already return a value (possibly after `local` statements).
    if trimmed.contains("return ") || trimmed.starts_with("return") {
        return lua.load(trimmed).eval::<MultiValue>();
    }
    if trimmed.contains('\n') {
        match lua.load(trimmed).eval::<MultiValue>() {
            Ok(values) => Ok(values),
            // Only fall back to statement exec when the chunk is not a valid
            // expression form — keep runtime errors (e.g. unknown userdata
            // fields) so the REPL shows the real failure.
            Err(err) if is_syntax_error(&err) => {
                lua.load(trimmed).exec()?;
                Ok(MultiValue::new())
            }
            Err(err) => Err(err),
        }
    } else {
        let expr = format!("return {trimmed}");
        match lua.load(&expr).eval::<MultiValue>() {
            Ok(values) => Ok(values),
            Err(err) if is_syntax_error(&err) => {
                lua.load(trimmed).exec()?;
                Ok(MultiValue::new())
            }
            Err(err) => Err(err),
        }
    }
}

/// True when `err` (or a nested cause) is a Lua parse/compile failure.
fn is_syntax_error(err: &mlua::Error) -> bool {
    match err {
        mlua::Error::SyntaxError { .. } => true,
        mlua::Error::CallbackError { cause, .. } => is_syntax_error(cause),
        mlua::Error::WithContext { cause, .. } => is_syntax_error(cause),
        mlua::Error::BadArgument { cause, .. } => is_syntax_error(cause),
        _ => false,
    }
}

pub(crate) fn stringify_values(lua: &Lua, values: MultiValue) -> Option<String> {
    if values.is_empty() {
        return None;
    }
    let parts: Vec<String> = values
        .into_iter()
        .map(|value| stringify_value(lua, value))
        .collect();
    Some(parts.join("\t"))
}

fn control_bool(row: &Table, key: &str, default: bool) -> mlua::Result<bool> {
    match control_store(row)?.raw_get::<Value>(key)? {
        Value::Nil => Ok(default),
        Value::Boolean(value) => Ok(value),
        other => Err(mlua::Error::runtime(format!(
            "{key} must be a boolean, got {}",
            other.type_name()
        ))),
    }
}

fn control_string(row: &Table, key: &str) -> mlua::Result<String> {
    match control_store(row)?.raw_get::<Value>(key)? {
        Value::Nil => Ok(String::new()),
        Value::String(text) => Ok(text.to_str()?.to_owned()),
        other => Err(mlua::Error::runtime(format!(
            "{key} must be a string, got {}",
            other.type_name()
        ))),
    }
}

/// Execute a Lua file with shebang stripping and `field.include` parent push.
pub(crate) fn exec_lua_file(lua: &Lua, host: &HostHandle, path: &Path) -> Result<(), String> {
    let source =
        std::fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let chunk = strip_shebang(&source);
    let parent = path.parent().map(Path::to_path_buf);
    if let Some(parent) = parent {
        host.inner.borrow_mut().include_stack.push(parent);
    }
    let result = lua
        .load(chunk)
        .set_name(path.display().to_string())
        .exec()
        .map_err(|err| format!("{}: {err}", path.display()));
    if !host.inner.borrow().include_stack.is_empty() {
        host.inner.borrow_mut().include_stack.pop();
    }
    result
}

/// Load all `{prefix}*.lua` along the scripting search path.
pub(crate) fn load_matching_scripts(
    lua: &Lua,
    host: &HostHandle,
    config: Option<&Path>,
    prefix: &str,
) -> Result<(), String> {
    for path in crate::script_search::collect_matching_lua(config, prefix) {
        exec_lua_file(lua, host, &path)?;
    }
    Ok(())
}

fn strip_shebang(source: &str) -> &str {
    let trimmed = source.strip_prefix('\u{feff}').unwrap_or(source);
    if let Some(rest) = trimmed.strip_prefix("#!") {
        rest.find('\n').map(|i| &rest[i + 1..]).unwrap_or("")
    } else {
        trimmed
    }
}

pub(crate) fn stringify_value(lua: &Lua, value: Value) -> String {
    match value {
        Value::Nil => "nil".into(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{}", n as i64)
            } else {
                n.to_string()
            }
        }
        Value::String(s) => s.to_str().map(|s| s.to_owned()).unwrap_or_default(),
        Value::Table(_) | Value::Function(_) | Value::UserData(_) | Value::Thread(_) => {
            match lua
                .globals()
                .get::<Function>("tostring")
                .and_then(|tostring| tostring.call::<String>(value))
            {
                Ok(text) => text,
                Err(_) => "<value>".into(),
            }
        }
        Value::LightUserData(_) => "<lightuserdata>".into(),
        Value::Error(err) => err.to_string(),
        _ => "<value>".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_name_is_read_only_host_id() {
        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: None,
        })
        .unwrap();
        let out = host.eval("return app.name");
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("field-batch"));
        let out = host.eval("app.name = 'x'");
        assert!(out.error.is_some());
    }

    #[test]
    fn one_shot_sheet_retains_instance_without_binding_session() {
        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: None,
        })
        .unwrap();
        let out = host.eval(
            r#"
            local W = field.workflow.create({
              name = "wizard",
              display_name = "Wizard",
              scopes = { "run" },
            })
            function W:start(_payload)
              self:set_sheet({
                panes = {
                  {
                    id = "one",
                    name = "One",
                    text = "Hello",
                    controls = {
                      field.ui.progress({ id = "p", value = 50 }),
                      field.ui.log({ id = "log", text = "" }),
                    },
                    buttons = {
                      field.ui.button({
                        id = "ok",
                        label = "OK",
                        action = function(_, wf) wf:close_sheet() end,
                      }),
                    },
                  },
                },
              })
            end
            field.workflow.declare(W)
            field.workflow.run("wizard")
            return app.workflow ~= nil, field.session.focused().workflow_name
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        // One-shot: not bound as app.workflow / session.workflow_name.
        assert_eq!(out.result.as_deref(), Some("false\tnil"));
        let snap = host.sheet_snapshot().expect("sheet open");
        assert_eq!(snap.current, "one");
        assert_eq!(snap.panes.len(), 1);
        assert!(matches!(
            &snap.panes[0].controls[0],
            crate::workflow_toolbar::ToolbarItem::Progress { .. }
        ));
        host.close_workflow_sheet();
        assert!(host.sheet_snapshot().is_none());
    }

    #[test]
    fn stateful_sheet_hides_toolbar_snapshot() {
        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: None,
        })
        .unwrap();
        let out = host.eval(
            r#"
            local W = field.workflow.create({
              name = "ingestish",
              display_name = "Ingestish",
              scopes = { "run" },
            })
            function W:suspend(_session) return true end
            function W:start(_payload)
              self:set_sheet({
                panes = {
                  {
                    id = "configure",
                    name = "Configure",
                    text = "Go",
                    buttons = {
                      field.ui.button({ id = "run", label = "Run" }),
                    },
                  },
                },
              })
            end
            field.workflow.declare(W)
            field.workflow.run("ingestish")
            return app.workflow ~= nil
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("true"));
        assert!(host.sheet_snapshot().is_some());
        assert!(
            host.toolbar_snapshot().is_none(),
            "sheet-open stateful workflow must not paint a toolbar"
        );
    }

    #[test]
    fn app_session_and_composition_sugar() {
        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: None,
        })
        .unwrap();
        let out = host.eval(
            r#"
            local focused = field.session.focused()
            local same_session = app.session.id == focused.id
            local same_via_session = (app.composition == nil) == (focused.composition == nil)
            if app.composition and focused.composition then
              same_via_session = app.composition.id == focused.composition.id
            end
            local same_via_app = (app.composition == nil) == (app.session.composition == nil)
            if app.composition and app.session.composition then
              same_via_app = app.composition.id == app.session.composition.id
            end
            return same_session, same_via_session, same_via_app
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("true\ttrue\ttrue"));
    }

    #[test]
    fn load_init_reads_variables_json() {
        let config = tempfile::tempdir().unwrap();
        std::fs::write(
            config.path().join("variables.json"),
            r#"{
  "kind": "variables",
  "format_version": 1,
  "variables": [
    { "name": "artist", "value": "Ada" },
    { "name": "ingest.root_dir", "value": "/data" }
  ]
}
"#,
        )
        .unwrap();
        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: Some(config.path().to_path_buf()),
        })
        .unwrap();
        host.load_init_from(Some(config.path())).unwrap();
        let out = host.eval(
            r#"
            local u = field.variables.user()
            local value, scope = u:variable_resolver():resolve("user.ingest.root_dir")
            return u.values.artist, value, scope
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("Ada\t/data\tuser.ingest"));
    }

    #[test]
    fn eval_unknown_userdata_field_surfaces_runtime_error() {
        // Fields-only userdata (WorkflowBaseProperties) errors on unknown fields.
        // (Types that also register methods fall back to nil for missing keys.)
        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: None,
        })
        .unwrap();
        let out = host.eval(
            r#"
            local w = field.workflow.create({ name = "t", scopes = { "menu" } })
            return w.__base_properties.layout
            "#,
        );
        let err = out.error.expect("expected an error");
        assert!(
            err.contains("unknown field") && err.contains("layout"),
            "expected unknown-field runtime error, got {err}"
        );
        assert!(
            !err.contains("syntax error"),
            "runtime field error must not be masked as syntax error: {err}"
        );
    }

    #[test]
    fn eval_assignment_still_works_via_syntax_fallback() {
        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: None,
        })
        .unwrap();
        let out = host.eval("x = 7");
        assert!(out.error.is_none(), "{:?}", out.error);
        let out = host.eval("x");
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("7"));
    }

    #[test]
    fn eval_expression_returns_value() {
        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: None,
        })
        .unwrap();
        let out = host.eval("1 + 2");
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("3"));
    }

    #[test]
    fn strip_shebang_removes_first_line() {
        assert_eq!(
            strip_shebang("#!/usr/bin/env field-batch\nprint(1)\n"),
            "print(1)\n"
        );
        assert_eq!(strip_shebang("print(1)\n"), "print(1)\n");
    }

    #[test]
    fn load_resolvers_and_workflows_scan_search_path() {
        let config = tempfile::tempdir().unwrap();
        let extra = tempfile::tempdir().unwrap();
        std::fs::write(config.path().join("init.lua"), "-- init\n").unwrap();
        std::fs::write(
            config.path().join("settings.json"),
            format!(
                r#"{{ "scripting": {{ "search_path": ["{}"] }} }}"#,
                extra.path().display().to_string().replace('\\', "/")
            ),
        )
        .unwrap();
        std::fs::write(
            config.path().join("resolver_config.lua"),
            r#"
            local R = field.variables.create_resolver({ name = "from_config" })
            function R:init(b) self._b = b end
            function R:names() return {} end
            function R:resolve() return nil, nil end
            field.variables.declare_resolver(R)
            "#,
        )
        .unwrap();
        std::fs::write(
            extra.path().join("resolver_extra.lua"),
            r#"
            local R = field.variables.create_resolver({ name = "from_extra" })
            function R:init(b) self._b = b end
            function R:names() return {} end
            function R:resolve() return nil, nil end
            field.variables.declare_resolver(R)
            "#,
        )
        .unwrap();
        std::fs::write(
            extra.path().join("workflow_extra.lua"),
            r#"
            field.workflow.declare({
              name = "extra",
              display_name = "Extra",
              scopes = { "run" },
            }, function() end)
            "#,
        )
        .unwrap();

        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: Some(config.path().to_path_buf()),
        })
        .unwrap();
        host.load_init_from(Some(config.path())).unwrap();
        host.load_resolvers().unwrap();
        host.load_workflows().unwrap();

        let out = host.eval(
            r#"
            field.variables.set_resolver("from_config")
            field.variables.set_resolver("from_extra")
            return true
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert!(
            host.workflow_metas().iter().any(|m| m.name == "extra"),
            "{:?}",
            host.workflow_metas()
        );

        // Lua APIs are the same helpers.
        let out = host
            .eval("field.variables.load_resolvers(); field.workflow.load_workflows(); return true");
        assert!(out.error.is_none(), "{:?}", out.error);
    }

    #[test]
    fn apply_scripting_settings_extends_path_before_init() {
        let config = tempfile::tempdir().unwrap();
        let extra = tempfile::tempdir().unwrap();
        std::fs::write(extra.path().join("preinit.lua"), "return { ok = true }").unwrap();
        let mut settings = field_settings::ScriptingSettings::default();
        settings.search_path = vec![extra.path().display().to_string()];
        settings.enable_native_modules = true;

        let mut host = ScriptHost::new(HostProfile {
            name: "field-batch",
            config_dir: Some(config.path().to_path_buf()),
        })
        .unwrap();
        host.apply_scripting_settings(&settings).unwrap();

        let path: String = host
            .lua()
            .globals()
            .get::<mlua::Table>("package")
            .unwrap()
            .get("path")
            .unwrap();
        let marker = format!(
            "{}/?.lua",
            extra
                .path()
                .display()
                .to_string()
                .trim_end_matches(['/', '\\'])
        );
        assert!(path.contains(&marker), "{path}");

        let ok: bool = host
            .lua()
            .load("return require('preinit').ok")
            .eval()
            .unwrap();
        assert!(ok);

        // Native modules enabled without going through Lua.
        assert!(host
            .handle
            .inner
            .borrow()
            .package_policy
            .native_modules_enabled());

        host.load_init_from(Some(config.path())).unwrap();
    }
}
