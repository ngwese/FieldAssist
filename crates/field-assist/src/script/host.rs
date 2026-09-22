// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, RwLock};

use field_session::{placeholder_media_descriptor, DocumentId, Session, SessionDocument};
use mlua::Lua;

use crate::model::{is_facomp_path, Buffer, BufferDocument, Composition, MediaStore};

pub use field_scripting::EMBEDDED_INIT;

const EMBEDDED_WORKFLOW_ADD: &str = include_str!("../../assets/workflow_add.lua");
const EMBEDDED_WORKFLOW_REPLACE: &str = include_str!("../../assets/workflow_replace.lua");
const EMBEDDED_WORKFLOW_REVIEW: &str = include_str!("../../assets/workflow_review.lua");

/// Test-only desktop model used by the script integration tests.
pub struct TestWorld {
    pub docs: HashMap<DocumentId, BufferDocument>,
    pub paths: HashMap<DocumentId, Option<PathBuf>>,
    pub names: HashMap<DocumentId, String>,
    pub active: Option<DocumentId>,
    pub session: Session,
    pub media_store: Arc<Mutex<MediaStore>>,
    next_id: u128,
    pub output_device: Option<String>,
    pub output_devices: Vec<String>,
    pub theme_name: String,
    pub theme_mode: String,
    pub themes: Vec<String>,
    pub looping: bool,
    pub preview: bool,
    pub explorer: bool,
    pub detail: bool,
    pub script: bool,
}

impl TestWorld {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            docs: HashMap::new(),
            paths: HashMap::new(),
            names: HashMap::new(),
            active: None,
            session: Session::new(),
            media_store: Arc::new(Mutex::new(MediaStore::in_memory())),
            next_id: 0,
            output_device: None,
            output_devices: Vec::new(),
            theme_name: "Default Dark".into(),
            theme_mode: "dark".into(),
            themes: vec!["Default Light".into(), "Default Dark".into()],
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
        for media in composition.pool().iter() {
            self.media_store.lock().unwrap().intern(media.clone());
        }
        let composition = Arc::new(RwLock::new(composition));
        let comp_id = composition.read().unwrap().id();
        let document =
            BufferDocument::with_shared(composition.clone(), Arc::new(RwLock::new(buffer)));
        let session_doc = match path.as_ref() {
            Some(path) if is_facomp_path(path) => {
                SessionDocument::new_composition(id, path.clone(), comp_id)
            }
            Some(path) => SessionDocument::new_media(
                id,
                path.clone(),
                comp_id,
                composition
                    .read()
                    .unwrap()
                    .pool()
                    .first()
                    .map(|m| m.to_descriptor())
                    .unwrap_or_else(|| placeholder_media_descriptor(path)),
            ),
            None => SessionDocument::new_composition(id, PathBuf::new(), comp_id),
        };
        self.docs.insert(id, document);
        self.paths.insert(id, path);
        self.names.insert(id, name.into());
        self.session.insert(session_doc);
        self.active = Some(id);
        id
    }

    pub(crate) fn close(&mut self, id: DocumentId) {
        self.docs.remove(&id);
        self.paths.remove(&id);
        self.names.remove(&id);
        self.session.close_document(id);
        self.active = self.session.active();
    }
}

/// FieldAssist facade over the shared Lua runtime.
pub struct ScriptHost {
    inner: field_scripting::ScriptHost,
}

#[allow(dead_code)]
impl ScriptHost {
    pub fn new() -> mlua::Result<Self> {
        Self::with_backend(super::backend::DesktopBackend::new())
    }
    #[cfg(test)]
    pub fn for_test(world: Rc<RefCell<TestWorld>>) -> mlua::Result<Self> {
        Self::with_backend(super::backend::DesktopBackend::for_test(world))
    }

    fn with_backend(backend: super::backend::DesktopBackend) -> mlua::Result<Self> {
        let concrete = backend.clone();
        let backend: Rc<RefCell<dyn field_scripting::ScriptBackend>> =
            Rc::new(RefCell::new(backend));
        let inner = field_scripting::ScriptHost::with_backend(
            field_scripting::HostProfile {
                name: super::app::HOST_NAME,
                config_dir: crate::commands::user_config_dir(),
            },
            backend,
        )?;
        super::backend::install_lua_backend(inner.lua(), concrete);
        super::app::bind_app(inner.lua())?;
        Ok(Self { inner })
    }
    pub fn lua(&self) -> &Lua {
        self.inner.lua()
    }
    pub fn eval(&mut self, code: &str) -> field_scripting::EvalOutput {
        self.inner.eval(code)
    }
    pub fn load_init(&mut self) -> Result<(), String> {
        self.load_init_from(crate::commands::user_config_dir().as_deref())
    }
    pub fn load_init_from(&mut self, config: Option<&Path>) -> Result<(), String> {
        self.inner.load_init_from(config)?;
        for (source, name) in [
            (EMBEDDED_WORKFLOW_ADD, "workflow_add.lua"),
            (EMBEDDED_WORKFLOW_REPLACE, "workflow_replace.lua"),
            (EMBEDDED_WORKFLOW_REVIEW, "workflow_review.lua"),
        ] {
            self.inner
                .lua()
                .load(source)
                .set_name(format!("@<embedded>/{name}"))
                .exec()
                .map_err(|e| format!("{name}: {e}"))?;
        }
        if let Some(dir) = config {
            let mut files = std::fs::read_dir(dir)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("workflow_") && n.ends_with(".lua"))
                })
                .collect::<Vec<_>>();
            files.sort();
            for path in files {
                self.inner.load_file(&path)?;
            }
        }
        Ok(())
    }
    pub fn invoke_workflow(&self, n: &str, p: &[PathBuf]) -> Result<(), String> {
        self.inner.invoke_workflow(n, p)
    }
    pub fn invoke_menu_workflow(&self, n: &str) -> Result<(), String> {
        self.inner.invoke_menu_workflow(n)
    }
    pub fn menu_workflows(&self) -> Vec<(String, String)> {
        self.inner.menu_workflows()
    }
    pub fn dispatch_toolbar_button(&self, id: &str) -> Result<(), String> {
        self.inner.dispatch_toolbar_button(id)
    }
    pub fn dispatch_toolbar_path(&self, id: &str, p: &[PathBuf]) -> Result<String, String> {
        self.inner.dispatch_toolbar_path(id, p)
    }
    pub fn dispatch_toolbar_toggle(&self, id: &str) -> Result<bool, String> {
        self.inner
            .dispatch_toolbar_toggle(id)
            .map_err(|e| e.to_string())
    }
    pub fn set_toolbar_entry_value(&self, id: &str, v: &str) -> Result<(), String> {
        self.inner
            .set_toolbar_entry_value(id, v)
            .map_err(|e| e.to_string())
    }
    pub fn finish_workflow(&self) -> Result<(), String> {
        self.inner.finish_workflow().map_err(|e| e.to_string())
    }
    pub fn cancel_workflow(&self) -> Result<(), String> {
        self.inner.cancel_workflow().map_err(|e| e.to_string())
    }
    pub fn suspend_workflow(&self) -> Result<bool, String> {
        self.inner.suspend_workflow()
    }
    pub fn resume_workflow(&self) -> Result<field_scripting::ResumeWorkflow, String> {
        self.inner.resume_workflow()
    }
    pub fn clear_session_workflow(&self) {
        self.inner.clear_session_workflow()
    }
    pub fn toolbar_snapshot(&self) -> Option<(String, Vec<field_scripting::ToolbarItem>)> {
        self.inner.toolbar_snapshot()
    }
    pub fn active_workflow_name(&self) -> Option<String> {
        self.inner.active_workflow_name()
    }
    pub fn fire_loaded(&self, id: DocumentId, e: f64) {
        self.inner.fire_loaded(id, e)
    }
    pub fn fire_saved(&self, id: DocumentId, e: f64) {
        self.inner.fire_saved(id, e)
    }
    pub fn fire_session_loaded(&self) {
        self.inner.fire_session_loaded()
    }
    pub fn fire_session_saved(&self) {
        self.inner.fire_session_saved()
    }
    pub fn fire_session_selected(&self) {
        self.inner.fire_session_selected()
    }
    pub fn fire_composition_selected(&self, id: Option<DocumentId>) {
        self.inner.fire_composition_selected(id)
    }
    pub fn fire_detect_layout(&self, id: DocumentId) {
        self.inner.fire_detect_layout(id)
    }
    pub fn layout_names(&self) -> Vec<String> {
        self.inner.layout_names()
    }
    pub fn layout_choices(&self) -> Vec<(String, String)> {
        self.inner.layout_choices()
    }
    pub fn layout(&self, n: &str) -> Option<field_scripting::ChannelLayoutDef> {
        self.inner.layout(n)
    }
    pub fn choose_layout(&self, id: DocumentId, n: Option<&str>) -> mlua::Result<()> {
        self.inner.choose_layout(id, n)
    }
    pub fn take_prints(&self) -> Vec<String> {
        self.inner.take_prints()
    }
    pub fn take_logs(&self) -> Vec<field_scripting::LogEntry> {
        self.inner.take_logs()
    }
    pub fn take_alerts(&self) -> Vec<(String, String)> {
        self.inner.take_alerts()
    }
    pub fn workflow_metas(&self) -> Vec<field_scripting::WorkflowMeta> {
        self.inner.workflow_metas()
    }
    pub fn drop_layout(&self) -> field_scripting::DropLayout {
        self.inner.drop_layout()
    }
    pub fn log(&self, l: field_scripting::LogLevel, t: String, m: String) {
        self.inner.log(l, t, m)
    }
}
