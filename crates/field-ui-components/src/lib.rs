// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

#![deny(missing_docs)]

//! # field-ui-components
//!
//! Reusable GPUI chrome and the gpui-free [`WaveformDataProvider`] trait.
//! Application-specific docks (explorer, monitor, render sheet, etc.) stay in
//! the host app. Transport and status-bar widgets remain there for now because
//! they still bind to app command and model types.
//!
//! ```no_run
//! use field_ui_components::WaveformDataProvider;
//!
//! struct EmptyWave;
//! impl WaveformDataProvider for EmptyWave {
//!     fn sample_rate(&self) -> u32 { 48_000 }
//!     fn channel_count(&self) -> usize { 0 }
//!     fn frames(&self) -> usize { 0 }
//!     fn duration_secs(&self) -> f64 { 0.0 }
//!     fn channel_label(&self, _channel: usize) -> String { String::new() }
//!     fn read_channel(&self, _channel: usize, _start: usize, _dest: &mut [f32]) {}
//!     fn min_max_in_range(&self, _channel: usize, _start: f64, _end: f64) -> (f32, f32) {
//!         (0.0, 0.0)
//!     }
//! }
//! ```

mod app_menu;
mod dock_skin;
mod waveform_data;

pub use app_menu::AppMenuBar;
pub use dock_skin::{
    detail_dock_min_size, explorer_dock_min_size, CenterTabBarHandler, CompactDockSkin,
    DETAIL_TAB_HISTORY, DETAIL_TAB_MARKER, DETAIL_TAB_MONITOR, DETAIL_TAB_REGIONS,
    EXPLORER_TAB_COMPOSITIONS,
};
pub use waveform_data::WaveformDataProvider;
