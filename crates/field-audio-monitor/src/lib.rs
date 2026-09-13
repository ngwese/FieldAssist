// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

#![deny(missing_docs)]

//! # field-audio-monitor
//!
//! Faust-backed monitor DSP chains for FieldAssist: mono, stereo, M/S, and
//! B-format (AmbiX / FuMa) decode to stereo headphones/speakers.
//!
//! Live parameters use a lock-free [`ParamStore`] (`AtomicU32` bit-casts of
//! `f32`). The UI writes controls without taking the DSP graph mutex; the
//! audio thread snapshots at block start, applies values to Faust, computes,
//! then publishes meters.
//!
//! ```
//! use field_audio_monitor::{MonitorChain, MonitorHost};
//!
//! let host = MonitorHost::new(48_000);
//! host.set_config(Some(MonitorChain::Stereo), None, 48_000);
//! host.set_param("/MonitorStereo/Output_Gain", -6.0);
//! assert!(host.get_param("/MonitorStereo/Output_Gain").is_some());
//! ```
//!
//! This crate does **not** depend on `field-audio-playback`. Wire
//! [`MonitorHost`] into a playback engine via an adapter in the application.

mod chain;
mod dsp;
#[allow(
    missing_docs,
    dead_code,
    non_snake_case,
    unused_parens,
    unused_variables,
    unused_mut,
    clippy::all
)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/monitor_dsp.rs"));
}
mod host;
mod params;
mod schema;

pub use chain::MonitorChain;
#[allow(unused_imports)]
pub use dsp::{create_dsp, IdentityDsp, MonitorDsp};
pub use host::{map_direct, MonitorHost};
pub use params::ParamStore;
pub use schema::{menu_items_from_meta, meta_value, parse_ui_json, FaustUiNode, FaustUiRoot};
