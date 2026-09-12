// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! In-memory planar PCM buffer used by the editor model.

/// Decoded, planar audio ready for editing and display.
#[derive(Debug)]
pub struct PcmBuffer {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Planar channel data (`channels[ch][frame]`).
    pub channels: Vec<Vec<f32>>,
    /// Overview peak bins per channel as `(min, max)` pairs.
    pub peaks: Vec<Vec<(f32, f32)>>,
}

impl PcmBuffer {
    /// Empty buffer at `sample_rate` with no channels.
    pub fn empty(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            channels: vec![],
            peaks: vec![],
        }
    }

    /// Number of planar channels.
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// Frame count of the first channel (0 if empty).
    pub fn frames(&self) -> usize {
        self.channels.first().map(|c| c.len()).unwrap_or(0)
    }

    /// Duration in seconds, or `0.0` when the rate is zero.
    pub fn duration_secs(&self) -> f64 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.frames() as f64 / f64::from(self.sample_rate)
        }
    }
}

/// Historical name for [`PcmBuffer`].
pub type DecodedAudio = PcmBuffer;
