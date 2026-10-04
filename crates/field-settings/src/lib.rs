// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

#![deny(missing_docs)]

//! Shared `settings.json` schema and load/save for FieldAssist hosts.
//!
//! Hosts choose which groups to apply. GPUI Settings UI stays in FieldAssist.

mod groups;
mod io;

pub use groups::AppSettings;
pub use groups::{
    AppearanceSettings, AudioSettings, ExperimentalSettings, ScriptingSettings, SelectionSettings,
    SpectrumGradientStopSettings, ViewSettings, WaveformSettings, DEFAULT_RIBBON_DB,
    DEFAULT_RIBBON_SHELL_VALUE_REDUCE, DETAIL_DOCK_TRUE_TAB, DOCK_HIDDEN, SCRIPT_DOCK_TRUE_TAB,
    SETTINGS_FORMAT_VERSION, SETTINGS_KIND, SPECTRUM_GRADIENT_DB_FLOOR,
};
pub use io::{load_from_dir, path_in, save_to_dir};
