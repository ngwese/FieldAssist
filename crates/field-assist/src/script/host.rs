// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, RwLock};

use mlua::{Function, Lua, MultiValue, Value};

use crate::model::composition::{is_facomp_path, Composition};
use crate::model::document::BufferDocument;
use crate::model::{is_fasession_path, Buffer, DocumentId, Session, SessionDocument, SessionId};

use super::access;
use super::app::bind_app;
use super::composition::LuaComposition;
use super::layout::ChannelLayoutDef;
use super::session::LuaSession;
use super::workflow::{
    layout_drop_targets, prototype_is_stateful, prototype_method, toolbar_from_prototype,
    toolbar_row_id, workflows_for_menu, DropLayout, ToolbarItem, WorkflowDef, WorkflowMeta,
    SCOPE_DRAG_DROP, SCOPE_MENU,
};

pub const EMBEDDED_INIT: &str = include_str!("../../assets/init.lua");
const EMBEDDED_WORKFLOW_ADD: &str = include_str!("../../assets/workflow_add.lua");
const EMBEDDED_WORKFLOW_REPLACE: &str = include_str!("../../assets/workflow_replace.lua");
const EMBEDDED_WORKFLOW_REVIEW: &str = include_str!("../../assets/workflow_review.lua");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    pub level: LogLevel,
    pub topic: String,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct EvalOutput {
    pub prints: Vec<String>,
    pub result: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResumeWorkflow {
    None,
    Resumed,
    Unknown { name: String },
}

struct HostInner {
    prints: Vec<String>,
    logs: Vec<LogEntry>,
    alerts: Vec<(String, String)>,
    loaded: Vec<Function>,
    saved: Vec<Function>,
    session_loaded: Vec<Function>,
    session_saved: Vec<Function>,
    detect_layout: Vec<Function>,
    layouts: Vec<ChannelLayoutDef>,
    workflows: BTreeMap<String, WorkflowDef>,
    active_workflow: Option<String>,
    detached_sessions: HashMap<SessionId, Session>,
    test: Option<Rc<RefCell<TestWorld>>>,
}

#[derive(Clone)]
pub struct HostHandle {
    inner: Rc<RefCell<HostInner>>,
}

pub struct ScriptHost {
    lua: Lua,
    handle: HostHandle,
}

pub struct TestWorld {
    pub docs: HashMap<DocumentId, BufferDocument>,
    pub paths: HashMap<DocumentId, Option<PathBuf>>,
    pub names: HashMap<DocumentId, String>,
    pub active: Option<DocumentId>,
    pub session: Session,
    next_id: u128,
    pub output_device: Option<String>,
    pub output_devices: Vec<String>,
    pub looping: bool,
    pub preview: bool,
    pub explorer: bool,
    pub detail: bool,
    pub script: bool,
}

impl TestWorld {
    pub fn new() -> Self {
        Self {
            docs: HashMap::new(),
            paths: HashMap::new(),
            names: HashMap::new(),
            active: None,
            session: Session::new(),
            next_id: 0,
            output_device: None,
            output_devices: Vec::new(),
            looping: false,
            preview: false,
            explorer: false,
            detail: false,
            script: false,
        }
    }

    pub fn push(
        &mut self,
        composition: Composition,
        buffer: Buffer,
        name: impl Into<String>,
        path: Option<PathBuf>,
    ) -> DocumentId {
        self.next_id += 1;
        let id = DocumentId::from_u128(self.next_id);
        let composition = Arc::new(RwLock::new(composition));
        let buffer = Arc::new(RwLock::new(buffer));
        let document = BufferDocument::with_shared(composition, buffer);
        self.docs.insert(id, document);
        self.paths.insert(id, path.clone());
        self.names.insert(id, name.into());
        self.session.insert(SessionDocument::new(id, path));
        self.active = Some(id);
        id
    }

    fn close(&mut self, id: DocumentId) {
        self.docs.remove(&id);
        self.paths.remove(&id);
        self.names.remove(&id);
        self.session.close_document(id);
        self.active = self.session.active();
    }
}

impl ScriptHost {
    pub fn new() -> mlua::Result<Self> {
        Self::with_test(None)
    }

    pub fn for_test(world: Rc<RefCell<TestWorld>>) -> mlua::Result<Self> {
        Self::with_test(Some(world))
    }

    fn with_test(test: Option<Rc<RefCell<TestWorld>>>) -> mlua::Result<Self> {
        let lua = Lua::new();
        let handle = HostHandle {
            inner: Rc::new(RefCell::new(HostInner {
                prints: Vec::new(),
                logs: Vec::new(),
                alerts: Vec::new(),
                loaded: Vec::new(),
                saved: Vec::new(),
                session_loaded: Vec::new(),
                session_saved: Vec::new(),
                detect_layout: Vec::new(),
                layouts: Vec::new(),
                workflows: BTreeMap::new(),
                active_workflow: None,
                detached_sessions: HashMap::new(),
                test,
            })),
        };
        lua.set_app_data(handle.clone());
        bind_app(&lua)?;
        install_print(&lua, handle.clone())?;
        Ok(Self { lua, handle })
    }

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

    pub fn load_init(&mut self) -> Result<(), String> {
        self.load_init_from(crate::commands::user_config_dir().as_deref())
    }

    pub fn load_init_from(&mut self, config_dir: Option<&Path>) -> Result<(), String> {
        self.load_embedded_workflows()?;
        self.load_init_script(config_dir)?;
        self.load_user_workflows(config_dir)?;
        Ok(())
    }

    fn load_init_script(&mut self, config_dir: Option<&Path>) -> Result<(), String> {
        if let Some(path) = user_init_path(config_dir) {
            return self
                .lua
                .load(path.as_path())
                .exec()
                .map_err(|err| format!("init.lua: {err}"));
        }
        self.lua
            .load(EMBEDDED_INIT)
            .set_name("@<embedded>/init.lua")
            .exec()
            .map_err(|err| format!("init.lua: {err}"))
    }

    fn load_embedded_workflows(&mut self) -> Result<(), String> {
        self.lua
            .load(EMBEDDED_WORKFLOW_ADD)
            .set_name("@<embedded>/workflow_add.lua")
            .exec()
            .map_err(|err| format!("workflow_add.lua: {err}"))?;
        self.lua
            .load(EMBEDDED_WORKFLOW_REPLACE)
            .set_name("@<embedded>/workflow_replace.lua")
            .exec()
            .map_err(|err| format!("workflow_replace.lua: {err}"))?;
        self.lua
            .load(EMBEDDED_WORKFLOW_REVIEW)
            .set_name("@<embedded>/workflow_review.lua")
            .exec()
            .map_err(|err| format!("workflow_review.lua: {err}"))?;
        Ok(())
    }

    fn load_user_workflows(&mut self, config_dir: Option<&Path>) -> Result<(), String> {
        let Some(dir) = config_dir else {
            return Ok(());
        };
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(format!("workflows: {err}")),
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(is_workflow_filename)
            })
            .collect();
        files.sort();
        for path in files {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("workflow.lua");
            self.lua
                .load(path.as_path())
                .exec()
                .map_err(|err| format!("{name}: {err}"))?;
        }
        Ok(())
    }

    pub fn invoke_workflow(&self, name: &str, paths: &[PathBuf]) -> Result<(), String> {
        self.start_workflow(name, SCOPE_DRAG_DROP, Some(paths))
    }

    pub fn invoke_menu_workflow(&self, name: &str) -> Result<(), String> {
        self.start_workflow(name, SCOPE_MENU, None)
    }

    pub fn menu_workflows(&self) -> Vec<(String, String)> {
        let metas = self.handle.workflow_metas();
        workflows_for_menu(&metas)
            .into_iter()
            .map(|workflow| (workflow.name.clone(), workflow.display_name.clone()))
            .collect()
    }

    fn start_workflow(
        &self,
        name: &str,
        scope: &str,
        paths: Option<&[PathBuf]>,
    ) -> Result<(), String> {
        let proto = self
            .handle
            .inner
            .borrow()
            .workflows
            .get(name)
            .map(|workflow| workflow.prototype.clone())
            .ok_or_else(|| format!("unknown workflow `{name}`"))?;
        let stateful = prototype_is_stateful(&proto);
        if stateful {
            let current = self.handle.inner.borrow().active_workflow.clone();
            if let Some(current) = current {
                if current != name {
                    let _ = self.handle.alert(
                        "Cannot start workflow".into(),
                        format!("`{current}` is already running."),
                    );
                    return Ok(());
                }
            }
        }
        let payload = self.lua.create_table().map_err(|err| err.to_string())?;
        payload.set("scope", scope).map_err(|err| err.to_string())?;
        if let Some(paths) = paths {
            let path_table = self.lua.create_table().map_err(|err| err.to_string())?;
            for (index, path) in paths.iter().enumerate() {
                path_table
                    .set(index + 1, path.display().to_string())
                    .map_err(|err| err.to_string())?;
            }
            payload
                .set("paths", path_table)
                .map_err(|err| err.to_string())?;
        }
        if let Some(start) = prototype_method(&proto, "start") {
            start
                .call::<()>((proto.clone(), payload))
                .map_err(|err| err.to_string())?;
        }
        if stateful {
            self.handle.bind_active_workflow(name);
        }
        Ok(())
    }

    pub fn dispatch_workflow_command(&self, command: &str) -> Result<(), String> {
        self.handle.dispatch_workflow_command(command)
    }

    pub fn dispatch_toolbar_path(&self, id: &str, paths: &[PathBuf]) -> Result<String, String> {
        self.handle
            .dispatch_toolbar_path(&self.lua, id, paths)
            .map_err(|err| err.to_string())
    }

    pub fn dispatch_toolbar_toggle(&self, id: &str) -> Result<bool, String> {
        self.handle
            .dispatch_toolbar_toggle(id)
            .map_err(|err| err.to_string())
    }

    pub fn set_toolbar_path_value(&self, id: &str, value: &str) -> Result<(), String> {
        self.handle
            .set_toolbar_path_value(id, value)
            .map_err(|err| err.to_string())
    }

    pub fn finish_workflow(&self) -> Result<(), String> {
        self.handle.finish_workflow()
    }

    pub fn cancel_workflow(&self) -> Result<(), String> {
        self.handle.cancel_workflow()
    }

    pub fn suspend_workflow(&self) -> Result<bool, String> {
        self.handle.suspend_workflow()
    }

    pub fn resume_workflow(&self) -> Result<ResumeWorkflow, String> {
        self.handle.resume_workflow()
    }

    pub fn clear_session_workflow(&self) {
        let _ = self.handle.set_session_workflow(None, None);
        self.handle.inner.borrow_mut().active_workflow = None;
    }

    pub fn toolbar_snapshot(&self) -> Option<(String, Vec<ToolbarItem>)> {
        self.handle.toolbar_snapshot()
    }

    pub fn active_workflow_name(&self) -> Option<String> {
        self.handle.inner.borrow().active_workflow.clone()
    }

    pub fn fire_loaded(&self, id: DocumentId, elapsed: f64) {
        let hooks = self.handle.inner.borrow().loaded.clone();
        let handle = LuaComposition { id };
        for hook in hooks {
            if let Err(err) = hook.call::<()>((handle, elapsed)) {
                self.handle
                    .inner
                    .borrow_mut()
                    .prints
                    .push(format!("loaded hook error: {err}"));
            }
        }
    }

    pub fn fire_saved(&self, id: DocumentId, elapsed: f64) {
        let hooks = self.handle.inner.borrow().saved.clone();
        let handle = LuaComposition { id };
        for hook in hooks {
            if let Err(err) = hook.call::<()>((handle, elapsed)) {
                self.handle
                    .inner
                    .borrow_mut()
                    .prints
                    .push(format!("saved hook error: {err}"));
            }
        }
    }

    pub fn fire_session_loaded(&self) {
        let hooks = self.handle.inner.borrow().session_loaded.clone();
        for hook in hooks {
            if let Err(err) = hook.call::<()>(LuaSession::active()) {
                self.handle
                    .inner
                    .borrow_mut()
                    .prints
                    .push(format!("session_loaded hook error: {err}"));
            }
        }
    }

    pub fn fire_session_saved(&self) {
        let hooks = self.handle.inner.borrow().session_saved.clone();
        for hook in hooks {
            if let Err(err) = hook.call::<()>(LuaSession::active()) {
                self.handle
                    .inner
                    .borrow_mut()
                    .prints
                    .push(format!("session_saved hook error: {err}"));
            }
        }
    }

    pub fn fire_detect_layout(&self, id: DocumentId) {
        self.handle.fire_detect_layout(id);
    }

    pub fn layout_names(&self) -> Vec<String> {
        self.handle.layout_names()
    }

    pub fn layout_choices(&self) -> Vec<(String, String)> {
        self.handle.layout_choices()
    }

    pub fn layout(&self, name: &str) -> Option<super::layout::ChannelLayoutDef> {
        self.handle.layout(name)
    }

    pub fn choose_layout(&self, id: DocumentId, name: Option<&str>) -> mlua::Result<()> {
        self.handle.choose_layout(id, name)
    }

    pub fn take_prints(&self) -> Vec<String> {
        std::mem::take(&mut self.handle.inner.borrow_mut().prints)
    }

    pub fn take_logs(&self) -> Vec<LogEntry> {
        std::mem::take(&mut self.handle.inner.borrow_mut().logs)
    }

    pub fn take_alerts(&self) -> Vec<(String, String)> {
        std::mem::take(&mut self.handle.inner.borrow_mut().alerts)
    }

    pub fn workflow_metas(&self) -> Vec<WorkflowMeta> {
        self.handle.workflow_metas()
    }

    pub fn drop_layout(&self) -> DropLayout {
        layout_drop_targets(&self.handle.workflow_metas(), SCOPE_DRAG_DROP)
    }

    pub fn log(&self, level: LogLevel, topic: String, message: String) {
        self.handle.log(level, topic, message);
    }
}

impl HostHandle {
    pub fn active(&self) -> Option<DocumentId> {
        if let Some(test) = &self.inner.borrow().test {
            return test.borrow().active;
        }
        access::with_view(|view, _, _| view.session_active())
            .ok()
            .flatten()
    }

    pub fn set_active(&self, id: DocumentId) -> mlua::Result<()> {
        if let Some(test) = self.inner.borrow().test.clone() {
            let mut world = test.borrow_mut();
            if world.session.get(id).is_none() {
                return Err(mlua::Error::runtime("composition is not open"));
            }
            world.session.focus(id);
            world.active = Some(id);
            return Ok(());
        }
        access::with_view(|view, window, cx| view.script_activate_document(id, window, cx))
            .map_err(mlua::Error::runtime)?
            .map_err(mlua::Error::runtime)
    }

    pub fn looping(&self) -> bool {
        if let Some(test) = &self.inner.borrow().test {
            return test.borrow().looping;
        }
        access::with_view(|view, _, _| view.playback_looping()).unwrap_or(false)
    }

    pub fn preview(&self) -> bool {
        if let Some(test) = &self.inner.borrow().test {
            return test.borrow().preview;
        }
        access::with_view(|view, _, _| view.preview_enabled()).unwrap_or(false)
    }

    pub fn explorer(&self) -> bool {
        if let Some(test) = &self.inner.borrow().test {
            return test.borrow().explorer;
        }
        access::with_view(|view, _, cx| view.explorer_dock_open(cx)).unwrap_or(false)
    }

    pub fn documents(&self) -> Vec<DocumentId> {
        if let Some(test) = &self.inner.borrow().test {
            return test
                .borrow()
                .session
                .documents()
                .iter()
                .map(|doc| doc.id)
                .collect();
        }
        access::with_view(|view, _, _| view.session_document_ids()).unwrap_or_default()
    }

    pub fn display_name(&self, id: DocumentId) -> Option<String> {
        if let Some(test) = &self.inner.borrow().test {
            if let Some(name) = test.borrow().names.get(&id).cloned() {
                return Some(name);
            }
        }
        if let Ok(Some(name)) = access::with_view(|view, _, cx| view.script_display_name(id, cx)) {
            return Some(name);
        }
        self.lookup_document(id).and_then(|doc| {
            doc.file_path()
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
        })
    }

    pub fn path(&self, id: DocumentId) -> Option<PathBuf> {
        if let Some(test) = &self.inner.borrow().test {
            if let Some(path) = test.borrow().paths.get(&id).cloned().flatten() {
                return Some(path);
            }
        }
        if let Ok(Some(path)) = access::with_view(|view, _, _| view.script_path(id)) {
            return Some(path);
        }
        self.lookup_document(id)
            .and_then(|doc| doc.file_path().map(Path::to_path_buf))
    }

    pub fn open(&self, path: &str) -> mlua::Result<DocumentId> {
        let path = PathBuf::from(path);
        if let Some(test) = self.inner.borrow().test.clone() {
            return open_in_test(&test, &path);
        }
        access::with_view(|view, window, cx| view.script_open(path, window, cx))
            .map_err(mlua::Error::runtime)?
            .map_err(mlua::Error::runtime)
    }

    pub fn replace_composition(&self, id: DocumentId, path: &str) -> mlua::Result<DocumentId> {
        let path = PathBuf::from(path);
        if is_fasession_path(&path) {
            return Err(mlua::Error::runtime(
                "cannot replace a composition with a session file",
            ));
        }
        if let Some(test) = self.inner.borrow().test.clone() {
            return replace_in_test(&test, id, &path);
        }
        access::with_view(|view, window, cx| view.script_replace_document(id, path, window, cx))
            .map_err(mlua::Error::runtime)?
            .map_err(mlua::Error::runtime)
    }

    pub fn command(&self, id: &str) -> Result<(), String> {
        crate::commands::validate_command_id(id)?;
        if let Some(test) = self.inner.borrow().test.clone() {
            let mut world = test.borrow_mut();
            match id {
                "transport.loop" => world.looping = !world.looping,
                "transport.preview" => world.preview = !world.preview,
                "view.show-explorer" => world.explorer = true,
                "view.hide-explorer" => world.explorer = false,
                "view.toggle-explorer" => world.explorer = !world.explorer,
                "view.show-detail" => world.detail = true,
                "view.hide-detail" => world.detail = false,
                "view.toggle-detail" => world.detail = !world.detail,
                "view.show-script" => world.script = true,
                "view.hide-script" => world.script = false,
                "view.toggle-script" => world.script = !world.script,
                _ => {}
            }
            return Ok(());
        }
        match access::with_view(|view, window, cx| view.invoke_command(id, window, cx)) {
            Ok(result) => result,
            Err(err) => Err(err),
        }
    }

    pub fn output_device(&self) -> Option<String> {
        if let Some(test) = &self.inner.borrow().test {
            return test.borrow().output_device.clone();
        }
        access::with_view(|view, _, _| view.output_device().map(str::to_string))
            .ok()
            .flatten()
    }

    pub fn output_devices(&self) -> Vec<String> {
        if let Some(test) = &self.inner.borrow().test {
            return test.borrow().output_devices.clone();
        }
        crate::playback::list_output_devices()
            .map(|devices| devices.into_iter().map(|info| info.name).collect())
            .unwrap_or_default()
    }

    pub fn set_output_device(&self, spec: Option<&str>) -> mlua::Result<()> {
        if let Some(test) = &self.inner.borrow().test {
            test.borrow_mut().output_device = spec.map(str::to_string);
            return Ok(());
        }
        access::with_view(|view, window, cx| view.set_output_device(spec, window, cx))
            .map_err(mlua::Error::runtime)?
            .map_err(mlua::Error::runtime)
    }

    pub fn on_loaded(&self, callback: Function) {
        self.inner.borrow_mut().loaded.push(callback);
    }

    pub fn on_saved(&self, callback: Function) {
        self.inner.borrow_mut().saved.push(callback);
    }

    pub fn on_session_loaded(&self, callback: Function) {
        self.inner.borrow_mut().session_loaded.push(callback);
    }

    pub fn on_session_saved(&self, callback: Function) {
        self.inner.borrow_mut().session_saved.push(callback);
    }

    pub fn on_detect_layout(&self, callback: Function) {
        self.inner.borrow_mut().detect_layout.push(callback);
    }

    pub fn log(&self, level: LogLevel, topic: String, message: String) {
        let entry = LogEntry {
            level,
            topic,
            message,
        };
        self.write_console(&entry);
        self.inner.borrow_mut().logs.push(entry);
    }

    fn write_console(&self, entry: &LogEntry) {
        if self.inner.borrow().test.is_some() {
            return;
        }
        let stdout = io::stdout();
        if !stdout.is_terminal() {
            return;
        }
        let mut out = stdout.lock();
        let _ = writeln!(out, "{}", format_console_line(entry));
    }

    pub fn define_layout(&self, layout: ChannelLayoutDef) {
        let mut inner = self.inner.borrow_mut();
        if let Some(existing) = inner
            .layouts
            .iter_mut()
            .find(|defined| defined.name == layout.name)
        {
            *existing = layout;
        } else {
            inner.layouts.push(layout);
        }
    }

    pub fn layout(&self, name: &str) -> Option<ChannelLayoutDef> {
        self.inner
            .borrow()
            .layouts
            .iter()
            .find(|layout| layout.name == name)
            .cloned()
    }

    pub fn layout_names(&self) -> Vec<String> {
        self.inner
            .borrow()
            .layouts
            .iter()
            .map(|layout| layout.name.clone())
            .collect()
    }

    pub fn layout_choices(&self) -> Vec<(String, String)> {
        self.inner
            .borrow()
            .layouts
            .iter()
            .map(|layout| (layout.name.clone(), layout.description.clone()))
            .collect()
    }

    pub fn apply_effective_layout(&self, id: DocumentId, name: Option<&str>) -> mlua::Result<()> {
        let (labels, default_chain) = match name {
            Some(name) => {
                let layout = self.layout(name);
                let labels = layout
                    .as_ref()
                    .map(|layout| layout.channels.clone())
                    .unwrap_or_default();
                let chain = layout
                    .as_ref()
                    .and_then(|layout| layout.monitor_chain_id().map(str::to_string));
                (labels, chain)
            }
            None => (BTreeMap::new(), None),
        };
        self.with_document(id, |doc| {
            let mut composition = doc.composition.write().unwrap();
            composition.apply_channel_layout(name.map(str::to_string), labels);
            if composition.monitor_chain().is_none() {
                if let Some(chain) = default_chain {
                    composition.set_monitor_chain(Some(chain));
                }
            }
            Ok(())
        })
    }

    pub fn choose_layout(&self, id: DocumentId, name: Option<&str>) -> mlua::Result<()> {
        let (labels, default_chain) = match name {
            Some(name) => {
                let layout = self.layout(name).ok_or_else(|| {
                    mlua::Error::runtime(format!("unknown channel layout `{name}`"))
                })?;
                let chain = layout.monitor_chain_id().map(str::to_string);
                (layout.channels, chain)
            }
            None => (BTreeMap::new(), None),
        };
        self.with_document(id, |doc| {
            let mut composition = doc.composition.write().unwrap();
            composition.choose_channel_layout(name.map(str::to_string), labels);
            composition.set_monitor_chain(default_chain);
            Ok(())
        })
    }

    pub fn fire_detect_layout(&self, id: DocumentId) {
        let chosen = self
            .with_document(id, |doc| {
                Ok(doc
                    .composition
                    .read()
                    .unwrap()
                    .chosen_channel_layout()
                    .map(str::to_string))
            })
            .ok()
            .flatten();
        let hooks = self.inner.borrow().detect_layout.clone();
        let handle = LuaComposition { id };
        let mut last = None;
        for hook in hooks {
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
                Err(err) => {
                    self.inner
                        .borrow_mut()
                        .prints
                        .push(format!("detect_layout hook error: {err}"));
                }
            }
        }
        if let Err(err) = self.apply_effective_layout(id, last.as_deref()) {
            self.inner
                .borrow_mut()
                .prints
                .push(format!("detect_layout hook error: {err}"));
        }
    }

    pub fn file_backed_path(&self, id: DocumentId) -> Option<PathBuf> {
        let media = self
            .with_document(id, |doc| {
                Ok(doc
                    .composition
                    .read()
                    .unwrap()
                    .pool()
                    .first()
                    .map(|media| media.path.clone()))
            })
            .ok()
            .flatten();
        if let Some(path) = media.filter(|path| is_file_backed(path)) {
            return Some(path);
        }
        self.path(id).filter(|path| is_file_backed(path))
    }

    pub fn session_id(&self, which: Option<SessionId>) -> String {
        self.with_session_kind(which, |session| session.id().to_string())
            .unwrap_or_default()
    }

    pub fn session_path(&self, which: Option<SessionId>) -> Option<String> {
        self.with_session_kind(which, |session| {
            session.path().map(|path| path.display().to_string())
        })
        .flatten()
    }

    pub fn session_workflow(&self, which: Option<SessionId>) -> Option<String> {
        self.with_session_kind(which, |session| session.workflow().map(str::to_string))
            .flatten()
    }

    pub fn set_session_workflow(
        &self,
        which: Option<SessionId>,
        workflow: Option<String>,
    ) -> mlua::Result<()> {
        self.with_session_kind_mut(which, |session| {
            session.set_workflow(workflow);
            Ok(())
        })
    }

    pub fn session_capture_ui(&self, which: Option<SessionId>) -> bool {
        self.with_session_kind(which, |session| session.capture_ui())
            .unwrap_or(true)
    }

    pub fn set_session_capture_ui(
        &self,
        which: Option<SessionId>,
        capture_ui: bool,
    ) -> mlua::Result<()> {
        self.with_session_kind_mut(which, |session| {
            session.set_capture_ui(capture_ui);
            Ok(())
        })
    }

    pub fn session_properties(&self, which: Option<SessionId>) -> BTreeMap<String, String> {
        self.with_session_kind(which, |session| session.properties().clone())
            .unwrap_or_default()
    }

    pub fn set_session_properties(
        &self,
        which: Option<SessionId>,
        properties: BTreeMap<String, String>,
    ) -> mlua::Result<()> {
        self.with_session_kind_mut(which, |session| {
            session.set_properties(properties);
            Ok(())
        })
    }

    pub fn session_active_document(&self, which: Option<SessionId>) -> Option<DocumentId> {
        self.with_session_kind(which, |session| session.active())
            .flatten()
    }

    pub fn session_documents(&self, which: Option<SessionId>) -> Vec<DocumentId> {
        self.with_session_kind(which, |session| {
            session.documents().iter().map(|doc| doc.id).collect()
        })
        .unwrap_or_default()
    }

    pub fn session_group_count(&self, which: Option<SessionId>, group: &str) -> usize {
        self.with_session_kind(which, |session| session.group_count(group))
            .unwrap_or(0)
    }

    /// Move `id` to 1-based `index` within the chosen session's document list.
    pub fn move_session_document(
        &self,
        which: Option<SessionId>,
        id: DocumentId,
        index: i64,
    ) -> mlua::Result<()> {
        self.with_session_kind_mut(which, |session| {
            let len = session.len() as i64;
            if index < 1 || index > len {
                return Err(mlua::Error::runtime(format!(
                    "move index must be between 1 and {len}"
                )));
            }
            if !session.move_document(id, (index - 1) as usize) {
                return Err(mlua::Error::runtime("composition is not in this session"));
            }
            Ok(())
        })?;
        if which.is_none() {
            self.refresh_explorer();
        }
        Ok(())
    }

    pub fn sessions(&self) -> Vec<LuaSession> {
        let mut sessions = vec![LuaSession::active()];
        let mut detached: Vec<SessionId> = self
            .inner
            .borrow()
            .detached_sessions
            .keys()
            .copied()
            .collect();
        detached.sort_by_key(|id| id.0);
        sessions.extend(detached.into_iter().map(|id| LuaSession { id: Some(id) }));
        sessions
    }

    pub fn load_session(&self, path: &str) -> mlua::Result<SessionId> {
        let path = PathBuf::from(path);
        if !is_fasession_path(&path) {
            return Err(mlua::Error::runtime(
                "load_session expects a .fasession file",
            ));
        }
        let json =
            std::fs::read_to_string(&path).map_err(|err| mlua::Error::runtime(err.to_string()))?;
        let loaded = Session::from_json(&json, Some(&path))
            .map_err(|err| mlua::Error::runtime(err.to_string()))?;
        let id = loaded.session.id();
        self.inner
            .borrow_mut()
            .detached_sessions
            .insert(id, loaded.session);
        Ok(id)
    }

    pub fn close_session(&self, which: Option<SessionId>) -> mlua::Result<()> {
        let Some(id) = which else {
            return Err(mlua::Error::runtime("cannot close the active session"));
        };
        if self
            .inner
            .borrow_mut()
            .detached_sessions
            .remove(&id)
            .is_none()
        {
            return Err(mlua::Error::runtime("session is not loaded"));
        }
        Ok(())
    }

    pub fn declare_workflow(&self, workflow: WorkflowDef) {
        self.inner
            .borrow_mut()
            .workflows
            .insert(workflow.meta.name.clone(), workflow);
    }

    pub fn toolbar_changed(&self, _prototype: &mlua::Table) -> mlua::Result<()> {
        if self.inner.borrow().test.is_some() {
            return Ok(());
        }
        let _ = access::with_view(|view, _, cx| view.refresh_workflow_bar(cx));
        Ok(())
    }

    pub fn dispatch_workflow_command(&self, command: &str) -> Result<(), String> {
        let Some(name) = self.inner.borrow().active_workflow.clone() else {
            return Ok(());
        };
        let proto = self
            .inner
            .borrow()
            .workflows
            .get(&name)
            .map(|workflow| workflow.prototype.clone())
            .ok_or_else(|| format!("unknown workflow `{name}`"))?;
        let handler = match proto.get::<Value>("__fa_command") {
            Ok(Value::Function(func)) => func,
            _ => return Ok(()),
        };
        handler.call::<()>(command).map_err(|err| err.to_string())
    }

    pub fn dispatch_toolbar_path(
        &self,
        lua: &Lua,
        id: &str,
        paths: &[PathBuf],
    ) -> mlua::Result<String> {
        let row = self.toolbar_row(id)?;
        let value = match row.get::<Value>("on_path")? {
            Value::Function(func) => {
                let table = lua.create_table()?;
                for (index, path) in paths.iter().enumerate() {
                    table.set(index + 1, path.display().to_string())?;
                }
                match func.call::<Value>(table)? {
                    Value::Nil => first_path_string(paths),
                    Value::String(text) => text.to_str()?.to_owned(),
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "on_path must return a string or nil, got {}",
                            other.type_name()
                        )))
                    }
                }
            }
            Value::Nil => first_path_string(paths),
            other => {
                return Err(mlua::Error::runtime(format!(
                    "on_path must be a function, got {}",
                    other.type_name()
                )))
            }
        };
        row.set("value", value.as_str())?;
        Ok(value)
    }

    pub fn dispatch_toolbar_toggle(&self, id: &str) -> mlua::Result<bool> {
        let row = self.toolbar_row(id)?;
        let current = match row.get::<Value>("value")? {
            Value::Nil => false,
            Value::Boolean(value) => value,
            other => {
                return Err(mlua::Error::runtime(format!(
                    "toggle value must be a boolean, got {}",
                    other.type_name()
                )))
            }
        };
        let next = match row.get::<Value>("on_change")? {
            Value::Function(func) => match func.call::<Value>(current)? {
                Value::Boolean(value) => value,
                Value::Nil => !current,
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "on_change must return a boolean, got {}",
                        other.type_name()
                    )))
                }
            },
            Value::Nil => !current,
            other => {
                return Err(mlua::Error::runtime(format!(
                    "on_change must be a function, got {}",
                    other.type_name()
                )))
            }
        };
        row.set("value", next)?;
        Ok(next)
    }

    pub fn set_toolbar_path_value(&self, id: &str, value: &str) -> mlua::Result<()> {
        let row = self.toolbar_row(id)?;
        row.set("value", value)?;
        Ok(())
    }

    fn toolbar_row(&self, id: &str) -> mlua::Result<mlua::Table> {
        let Some(name) = self.inner.borrow().active_workflow.clone() else {
            return Err(mlua::Error::runtime("no workflow is running"));
        };
        let proto = self
            .inner
            .borrow()
            .workflows
            .get(&name)
            .map(|workflow| workflow.prototype.clone())
            .ok_or_else(|| mlua::Error::runtime(format!("unknown workflow `{name}`")))?;
        let toolbar = match proto.get::<Value>("__fa_toolbar")? {
            Value::Table(table) => table,
            _ => return Err(mlua::Error::runtime("workflow has no toolbar")),
        };
        for row in toolbar.sequence_values::<Value>() {
            let Value::Table(row) = row? else {
                continue;
            };
            if toolbar_row_id(&row)?.as_deref() == Some(id) {
                return Ok(row);
            }
        }
        Err(mlua::Error::runtime(format!("no toolbar item `{id}`")))
    }

    pub fn finish_workflow(&self) -> Result<(), String> {
        self.end_active_workflow("finish")
    }

    pub fn cancel_workflow(&self) -> Result<(), String> {
        self.end_active_workflow("cancel")
    }

    fn end_active_workflow(&self, method: &str) -> Result<(), String> {
        let Some(name) = self.inner.borrow().active_workflow.clone() else {
            return Ok(());
        };
        let proto = self
            .inner
            .borrow()
            .workflows
            .get(&name)
            .map(|workflow| workflow.prototype.clone());
        if let Some(proto) = proto {
            if let Some(func) = prototype_method(&proto, method) {
                func.call::<()>((proto, LuaSession::active()))
                    .map_err(|err| err.to_string())?;
            }
        }
        self.inner.borrow_mut().active_workflow = None;
        let _ = self.set_session_workflow(None, None);
        Ok(())
    }

    fn bind_active_workflow(&self, name: &str) {
        self.inner.borrow_mut().active_workflow = Some(name.to_string());
        let _ = self.set_session_workflow(None, Some(name.to_string()));
    }

    pub fn suspend_workflow(&self) -> Result<bool, String> {
        let Some(name) = self.inner.borrow().active_workflow.clone() else {
            return Ok(true);
        };
        let Some(proto) = self
            .inner
            .borrow()
            .workflows
            .get(&name)
            .map(|workflow| workflow.prototype.clone())
        else {
            return Ok(true);
        };
        let Some(func) = prototype_method(&proto, "suspend") else {
            return Ok(true);
        };
        let result: Value = func
            .call((proto, LuaSession::active()))
            .map_err(|err| err.to_string())?;
        match result {
            Value::Nil | Value::Boolean(true) => Ok(true),
            Value::Boolean(false) => Ok(false),
            other => Err(format!(
                "suspend must return a boolean or nil, got {}",
                other.type_name()
            )),
        }
    }

    pub fn resume_workflow(&self) -> Result<ResumeWorkflow, String> {
        let Some(name) = self.session_workflow(None) else {
            self.inner.borrow_mut().active_workflow = None;
            return Ok(ResumeWorkflow::None);
        };
        let proto = self
            .inner
            .borrow()
            .workflows
            .get(&name)
            .map(|workflow| workflow.prototype.clone());
        let Some(proto) = proto else {
            self.inner.borrow_mut().active_workflow = None;
            self.log(
                LogLevel::Error,
                "workflow".into(),
                format!("unknown workflow `{name}`"),
            );
            return Ok(ResumeWorkflow::Unknown { name });
        };
        if !prototype_is_stateful(&proto) {
            self.inner.borrow_mut().active_workflow = None;
            self.log(
                LogLevel::Error,
                "workflow".into(),
                format!("workflow `{name}` is not stateful"),
            );
            return Ok(ResumeWorkflow::Unknown { name });
        }
        if let Some(func) = prototype_method(&proto, "resume") {
            func.call::<()>((proto, LuaSession::active()))
                .map_err(|err| err.to_string())?;
        }
        self.inner.borrow_mut().active_workflow = Some(name);
        Ok(ResumeWorkflow::Resumed)
    }

    pub fn toolbar_snapshot(&self) -> Option<(String, Vec<ToolbarItem>)> {
        let name = self.inner.borrow().active_workflow.clone()?;
        let workflow = self.inner.borrow().workflows.get(&name).map(|workflow| {
            (
                workflow.meta.display_name.clone(),
                workflow.prototype.clone(),
            )
        })?;
        let items = toolbar_from_prototype(&workflow.1);
        if items.is_empty() {
            return None;
        }
        Some((workflow.0, items))
    }

    pub fn workflow_metas(&self) -> Vec<WorkflowMeta> {
        self.inner
            .borrow()
            .workflows
            .values()
            .map(|workflow| workflow.meta.clone())
            .collect()
    }

    pub fn alert(&self, subject: String, body: String) -> mlua::Result<()> {
        if self.inner.borrow().test.is_some() {
            self.inner.borrow_mut().alerts.push((subject, body));
            return Ok(());
        }
        access::with_view(|view, window, cx| view.script_alert(&subject, &body, window, cx))
            .map_err(mlua::Error::runtime)
    }

    pub fn composition_id(&self, id: DocumentId) -> mlua::Result<String> {
        self.require_document(id)?;
        Ok(id.to_string())
    }

    pub fn document_group(&self, id: DocumentId) -> mlua::Result<Option<String>> {
        self.lookup_document(id)
            .map(|doc| doc.group)
            .ok_or_else(|| mlua::Error::runtime("composition is not open"))
    }

    pub fn set_document_group(&self, id: DocumentId, group: Option<String>) -> mlua::Result<()> {
        self.with_document_session_mut(id, |session| {
            if session.set_document_group(id, group) {
                Ok(())
            } else {
                Err(mlua::Error::runtime("composition is not open"))
            }
        })?;
        self.refresh_explorer();
        Ok(())
    }

    pub fn refresh_explorer(&self) {
        if self.inner.borrow().test.is_some() {
            return;
        }
        let _ = access::with_view(|view, _, cx| view.refresh_explorer(cx));
    }

    pub fn document_state(&self, id: DocumentId) -> mlua::Result<Option<String>> {
        self.lookup_document(id)
            .map(|doc| doc.state)
            .ok_or_else(|| mlua::Error::runtime("composition is not open"))
    }

    pub fn set_document_state(&self, id: DocumentId, state: Option<String>) -> mlua::Result<()> {
        self.with_document_session_mut(id, |session| {
            if session.set_document_state(id, state) {
                Ok(())
            } else {
                Err(mlua::Error::runtime("composition is not open"))
            }
        })
    }

    pub fn document_properties(&self, id: DocumentId) -> mlua::Result<BTreeMap<String, String>> {
        self.lookup_document(id)
            .map(|doc| doc.properties)
            .ok_or_else(|| mlua::Error::runtime("composition is not open"))
    }

    pub fn set_document_properties(
        &self,
        id: DocumentId,
        properties: BTreeMap<String, String>,
    ) -> mlua::Result<()> {
        self.with_document_session_mut(id, |session| {
            if session.set_document_properties(id, properties) {
                Ok(())
            } else {
                Err(mlua::Error::runtime("composition is not open"))
            }
        })
    }

    pub fn save_session(&self, path: Option<String>) -> mlua::Result<()> {
        if let Some(test) = &self.inner.borrow().test {
            let mut world = test.borrow_mut();
            let names = world.names.clone();
            let dest = match path {
                Some(path) => PathBuf::from(path),
                None => world
                    .session
                    .path()
                    .map(Path::to_path_buf)
                    .ok_or_else(|| mlua::Error::runtime("session has no path; call save_as"))?,
            };
            world
                .session
                .save_to_path(
                    &dest,
                    |id| names.get(&id).cloned().unwrap_or_else(|| id.to_string()),
                    None,
                )
                .map_err(|err| mlua::Error::runtime(err.to_string()))?;
            return Ok(());
        }
        access::with_view(|view, window, cx| match path {
            Some(path) => view.script_save_session_to(PathBuf::from(path), window, cx),
            None => view.script_save_session(window, cx),
        })
        .map_err(mlua::Error::runtime)?
        .map_err(mlua::Error::runtime)
    }

    pub fn save_composition(&self, id: DocumentId) -> mlua::Result<()> {
        self.require_document(id)?;
        if self.inner.borrow().test.is_some() {
            return Ok(());
        }
        access::with_view(|view, window, cx| view.script_save_document(id, window, cx))
            .map_err(mlua::Error::runtime)?
            .map_err(mlua::Error::runtime)
    }

    pub fn close_composition(&self, id: DocumentId) -> mlua::Result<()> {
        if let Some(test) = &self.inner.borrow().test {
            let mut world = test.borrow_mut();
            if !world.docs.contains_key(&id) {
                return Err(mlua::Error::runtime("composition is not open"));
            }
            world.close(id);
            return Ok(());
        }
        access::with_view(|view, window, cx| view.script_close_document(id, window, cx))
            .map_err(mlua::Error::runtime)
    }

    fn require_document(&self, id: DocumentId) -> mlua::Result<()> {
        if self.lookup_document(id).is_some() {
            return Ok(());
        }
        Err(mlua::Error::runtime("composition is not open"))
    }

    fn lookup_document(&self, id: DocumentId) -> Option<SessionDocument> {
        if let Some(doc) = self
            .with_session(|session| session.get(id).cloned())
            .flatten()
        {
            return Some(doc);
        }
        let inner = self.inner.borrow();
        for session in inner.detached_sessions.values() {
            if let Some(doc) = session.get(id) {
                return Some(doc.clone());
            }
        }
        None
    }

    fn with_session_kind<R>(
        &self,
        which: Option<SessionId>,
        f: impl FnOnce(&Session) -> R,
    ) -> Option<R> {
        match which {
            None => self.with_session(f),
            Some(id) => {
                if let Some(session) = self.inner.borrow().detached_sessions.get(&id).cloned() {
                    return Some(f(&session));
                }
                self.with_session(|session| {
                    if session.id() == id {
                        Some(f(session))
                    } else {
                        None
                    }
                })
                .flatten()
            }
        }
    }

    fn with_session_kind_mut<R>(
        &self,
        which: Option<SessionId>,
        f: impl FnOnce(&mut Session) -> mlua::Result<R>,
    ) -> mlua::Result<R> {
        match which {
            None => self.with_session_mut(f),
            Some(id) => {
                {
                    let mut inner = self.inner.borrow_mut();
                    if let Some(session) = inner.detached_sessions.get_mut(&id) {
                        return f(session);
                    }
                }
                self.with_session_mut(|session| {
                    if session.id() == id {
                        f(session)
                    } else {
                        Err(mlua::Error::runtime("session is not loaded"))
                    }
                })
            }
        }
    }

    fn with_document_session_mut<R>(
        &self,
        id: DocumentId,
        f: impl FnOnce(&mut Session) -> mlua::Result<R>,
    ) -> mlua::Result<R> {
        let in_active = self
            .with_session(|session| session.get(id).is_some())
            .unwrap_or(false);
        if in_active {
            return self.with_session_mut(f);
        }
        let mut inner = self.inner.borrow_mut();
        if let Some(session) = inner
            .detached_sessions
            .values_mut()
            .find(|session| session.get(id).is_some())
        {
            return f(session);
        }
        Err(mlua::Error::runtime("composition is not open"))
    }

    fn with_session<R>(&self, f: impl FnOnce(&Session) -> R) -> Option<R> {
        if let Some(test) = &self.inner.borrow().test {
            return Some(f(&test.borrow().session));
        }
        access::with_view(|view, _, _| f(view.session())).ok()
    }

    fn with_session_mut<R>(
        &self,
        f: impl FnOnce(&mut Session) -> mlua::Result<R>,
    ) -> mlua::Result<R> {
        if let Some(test) = &self.inner.borrow().test {
            return f(&mut test.borrow_mut().session);
        }
        access::with_view(|view, _, _| f(view.session_mut())).map_err(mlua::Error::runtime)?
    }

    pub fn after_edit(&self, id: DocumentId) -> mlua::Result<()> {
        if self.inner.borrow().test.is_some() {
            return Ok(());
        }
        access::with_view(|view, window, cx| view.after_script_edit(id, window, cx))
            .map_err(mlua::Error::runtime)
    }

    pub fn with_document<R>(
        &self,
        id: DocumentId,
        f: impl FnOnce(&mut BufferDocument) -> mlua::Result<R>,
    ) -> mlua::Result<R> {
        if let Some(test) = &self.inner.borrow().test {
            let mut world = test.borrow_mut();
            let doc = world
                .docs
                .get_mut(&id)
                .ok_or_else(|| mlua::Error::runtime("composition is not open"))?;
            return f(doc);
        }
        access::with_view(|view, _, cx| view.script_with_document(id, cx, f))
            .map_err(mlua::Error::runtime)?
    }
}

pub fn host_from_lua(lua: &Lua) -> mlua::Result<HostHandle> {
    lua.app_data_ref::<HostHandle>()
        .map(|handle| handle.clone())
        .ok_or_else(|| mlua::Error::runtime("script host is not bound"))
}

pub fn with_document<R>(
    lua: &Lua,
    id: DocumentId,
    f: impl FnOnce(&mut BufferDocument) -> mlua::Result<R>,
) -> mlua::Result<R> {
    host_from_lua(lua)?.with_document(id, f)
}

fn install_print(lua: &Lua, handle: HostHandle) -> mlua::Result<()> {
    let print = lua.create_function(move |lua, args: MultiValue| {
        let text = stringify_values(lua, args).unwrap_or_default();
        handle.inner.borrow_mut().prints.push(text);
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
    match lua
        .load(&format!("return {trimmed}"))
        .set_name("=repl")
        .eval::<MultiValue>()
    {
        Ok(values) => Ok(values),
        Err(_) => lua.load(trimmed).set_name("=repl").eval::<MultiValue>(),
    }
}

pub fn user_init_path(config_dir: Option<&Path>) -> Option<PathBuf> {
    let path = config_dir?.join("init.lua");
    path.is_file().then_some(path)
}

fn is_workflow_filename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("workflow_") && lower.ends_with(".lua")
}

fn first_path_string(paths: &[PathBuf]) -> String {
    paths
        .first()
        .map(|path| path.display().to_string())
        .unwrap_or_default()
}

fn open_in_test(test: &Rc<RefCell<TestWorld>>, path: &Path) -> mlua::Result<DocumentId> {
    if is_fasession_path(path) {
        let json =
            std::fs::read_to_string(path).map_err(|err| mlua::Error::runtime(err.to_string()))?;
        let loaded = Session::from_json(&json, Some(path))
            .map_err(|err| mlua::Error::runtime(err.to_string()))?;
        let mut world = test.borrow_mut();
        world.docs.clear();
        world.paths.clear();
        world.names.clear();
        world.session = loaded.session;
        let docs: Vec<SessionDocument> = world.session.documents().to_vec();
        for doc in docs {
            let name = doc
                .file_path()
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| doc.id.to_string());
            let composition = Composition::new(44100, 2);
            let composition = Arc::new(RwLock::new(composition));
            let buffer = Arc::new(RwLock::new(Buffer::empty()));
            let document = BufferDocument::with_shared(composition, buffer);
            world.docs.insert(doc.id, document);
            world
                .paths
                .insert(doc.id, doc.file_path().map(Path::to_path_buf));
            world.names.insert(doc.id, name);
        }
        world.active = world.session.active();
        return world
            .active
            .ok_or_else(|| mlua::Error::runtime("session has no documents"));
    }
    if let Some(id) = test.borrow().session.find_by_path(path) {
        test.borrow_mut().active = Some(id);
        test.borrow_mut().session.focus(id);
        return Ok(id);
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let mut world = test.borrow_mut();
    let id = world.push(
        Composition::new(44100, 2),
        Buffer::empty(),
        name,
        Some(path.to_path_buf()),
    );
    if is_facomp_path(path) {
        world.session.set_project_path(id, path.to_path_buf());
    }
    Ok(id)
}

fn replace_in_test(
    test: &Rc<RefCell<TestWorld>>,
    id: DocumentId,
    path: &Path,
) -> mlua::Result<DocumentId> {
    if test.borrow().session.get(id).is_none() {
        return Err(mlua::Error::runtime("composition is not open"));
    }
    if let Some(existing) = test.borrow().session.find_by_path(path) {
        test.borrow_mut().active = Some(existing);
        test.borrow_mut().session.focus(existing);
        return Ok(existing);
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let mut world = test.borrow_mut();
    world.session.replace_document_path(id, path.to_path_buf());
    world.paths.insert(id, Some(path.to_path_buf()));
    world.names.insert(id, name);
    world.active = Some(id);
    world.session.focus(id);
    Ok(id)
}

fn is_file_backed(path: &Path) -> bool {
    !path.to_string_lossy().starts_with("memory:")
}

fn stringify_values(lua: &Lua, values: MultiValue) -> Option<String> {
    if values.is_empty() {
        return None;
    }
    let tostring: Function = match lua.globals().get("tostring") {
        Ok(f) => f,
        Err(_) => return None,
    };
    let mut parts = Vec::new();
    for value in values {
        if matches!(value, Value::Nil) && parts.is_empty() {
            continue;
        }
        match tostring.call::<String>(value) {
            Ok(text) => parts.push(text),
            Err(_) => parts.push("<unprintable>".into()),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\t"))
    }
}

pub fn stringify_value(lua: &Lua, value: Value) -> String {
    let tostring: Function = match lua.globals().get("tostring") {
        Ok(f) => f,
        Err(_) => return "<unprintable>".into(),
    };
    tostring
        .call::<String>(value)
        .unwrap_or_else(|_| "<unprintable>".into())
}

fn format_console_line(entry: &LogEntry) -> String {
    let color = match entry.level {
        LogLevel::Info => "\x1b[32m",
        LogLevel::Warn => "\x1b[33m",
        LogLevel::Error => "\x1b[31m",
    };
    format!(
        "{color}{:<5}\x1b[0m  {}  {}",
        entry.level.as_str(),
        entry.topic,
        entry.message
    )
}
