// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Host construction and eval.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mlua::{Function, Lua, MultiValue, Table, Value};

use crate::app::bind_app;
use crate::field_ns::bind_field;
use crate::layout::ChannelLayoutDef;
use crate::workflow::{
    instance_display_name, instance_name, prototype_is_stateful, table_method, workflow_new,
    workflow_start, WorkflowDef, WorkflowMeta,
};
use crate::workflow_app::{layout_drop_targets, workflows_for_menu, DropLayout};
use crate::workflow_toolbar::{invoke_action, toolbar_from_table, toolbar_row, ToolbarItem};
use crate::world::HeadlessWorld;

/// Profile for the enclosing application process.
#[derive(Clone, Debug)]
pub struct HostProfile {
    /// Read-only `app.name` (e.g. `"field-assist"`, `"field-batch"`).
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
    pub(crate) layouts: Vec<ChannelLayoutDef>,
    pub(crate) workflows: BTreeMap<String, WorkflowDef>,
    pub(crate) active: Option<Table>,
    pub(crate) world: Rc<RefCell<HeadlessWorld>>,
    pub(crate) include_stack: Vec<PathBuf>,
    pub(crate) include_cache: HashMap<String, Value>,
    pub(crate) args: Vec<String>,
}

/// Shared handle stored in Lua app data.
#[derive(Clone)]
pub struct HostHandle {
    pub(crate) inner: Rc<RefCell<HostInner>>,
}

/// Lua runtime with `app` + `field` bindings.
pub struct ScriptHost {
    lua: Lua,
    handle: HostHandle,
}

impl ScriptHost {
    /// Create a host for `profile` with an empty [`HeadlessWorld`].
    pub fn new(profile: HostProfile) -> mlua::Result<Self> {
        Self::with_world(profile, HeadlessWorld::new())
    }

    /// Create a host with a pre-built world (tests).
    pub fn with_world(profile: HostProfile, world: HeadlessWorld) -> mlua::Result<Self> {
        let lua = Lua::new();
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
                layouts: Vec::new(),
                workflows: BTreeMap::new(),
                active: None,
                world: Rc::new(RefCell::new(world)),
                include_stack: Vec::new(),
                include_cache: HashMap::new(),
                args: Vec::new(),
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

    /// Shared world handle.
    pub fn world(&self) -> Rc<RefCell<HeadlessWorld>> {
        self.handle.inner.borrow().world.clone()
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

    /// Load and execute a Lua file.
    ///
    /// A leading Unix shebang (`#!…`) is stripped so scripts can use
    /// `#!/usr/bin/env field-batch`.
    pub fn load_file(&mut self, path: &Path) -> Result<(), String> {
        let source =
            std::fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
        let chunk = strip_shebang(&source);
        let parent = path.parent().map(Path::to_path_buf);
        if let Some(parent) = parent {
            self.handle.inner.borrow_mut().include_stack.push(parent);
        }
        let result = self
            .lua
            .load(chunk)
            .set_name(path.display().to_string())
            .exec()
            .map_err(|err| format!("{}: {err}", path.display()));
        if !self.handle.inner.borrow().include_stack.is_empty() {
            self.handle.inner.borrow_mut().include_stack.pop();
        }
        result
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
    pub fn toolbar_snapshot(&self) -> Option<(String, Vec<ToolbarItem>)> {
        let active = self.handle.inner.borrow().active.clone()?;
        let name = instance_display_name(&active).ok()?;
        let items = toolbar_from_table(&active);
        Some((name, items))
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
}

impl HostHandle {
    pub(crate) fn workflow_run(
        &self,
        lua: &Lua,
        name: &str,
        payload: Table,
    ) -> mlua::Result<Option<Table>> {
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
        if stateful {
            self.bind_active_workflow(instance.clone())?;
        }
        Ok(Some(instance))
    }

    pub(crate) fn bind_active_workflow(&self, instance: Table) -> mlua::Result<()> {
        let name = instance_name(&instance)?;
        self.with_world_mut(|world| world.session.set_workflow(Some(name)));
        self.inner.borrow_mut().active = Some(instance);
        Ok(())
    }

    pub(crate) fn finish_workflow(&self, lua: &Lua) -> mlua::Result<()> {
        let Some(instance) = self.inner.borrow_mut().active.take() else {
            return Ok(());
        };
        if let Some(finish) = table_method(&instance, "finish") {
            let session = crate::session::LuaSession::shared();
            finish.call::<()>((instance.clone(), session))?;
        }
        let _ = lua;
        self.with_world_mut(|world| world.session.set_workflow(None));
        Ok(())
    }

    pub(crate) fn cancel_workflow(&self, lua: &Lua) -> mlua::Result<()> {
        let Some(instance) = self.inner.borrow_mut().active.take() else {
            return Ok(());
        };
        if let Some(cancel) = table_method(&instance, "cancel") {
            let session = crate::session::LuaSession::shared();
            cancel.call::<()>((instance.clone(), session))?;
        }
        let _ = lua;
        self.with_world_mut(|world| world.session.set_workflow(None));
        Ok(())
    }

    pub(crate) fn alert(&self, subject: String, body: String) -> mlua::Result<()> {
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
        Ok(())
    }

    pub(crate) fn with_world<R>(&self, f: impl FnOnce(&HeadlessWorld) -> R) -> R {
        let inner = self.inner.borrow();
        let world = inner.world.borrow();
        f(&world)
    }

    pub(crate) fn with_world_mut<R>(&self, f: impl FnOnce(&mut HeadlessWorld) -> R) -> R {
        let inner = self.inner.borrow();
        let mut world = inner.world.borrow_mut();
        f(&mut world)
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

    #[allow(dead_code)]
    fn toolbar_target(&self, id: &str) -> mlua::Result<Option<(Table, Table)>> {
        let Some(instance) = self.inner.borrow().active.clone() else {
            return Ok(None);
        };
        let row = match toolbar_row(&instance, id) {
            Ok(row) => row,
            Err(mlua::Error::RuntimeError(message))
                if message.starts_with("no toolbar item")
                    || message == "workflow has no toolbar" =>
            {
                return Ok(None);
            }
            Err(err) => return Err(err),
        };
        Ok(Some((instance, row)))
    }
}

/// Fetch the host handle from Lua app data.
pub fn host_from_lua(lua: &Lua) -> mlua::Result<HostHandle> {
    lua.app_data_ref::<HostHandle>()
        .map(|h| h.clone())
        .ok_or_else(|| mlua::Error::runtime("script host is not available"))
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
    // Prefer expression form; if the chunk already returns, eval as-is.
    let expr = if trimmed.starts_with("return ") || trimmed.starts_with("return\n") {
        trimmed.to_string()
    } else if trimmed.contains('\n') {
        // Multi-statement: exec then nothing; callers should use explicit return.
        match lua.load(trimmed).eval::<MultiValue>() {
            Ok(values) => return Ok(values),
            Err(_) => {
                lua.load(trimmed).exec()?;
                return Ok(MultiValue::new());
            }
        }
    } else {
        format!("return {trimmed}")
    };
    match lua.load(&expr).eval::<MultiValue>() {
        Ok(values) => Ok(values),
        Err(_) => {
            lua.load(trimmed).exec()?;
            Ok(MultiValue::new())
        }
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
    fn strip_shebang_removes_first_line() {
        assert_eq!(
            strip_shebang("#!/usr/bin/env field-batch\nprint(1)\n"),
            "print(1)\n"
        );
        assert_eq!(strip_shebang("print(1)\n"), "print(1)\n");
    }
}
