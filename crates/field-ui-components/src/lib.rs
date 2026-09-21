// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

#![deny(missing_docs)]

//! # field-ui-components
//!
//! Reusable GPUI chrome, waveform display, and host data traits.
//! Application-specific docks (explorer, render sheet, etc.) stay in the host
//! app. Dock tab titles are owned by the host; pass them to
//! [`tool_dock_min_size`] when measuring side docks.
//!
//! Host apps import this crate directly (for example
//! `use field_ui_components::AppMenuBar`); do not re-export types through the
//! application crate.
//!
//! ```no_run
//! use field_ui_components::{LaneScope, PaintRegion, WaveformDataProvider, WaveformEditor};
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
//! impl WaveformEditor for EmptyWave {
//!     fn selection_span(&self) -> Option<(usize, usize)> { None }
//!     fn playhead(&self) -> Option<(usize, LaneScope)> { None }
//!     fn channel_lanes(&self, _lane: usize, _alt: bool) -> LaneScope { LaneScope::All }
//!     fn begin_replace(&mut self, _: usize, _: LaneScope, _: usize) {}
//!     fn begin_extend(&mut self, _: usize, _: LaneScope, _: usize) {}
//!     fn begin_disjoint(&mut self, _: usize, _: LaneScope, _: usize) {}
//!     fn update_drag(&mut self, _: usize, _: usize) {}
//!     fn finish_drag(&mut self) {}
//!     fn click_without_drag(&mut self, _: usize, _: LaneScope, _: bool) {}
//!     fn selection_regions(&self) -> Vec<PaintRegion> { Vec::new() }
//!     fn named_regions(&self) -> Vec<PaintRegion> { Vec::new() }
//!     fn markers_for_paint(&self) -> Vec<(u64, [f32; 4])> { Vec::new() }
//!     fn modified_ranges(&self) -> Vec<(u64, u64)> { Vec::new() }
//!     fn ranges_for_edit(&self, _: u64) -> Vec<(u64, u64)> { Vec::new() }
//!     fn selection_position_sample(&self) -> Option<usize> { None }
//! }
//! ```

mod app_menu;
mod dock_skin;
mod edits;
mod markers;
mod messages;
mod monitor;
mod param_ui;
mod regions;
mod repl;
mod status_bar;
mod theme;
mod transport;
mod waveform;
mod waveform_axis;
mod waveform_data;
mod waveform_editor;

pub use app_menu::AppMenuBar;
pub use dock_skin::{tool_dock_min_size, CenterTabBarHandler, CompactDockSkin};
pub use edits::{
    EditActivateHandler, EditCard, EditClickHandler, EditHoverHandler, EditsData, EditsPanel,
};
pub use markers::{
    DeleteSelectedMarker, MarkerDeleteHandler, MarkerRow, MarkerSelectHandler, MarkersData,
    MarkersPanel,
};
pub use messages::{LogLevel, LogLine, MessagesPanel};
pub use monitor::{MonitorCallbacks, MonitorPanel, MonitorSnapshotProvider};
pub use param_ui::{ChainChoice, MonitorSnapshot, ParamUiNode};
pub use regions::{
    RegionDeleteHandler, RegionGroup, RegionRow, RegionSelectHandler, RegionsData, RegionsPanel,
};
pub use repl::{ReplEvalHandler, ReplOutput, ReplPanel};
pub use status_bar::{FileStatus, LayoutPicker, SessionStatusBar};
pub use theme::{content_foreground, ContentForeground};
pub use transport::{Transport, TransportAction};
pub use waveform::WaveformDisplay;
pub use waveform_axis::{format_db_hover, format_hz_hover, hover_axis_quantize, WaveformHoverAxis};
pub use waveform_data::{
    clamp_peaks_spectrum_split, WaveformDataProvider, WaveformRepresentation,
    DEFAULT_PEAKS_SPECTRUM_SPLIT, MAX_PEAKS_SPECTRUM_SPLIT, MIN_PEAKS_SPECTRUM_SPLIT,
};
pub use waveform_editor::{LaneScope, PaintRegion, PeakStatus, WaveformEditor};
