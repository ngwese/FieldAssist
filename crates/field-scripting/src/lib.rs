// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

#![deny(missing_docs)]

//! # field-scripting
//!
//! Shared Lua 5.4 host for FieldAssist, field-batch, and related tools.
//! See the [crate README](../README.md) for backend integration guidance.
//!
//! The global `app` object is a thin host facade (`app.name` identifies the
//! process). Common APIs live under the `field` namespace
//! (`field.session`, `field.fs`, `field.workflow`, …).
//!
//! ```no_run
//! use field_scripting::{HostProfile, ScriptHost};
//!
//! let mut host = ScriptHost::new(HostProfile {
//!     name: "field-batch",
//!     config_dir: None,
//! })?;
//! let out = host.eval("return app.name");
//! assert_eq!(out.result.as_deref(), Some("field-batch"));
//! # Ok::<(), mlua::Error>(())
//! ```

mod app;
mod audio_devices;
mod backend;
mod bindings;
mod composition;
mod config_dir;
mod export;
mod field_ns;
mod fs;
mod host;
mod include;
mod keep_lua_c_api;
mod layout;
mod log;
mod marker;
mod media;
mod package_policy;
mod prototype;
mod region;
mod script_search;
mod selection;
mod session;
mod ui;
mod url;
mod util;
mod variables;
mod workflow;
mod workflow_app;
mod workflow_sheet;
mod workflow_toolbar;
mod world;

pub use audio_devices::bind_audio_devices;
pub use backend::{BackendHandle, HeadlessBackend, ScriptBackend};
pub use bindings::{
    LuaBindings, live_user_bindings, split_readonly_by_scope, split_source_by_scope,
    split_user_detached, user_scope_paths,
};
pub use config_dir::user_config_dir;
pub use export::{
    ExportChannels, ExportProfileDef, ExportSourceDefaults, ResolvedExportSettings,
    resolve_export_settings,
};
pub use fs::bind_fs;
pub use host::{
    EMBEDDED_INIT, EMBEDDED_RESOLVER_DEFAULT, EvalOutput, HostHandle, HostProfile, LogEntry,
    LogLevel, ResumeWorkflow, ScriptHost, host_from_lua,
};
pub use layout::ChannelLayoutDef;
pub use package_policy::{PackagePolicy, install_package_policy};
pub use script_search::{
    RESOLVER_PREFIX, WORKFLOW_PREFIX, collect_matching_lua, matching_lua_files, script_search_dirs,
};
pub use url::bind_url;
pub use variables::{
    LuaVariableResolver, ResolverDef, composition_layer_variables,
    composition_layer_variables_for_host, composition_layer_with_channel_layout_token,
    composition_site_bindings, composition_source_variables, export_site_bindings,
    resolve_composition_variable, resolve_with_active, session_site_bindings,
};
pub use workflow::{WorkflowDef, WorkflowMeta};
pub use workflow_app::{
    DropLayout, SCOPE_DRAG_DROP, SCOPE_MENU, layout_drop_targets, workflows_for_menu,
};
pub use workflow_sheet::{SheetPane, SheetSnapshot};
pub use workflow_toolbar::{PathBrowse, ProgressVariant, ToolbarAlign, ToolbarItem};
pub use world::{HeadlessWorld, OpenDocument};

pub use composition::LuaComposition;
pub use session::LuaSession;

/// Install the shared `field.*` namespace (used by field-batch and FieldAssist).
pub use field_ns::bind_field;
