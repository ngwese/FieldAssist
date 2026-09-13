// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Playback engine re-exports plus app-specific session and providers.

pub mod anchors;
pub mod provider;
pub mod session;

#[cfg(test)]
mod monitor_tests;

#[allow(unused_imports)] // compatibility re-exports for app call sites
pub use field_audio_playback::{
    list_output_devices, output_device_name, print_output_devices, resolve_output_device,
    MonitorProcess, PlaybackDataProvider, PlaybackEngine, Playhead, Transport, TransportState,
};
pub use session::PlaybackSession;
