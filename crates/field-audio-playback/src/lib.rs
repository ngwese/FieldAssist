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
//! ## Realtime path
//!
//! [`PlaybackEngine`] starts a dedicated **prefetch** thread that may allocate,
//! lock the composition pager, decode media, and run **bandlimited sample-rate
//! conversion** when the source rate differs from the device rate. Matched-rate
//! Direct playback copies frames bit-exactly. The CPAL callback only drains a
//! lock-free [`PrefetchRing`] and applies Direct channel mapping or a
//! [`MonitorProcess`] — see the crate `AGENTS.md` for the quality gates (zero
//! heap allocation, zero blocking lock contention on the callback). Hosts drain
//! underruns and CPAL stream errors with [`PlaybackShared::take_faults`] (and
//! optional [`PlaybackFaultFlusher`] coalescing) on a non-realtime thread.
//!
//! Direct / no-monitor means Faust is bypassed; rate conversion still runs on
//! the prefetch thread whenever source and device rates differ.
//!
//! `dasp::ring_buffer` is intentionally **not** used for this boundary: its
//! `Fixed`/`Bounded` types require `&mut self` for push/pop and cannot be shared
//! across threads without a mutex.
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
mod faults;
mod monitor;
mod playhead;
mod prefetch;
mod provider;
mod src_convert;
mod transport;

pub use device::{
    list_output_devices, output_device_name, print_output_devices, resolve_output_device,
    OutputDeviceInfo,
};
pub use engine::{PlaybackEngine, PlaybackShared, PlaybackStats, PLAYBACK_READ_FRAMES};
pub use faults::{
    PlaybackFaultFlusher, PlaybackFaultLevel, PlaybackFaultMessage, PlaybackFaults,
    PLAYBACK_FAULT_LOG_INTERVAL,
};
pub use monitor::{map_direct, MonitorProcess};
pub use playhead::{Playhead, PlayheadEvent};
pub use prefetch::{PrefetchRing, PREFETCH_CAPACITY_FRAMES, PREFETCH_CHUNK_FRAMES};
pub use provider::PlaybackDataProvider;
pub use transport::{Transport, TransportState};
