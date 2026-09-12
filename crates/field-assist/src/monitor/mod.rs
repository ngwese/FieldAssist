// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Monitor DSP re-exports and playback adapter.

#[allow(unused_imports)] // compatibility re-exports for app call sites
pub use field_audio_monitor::{
    menu_items_from_meta, meta_value, parse_ui_json, FaustUiNode, FaustUiRoot, MonitorChain,
    MonitorHost,
};

mod process;

pub use process::MonitorHostProcess;
