// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Adapter from [`MonitorHost`] to [`field_audio_playback::MonitorProcess`].

use std::collections::HashMap;
use std::sync::Arc;

use field_audio_monitor::MonitorHost;
use field_audio_playback::MonitorProcess;

/// Wraps [`MonitorHost`] so the playback engine stays free of Faust.
pub struct MonitorHostProcess {
    host: Arc<MonitorHost>,
}

impl MonitorHostProcess {
    /// Create an adapter around a shared host.
    pub fn new(host: Arc<MonitorHost>) -> Self {
        Self { host }
    }
}

impl MonitorProcess for MonitorHostProcess {
    fn process_gathered(
        &self,
        gathered: &[f32],
        src_ch: usize,
        frames: usize,
        output: &mut [f32],
        out_ch: usize,
    ) {
        self.host
            .process_gathered(gathered, src_ch, frames, output, out_ch);
    }

    fn set_param(&self, address: &str, value: f32) {
        self.host.set_param(address, value);
    }

    fn get_param(&self, address: &str) -> Option<f32> {
        self.host.get_param(address)
    }

    fn meter(&self, address: &str) -> Option<f32> {
        self.host.meter(address)
    }

    fn meters(&self) -> HashMap<String, f32> {
        self.host.meters()
    }

    fn input_meters_above(&self, quiet_db: f32) -> bool {
        self.host.input_meters_above(quiet_db)
    }

    fn ui_json(&self) -> Option<&'static str> {
        self.host.ui_json()
    }

    fn set_output_sample_rate(&self, sample_rate: u32) {
        let chain = self.host.chain();
        let channels = self.host.playback_channels();
        self.host.set_config(chain, channels, sample_rate);
    }
}
