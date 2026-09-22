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
mod composition;
mod config_dir;
mod field_ns;
mod fs;
mod host;
mod include;
mod layout;
mod log;
mod marker;
mod media;
mod package_policy;
mod prototype;
mod region;
mod selection;
mod session;
mod ui;
mod url;
mod util;
mod workflow;
mod workflow_app;
mod workflow_toolbar;
mod world;

pub use audio_devices::bind_audio_devices;
pub use backend::{BackendHandle, HeadlessBackend, ScriptBackend};
pub use config_dir::user_config_dir;
pub use fs::bind_fs;
pub use host::{
    host_from_lua, EvalOutput, HostHandle, HostProfile, LogEntry, LogLevel, ResumeWorkflow,
    ScriptHost,
};
pub use layout::ChannelLayoutDef;
pub use package_policy::{install_package_policy, PackagePolicy};
pub use url::bind_url;
pub use workflow::{WorkflowDef, WorkflowMeta};
pub use workflow_app::{
    layout_drop_targets, workflows_for_menu, DropLayout, SCOPE_DRAG_DROP, SCOPE_MENU,
};
pub use workflow_toolbar::{PathBrowse, ToolbarAlign, ToolbarItem};
pub use world::{HeadlessWorld, OpenDocument};

/// Install the shared `field.*` namespace (used by field-batch and FieldAssist).
pub use field_ns::bind_field;
