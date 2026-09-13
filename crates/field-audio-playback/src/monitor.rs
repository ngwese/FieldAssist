// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Monitor processing trait used by the playback engine.

use std::collections::HashMap;

/// Audio-thread monitor insert used by [`crate::PlaybackShared`].
///
/// [`Self::process_gathered`] runs on the **CPAL callback** and must meet the
/// realtime quality gates (no heap allocation, no blocking locks, no I/O).
/// Implementations should keep [`Self::set_param`] / [`Self::get_param`] /
/// [`Self::meter`] lock-free. Graph swaps may allocate off the callback and
/// publish a new process slot via `ArcSwap` (or equivalent).
///
/// Defined here so `field-audio-playback` does not depend on Faust or
/// `field-audio-monitor`. The application crate adapts `MonitorHost`.
pub trait MonitorProcess: Send + Sync {
    /// Process interleaved source frames into device interleaved output.
    ///
    /// Called from the realtime callback; must not allocate or block.
    fn process_gathered(
        &self,
        gathered: &[f32],
        src_ch: usize,
        frames: usize,
        output: &mut [f32],
        out_ch: usize,
    );

    /// Set a live control parameter (prefer lock-free).
    fn set_param(&self, address: &str, value: f32);

    /// Read a live control parameter.
    fn get_param(&self, address: &str) -> Option<f32>;

    /// Read a meter value.
    fn meter(&self, address: &str) -> Option<f32>;

    /// Snapshot all meters (may allocate; UI thread only).
    fn meters(&self) -> HashMap<String, f32>;

    /// Whether any input meter is above `quiet_db` (realtime-safe).
    fn input_meters_above(&self, quiet_db: f32) -> bool {
        let _ = quiet_db;
        false
    }

    /// Faust (or other) UI JSON for the active chain, if any.
    fn ui_json(&self) -> Option<&'static str>;

    /// Update sample rate used by the DSP graph (may recreate DSP).
    fn set_output_sample_rate(&self, sample_rate: u32);

    /// Output channel count the DSP is targeting (stereo monitor → 2).
    fn output_channels(&self) -> usize {
        2
    }
}

/// Direct channel mapping when no monitor process is installed.
pub fn map_direct(
    gathered: &[f32],
    src_ch: usize,
    frames: usize,
    output: &mut [f32],
    out_ch: usize,
) {
    for frame in 0..frames {
        let base = frame * src_ch;
        for ch in 0..out_ch {
            let sample = if src_ch == 1 {
                gathered[base]
            } else if ch < src_ch {
                gathered[base + ch]
            } else {
                0.0
            };
            output[frame * out_ch + ch] = sample;
        }
    }
}
