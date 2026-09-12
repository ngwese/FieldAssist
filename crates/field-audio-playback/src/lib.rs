// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

#![deny(missing_docs)]

//! # field-audio-playback
//!
//! Device I/O, transport, and a playback engine that pulls interleaved frames
//! from a [`PlaybackDataProvider`] and optionally runs a [`MonitorProcess`].
//!
//! This crate does **not** depend on `field-audio-monitor` or composition.
//! Applications supply a `MonitorProcess` adapter (for example wrapping
//! `field_audio_monitor::MonitorHost`) and providers for their buffers.
//!
//! ```
//! use field_audio_playback::{PlaybackDataProvider, TransportState};
//!
//! struct Silence;
//! impl PlaybackDataProvider for Silence {
//!     fn sample_rate(&self) -> u32 { 48_000 }
//!     fn channel_count(&self) -> usize { 1 }
//!     fn frames(&self) -> usize { 1 }
//!     fn read_interleaved(&self, _start: usize, _count: usize, dest: &mut [f32]) {
//!         dest.fill(0.0);
//!     }
//! }
//!
//! assert_eq!(TransportState::Stopped.to_u8(), 0);
//! let _ = Silence;
//! ```

mod device;
mod engine;
mod monitor;
mod playhead;
mod provider;
mod transport;

pub use device::{
    list_output_devices, output_device_name, print_output_devices, resolve_output_device,
    OutputDeviceInfo,
};
pub use engine::{PlaybackEngine, PlaybackShared, PLAYBACK_READ_FRAMES};
pub use monitor::{map_direct, MonitorProcess};
pub use playhead::{Playhead, PlayheadEvent};
pub use provider::PlaybackDataProvider;
pub use transport::{Transport, TransportState};
