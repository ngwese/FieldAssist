// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! Integration tests for MonitorHost wired as MonitorProcess.

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use field_audio_playback::{PlaybackDataProvider, PlaybackShared, TransportState};

    use crate::monitor::{MonitorChain, MonitorHost, MonitorHostProcess};
    use crate::playback::MonitorProcess;

    struct PlanarAudio {
        sample_rate: u32,
        channels: Vec<Vec<f32>>,
    }

    impl PlaybackDataProvider for PlanarAudio {
        fn sample_rate(&self) -> u32 {
            self.sample_rate
        }
        fn channel_count(&self) -> usize {
            self.channels.len()
        }
        fn frames(&self) -> usize {
            self.channels.first().map(|c| c.len()).unwrap_or(0)
        }
        fn read_interleaved(&self, start: usize, count: usize, dest: &mut [f32]) {
            let ch_count = self.channels.len();
            if ch_count == 0 {
                dest.fill(0.0);
                return;
            }
            let total = count * ch_count;
            dest[..total].fill(0.0);
            for (ch, samples) in self.channels.iter().enumerate() {
                let end = (start + count).min(samples.len());
                if start >= end {
                    continue;
                }
                for (frame, &sample) in samples[start..end].iter().enumerate() {
                    dest[frame * ch_count + ch] = sample;
                }
            }
        }
    }

    fn with_monitor(
        channels: Vec<Vec<f32>>,
        chain: MonitorChain,
        route: Option<Vec<usize>>,
    ) -> PlaybackShared {
        let shared = PlaybackShared::with_output_layout(
            Arc::new(PlanarAudio {
                sample_rate: 44100,
                channels,
            }),
            44100,
            2,
        );
        let host = Arc::new(MonitorHost::new(44100));
        host.set_config(Some(chain), route, 44100);
        shared.set_monitor_process(Some(
            Arc::new(MonitorHostProcess::new(host)) as Arc<dyn MonitorProcess>
        ));
        shared
    }

    #[test]
    fn monitor_subset_routes_last_two_channels() {
        let mut channels = vec![vec![0.0f32; 40]; 6];
        for i in 0..40 {
            channels[4][i] = 0.4;
            channels[5][i] = -0.6;
        }
        let shared = with_monitor(channels, MonitorChain::Stereo, Some(vec![4, 5]));
        shared.set_transport(TransportState::Playing);
        let mut out = vec![0.0; 20];
        shared.fill_output(&mut out);
        assert!((out[0] - 0.4).abs() < 0.05, "{}", out[0]);
        assert!((out[1] + 0.6).abs() < 0.05, "{}", out[1]);
        assert!((out[2] - 0.4).abs() < 0.05);
        assert!((out[3] + 0.6).abs() < 0.05);
    }

    #[test]
    fn six_channel_file_routes_foa_or_stereo_subset() {
        let mut channels = vec![vec![0.0f32; 40]; 6];
        for i in 0..40 {
            channels[0][i] = 1.0;
            channels[4][i] = 0.4;
            channels[5][i] = -0.6;
        }
        let shared = with_monitor(
            channels.clone(),
            MonitorChain::Foa,
            Some(vec![0, 1, 2, 3]),
        );
        shared.set_transport(TransportState::Playing);
        let mut out = vec![0.0; 20];
        shared.fill_output(&mut out);
        assert!((out[0] - 0.7071).abs() < 0.08, "{}", out[0]);
        assert!((out[1] - out[0]).abs() < 0.03);

        let shared = with_monitor(channels, MonitorChain::Stereo, Some(vec![4, 5]));
        shared.set_transport(TransportState::Playing);
        let mut out = vec![0.0; 20];
        shared.fill_output(&mut out);
        assert!((out[0] - 0.4).abs() < 0.05, "{}", out[0]);
        assert!((out[1] + 0.6).abs() < 0.05, "{}", out[1]);
    }

    #[test]
    fn six_channel_file_routes_foa_fuma_subset() {
        let mut channels = vec![vec![0.0f32; 40]; 6];
        for i in 0..40 {
            channels[0][i] = 1.0 / 2.0f32.sqrt();
        }
        let shared = with_monitor(channels, MonitorChain::FoaFuma, Some(vec![0, 1, 2, 3]));
        shared.set_transport(TransportState::Playing);
        let mut out = vec![0.0; 20];
        shared.fill_output(&mut out);
        assert!((out[0] - 0.7071).abs() < 0.08, "{}", out[0]);
        assert!((out[1] - out[0]).abs() < 0.03);
    }
}
