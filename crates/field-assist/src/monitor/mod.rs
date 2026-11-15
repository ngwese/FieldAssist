// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Monitor DSP re-exports and playback adapter.

#[allow(unused_imports)] // compatibility re-exports for app call sites
pub use field_audio_monitor::{
    FaustUiNode, FaustUiRoot, MonitorChain, MonitorHost, menu_items_from_meta, meta_value,
    parse_ui_json,
};

mod offline;
mod process;

#[allow(unused_imports)] // used by scripting surface (task 3.x)
pub use offline::FaustOfflineDsp;
pub use process::MonitorHostProcess;
