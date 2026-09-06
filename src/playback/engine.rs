// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};

use crate::monitor::{MonitorChain, MonitorHost};

use super::provider::PlaybackDataProvider;
use super::transport::TransportState;

const IN_OUT_NONE: usize = usize::MAX;

/// Source frames fetched per provider call while filling the output device.
pub const PLAYBACK_READ_FRAMES: usize = 128;

pub struct PlaybackShared {
    pub provider: Arc<dyn PlaybackDataProvider>,
    pub position: AtomicUsize,
    pub transport: AtomicU8,
    pub looping: std::sync::atomic::AtomicBool,
    pub in_point: AtomicUsize,
    pub out_point: AtomicUsize,
    epoch: AtomicUsize,
    output_rate: u32,
    output_channels: usize,
    monitor: Mutex<MonitorHost>,
}

impl PlaybackShared {
    pub fn new(provider: Arc<dyn PlaybackDataProvider>, output_rate: u32) -> Self {
        let output_channels = provider.channel_count().max(1);
        Self::with_output_layout(provider, output_rate, output_channels)
    }

    pub fn with_output_layout(
        provider: Arc<dyn PlaybackDataProvider>,
        output_rate: u32,
        output_channels: usize,
    ) -> Self {
        Self {
            output_rate,
            output_channels: output_channels.max(1),
            provider,
            position: AtomicUsize::new(0),
            transport: AtomicU8::new(TransportState::Stopped.to_u8()),
            looping: std::sync::atomic::AtomicBool::new(false),
            in_point: AtomicUsize::new(IN_OUT_NONE),
            out_point: AtomicUsize::new(IN_OUT_NONE),
            epoch: AtomicUsize::new(0),
            monitor: Mutex::new(MonitorHost::new(output_rate)),
        }
    }

    pub fn bump_epoch(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }

    pub fn set_transport(&self, state: TransportState) {
        self.transport.store(state.to_u8(), Ordering::SeqCst);
    }

    pub fn transport(&self) -> TransportState {
        TransportState::from_u8(self.transport.load(Ordering::SeqCst))
    }

    pub fn set_position(&self, sample: usize) {
        self.position.store(sample, Ordering::SeqCst);
    }

    pub fn position(&self) -> usize {
        self.position.load(Ordering::SeqCst)
    }

    pub fn set_looping(&self, looping: bool) {
        self.looping.store(looping, Ordering::SeqCst);
    }

    pub fn set_in_out(&self, in_point: Option<usize>, out_point: Option<usize>) {
        self.in_point
            .store(in_point.unwrap_or(IN_OUT_NONE), Ordering::SeqCst);
        self.out_point
            .store(out_point.unwrap_or(IN_OUT_NONE), Ordering::SeqCst);
    }

    pub fn output_rate(&self) -> u32 {
        self.output_rate
    }

    pub fn set_monitor(&self, chain: Option<MonitorChain>, playback_channels: Option<Vec<usize>>) {
        self.monitor
            .lock()
            .unwrap()
            .set_config(chain, playback_channels, self.output_rate);
    }

    pub fn set_monitor_param(&self, address: &str, value: f32) {
        self.monitor.lock().unwrap().set_param(address, value);
    }

    pub fn flush_monitor_working(
        &self,
    ) -> std::collections::HashMap<MonitorChain, std::collections::HashMap<String, f32>> {
        self.monitor.lock().unwrap().flush_working()
    }

    pub fn monitor_session_params(
        &self,
    ) -> std::collections::HashMap<MonitorChain, std::collections::HashMap<String, f32>> {
        self.monitor.lock().unwrap().session_params()
    }

    pub fn merge_monitor_into_session(
        &self,
        params: &std::collections::HashMap<MonitorChain, std::collections::HashMap<String, f32>>,
    ) {
        self.monitor.lock().unwrap().merge_into_session(params);
    }

    pub fn replace_monitor_working(
        &self,
        chain: Option<MonitorChain>,
        playback_channels: Option<Vec<usize>>,
        working: std::collections::HashMap<MonitorChain, std::collections::HashMap<String, f32>>,
    ) {
        self.monitor.lock().unwrap().replace_working(
            chain,
            playback_channels,
            self.output_rate,
            working,
        );
    }

    pub fn monitor_param(&self, address: &str) -> Option<f32> {
        self.monitor.lock().unwrap().get_param(address)
    }

    pub fn monitor_meter(&self, address: &str) -> Option<f32> {
        self.monitor.lock().unwrap().meter(address)
    }

    pub fn monitor_ui_json(&self) -> Option<&'static str> {
        self.monitor.lock().unwrap().ui_json()
    }

    pub fn monitor_meters(&self) -> std::collections::HashMap<String, f32> {
        self.monitor.lock().unwrap().meters().clone()
    }

    fn playback_start(&self) -> usize {
        let v = self.in_point.load(Ordering::SeqCst);
        if v == IN_OUT_NONE {
            0
        } else {
            v
        }
    }

    fn playback_end(&self) -> usize {
        let looping = self.looping.load(Ordering::SeqCst);
        if looping {
            let v = self.out_point.load(Ordering::SeqCst);
            if v == IN_OUT_NONE {
                self.provider.frames().saturating_sub(1)
            } else {
                v
            }
        } else {
            self.provider.frames().saturating_sub(1)
        }
    }

    pub(crate) fn fill_output(&self, output: &mut [f32]) {
        output.fill(0.0);
        if self.transport() != TransportState::Playing {
            return;
        }

        let src_ch = self.provider.channel_count();
        if src_ch == 0 {
            return;
        }
        let out_ch = self.output_channels.max(1);
        let out_frames = output.len() / out_ch;
        if out_frames == 0 {
            return;
        }

        let end = self.playback_end();
        let start_bound = self.playback_start();
        let looping = self.looping.load(Ordering::SeqCst);
        let src_rate = self.provider.sample_rate().max(1);
        let step = src_rate as f64 / f64::from(self.output_rate.max(1));
        let epoch = self.epoch.load(Ordering::SeqCst);
        let origin = self.position.load(Ordering::SeqCst);

        // A new Play issued while parked on the last sample should start over.
        // Natural end-of-buffer always sets Stopped, so this only runs on a
        // fresh Playing transition from the end.
        let mut pos_f = if !looping && origin >= end {
            start_bound as f64
        } else {
            origin as f64
        };

        let mut monitor = self.monitor.lock().unwrap();
        let n = PLAYBACK_READ_FRAMES * src_ch;
        let _ = monitor.read_buf_mut(n.max(1));
        let _ = monitor.gathered_mut((out_frames * src_ch).max(1));

        let mut buf_origin = 0usize;
        let mut buf_frames = 0usize;
        let mut reached_end = false;
        let mut produced = 0usize;

        for _out_frame in 0..out_frames {
            let source_pos = pos_f as usize;
            if source_pos > end {
                if looping && end > start_bound {
                    pos_f = start_bound as f64;
                    buf_frames = 0;
                    continue;
                }
                reached_end = true;
                break;
            }

            if buf_frames == 0 || source_pos < buf_origin || source_pos - buf_origin >= buf_frames {
                let remaining = end.saturating_add(1).saturating_sub(source_pos);
                let take = remaining.min(PLAYBACK_READ_FRAMES).max(1);
                let n = take * src_ch;
                let read_buf = monitor.read_buf_mut(n);
                self.provider.read_interleaved(source_pos, take, read_buf);
                buf_origin = source_pos;
                buf_frames = take;
            }
            let local = source_pos - buf_origin;
            let base = local * src_ch;
            let dest_base = produced * src_ch;
            monitor.copy_read_frame(base, src_ch, dest_base);
            produced += 1;
            pos_f += step;
        }

        if produced > 0 {
            monitor.process_produced(src_ch, produced, output, out_ch);
        }

        if !looping && pos_f > end as f64 {
            reached_end = true;
        }

        if self.epoch.load(Ordering::SeqCst) != epoch {
            return;
        }

        if reached_end {
            self.set_transport(TransportState::Stopped);
            self.position.store(end, Ordering::SeqCst);
            return;
        }

        let final_pos = pos_f.min(end as f64) as usize;
        self.position.store(final_pos, Ordering::SeqCst);
    }
}

pub struct PlaybackEngine {
    _stream: Stream,
    pub shared: Arc<PlaybackShared>,
}

impl PlaybackEngine {
    pub fn open(device: &Device, provider: Arc<dyn PlaybackDataProvider>) -> Result<Self> {
        let default_config = device
            .default_output_config()
            .context("failed to get default output config")?;

        let sample_format = default_config.sample_format();
        let stream_config = StreamConfig {
            channels: default_config.channels(),
            sample_rate: default_config.sample_rate(),
            buffer_size: cpal::BufferSize::Default,
        };
        let output_rate = stream_config.sample_rate;
        let output_channels = stream_config.channels as usize;
        let shared = Arc::new(PlaybackShared::with_output_layout(
            provider,
            output_rate,
            output_channels,
        ));
        let shared_cb = shared.clone();

        let stream = build_output_stream(device, stream_config, sample_format, shared_cb.clone())
            .or_else(|_| {
                let fallback = StreamConfig {
                    channels: default_config.channels(),
                    sample_rate: default_config.sample_rate(),
                    buffer_size: cpal::BufferSize::Default,
                };
                build_output_stream(device, fallback, sample_format, shared_cb)
            })
            .context("failed to build output stream")?;

        stream.play().context("failed to start output stream")?;

        Ok(Self {
            _stream: stream,
            shared,
        })
    }
}

fn build_output_stream(
    device: &Device,
    stream_config: StreamConfig,
    sample_format: SampleFormat,
    shared: Arc<PlaybackShared>,
) -> Result<Stream> {
    match sample_format {
        SampleFormat::F32 => device.build_output_stream(
            stream_config,
            move |data: &mut [f32], _| shared.fill_output(data),
            |_| {},
            None,
        ),
        SampleFormat::I16 => {
            let shared = shared.clone();
            device.build_output_stream(
                stream_config,
                move |data: &mut [i16], _| {
                    let mut temp = vec![0.0f32; data.len()];
                    shared.fill_output(&mut temp);
                    for (out, sample) in data.iter_mut().zip(temp) {
                        *out = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                    }
                },
                |_| {},
                None,
            )
        }
        SampleFormat::I32 => {
            let shared = shared.clone();
            device.build_output_stream(
                stream_config,
                move |data: &mut [i32], _| {
                    let mut temp = vec![0.0f32; data.len()];
                    shared.fill_output(&mut temp);
                    for (out, sample) in data.iter_mut().zip(temp) {
                        *out = (sample.clamp(-1.0, 1.0) * i32::MAX as f32) as i32;
                    }
                },
                |_| {},
                None,
            )
        }
        other => anyhow::bail!("unsupported output sample format: {other:?}"),
    }
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::DecodedAudio;

    fn shared(frames: usize) -> PlaybackShared {
        let audio = DecodedAudio {
            sample_rate: 44100,
            channels: vec![vec![0.0; frames]],
            peaks: vec![vec![]],
        };
        PlaybackShared::new(Arc::new(audio), 44100)
    }

    #[test]
    fn play_from_last_sample_without_loop_restarts_from_start() {
        let shared = shared(100);
        shared.set_position(99);
        shared.set_transport(TransportState::Playing);
        let mut out = vec![0.0; 8];
        shared.fill_output(&mut out);
        assert_eq!(shared.transport(), TransportState::Playing);
        assert_eq!(shared.position(), 8);
    }

    #[test]
    fn reaching_end_without_loop_stops_on_last_sample() {
        let shared = shared(100);
        shared.set_position(98);
        shared.set_transport(TransportState::Playing);
        let mut out = vec![0.0; 16];
        shared.fill_output(&mut out);
        assert_eq!(shared.transport(), TransportState::Stopped);
        assert_eq!(shared.position(), 99);
    }

    #[test]
    fn stale_callback_does_not_stop_restarted_play() {
        let shared = shared(100);
        shared.set_position(99);
        shared.set_transport(TransportState::Playing);
        shared.bump_epoch();
        shared.set_position(0);
        let mut out = vec![0.0; 8];
        shared.fill_output(&mut out);
        assert_eq!(shared.transport(), TransportState::Playing);
        assert_eq!(shared.position(), 8);
    }

    #[test]
    fn block_reads_preserve_source_samples() {
        let samples: Vec<f32> = (0..300).map(|i| i as f32).collect();
        let audio = DecodedAudio {
            sample_rate: 44100,
            channels: vec![samples.clone()],
            peaks: vec![vec![]],
        };
        let shared = PlaybackShared::new(Arc::new(audio), 44100);
        shared.set_transport(TransportState::Playing);
        let mut out = vec![0.0; 200];
        shared.fill_output(&mut out);
        assert_eq!(&out[..], &samples[..200]);
        assert_eq!(shared.position(), 200);
        assert!(PLAYBACK_READ_FRAMES >= 1);
        assert_eq!(PLAYBACK_READ_FRAMES, 128);
    }

    #[test]
    fn mono_source_upmixes_to_stereo_device() {
        let samples: Vec<f32> = (0..50).map(|i| i as f32).collect();
        let audio = DecodedAudio {
            sample_rate: 44100,
            channels: vec![samples.clone()],
            peaks: vec![vec![]],
        };
        let shared = PlaybackShared::with_output_layout(Arc::new(audio), 44100, 2);
        shared.set_transport(TransportState::Playing);
        let mut out = vec![0.0; 20];
        shared.fill_output(&mut out);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[1], 0.0);
        assert_eq!(out[2], 1.0);
        assert_eq!(out[3], 1.0);
        assert_eq!(shared.position(), 10);
    }

    #[test]
    fn monitor_subset_routes_last_two_channels() {
        let mut channels = vec![vec![0.0f32; 40]; 6];
        for i in 0..40 {
            channels[4][i] = 0.4;
            channels[5][i] = -0.6;
        }
        let audio = DecodedAudio {
            sample_rate: 44100,
            channels,
            peaks: vec![vec![]; 6],
        };
        let shared = PlaybackShared::with_output_layout(Arc::new(audio), 44100, 2);
        shared.set_monitor(Some(MonitorChain::Stereo), Some(vec![4, 5]));
        shared.set_transport(TransportState::Playing);
        let mut out = vec![0.0; 20];
        shared.fill_output(&mut out);
        assert!((out[0] - 0.4).abs() < 0.05, "{}", out[0]);
        assert!((out[1] + 0.6).abs() < 0.05, "{}", out[1]);
        assert!((out[2] - 0.4).abs() < 0.05);
        assert!((out[3] + 0.6).abs() < 0.05);
    }

    #[test]
    fn no_monitor_chain_keeps_direct_mapping() {
        let left: Vec<f32> = (0..20).map(|i| i as f32).collect();
        let right: Vec<f32> = (0..20).map(|i| 100.0 + i as f32).collect();
        let audio = DecodedAudio {
            sample_rate: 44100,
            channels: vec![left.clone(), right.clone()],
            peaks: vec![vec![], vec![]],
        };
        let shared = PlaybackShared::with_output_layout(Arc::new(audio), 44100, 2);
        shared.set_transport(TransportState::Playing);
        let mut out = vec![0.0; 8];
        shared.fill_output(&mut out);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[1], 100.0);
        assert_eq!(out[2], 1.0);
        assert_eq!(out[3], 101.0);
    }

    #[test]
    fn six_channel_file_routes_foa_or_stereo_subset() {
        let mut channels = vec![vec![0.0f32; 40]; 6];
        for i in 0..40 {
            channels[0][i] = 1.0;
            channels[4][i] = 0.4;
            channels[5][i] = -0.6;
        }
        let audio = DecodedAudio {
            sample_rate: 44100,
            channels: channels.clone(),
            peaks: vec![vec![]; 6],
        };
        let shared = PlaybackShared::with_output_layout(Arc::new(audio), 44100, 2);
        shared.set_monitor(Some(MonitorChain::Foa), Some(vec![0, 1, 2, 3]));
        shared.set_transport(TransportState::Playing);
        let mut out = vec![0.0; 20];
        shared.fill_output(&mut out);
        assert!((out[0] - 0.7071).abs() < 0.08, "{}", out[0]);
        assert!((out[1] - out[0]).abs() < 0.03);

        let audio = DecodedAudio {
            sample_rate: 44100,
            channels,
            peaks: vec![vec![]; 6],
        };
        let shared = PlaybackShared::with_output_layout(Arc::new(audio), 44100, 2);
        shared.set_monitor(Some(MonitorChain::Stereo), Some(vec![4, 5]));
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
        let audio = DecodedAudio {
            sample_rate: 44100,
            channels,
            peaks: vec![vec![]; 6],
        };
        let shared = PlaybackShared::with_output_layout(Arc::new(audio), 44100, 2);
        shared.set_monitor(Some(MonitorChain::FoaFuma), Some(vec![0, 1, 2, 3]));
        shared.set_transport(TransportState::Playing);
        let mut out = vec![0.0; 20];
        shared.fill_output(&mut out);
        assert!((out[0] - 0.7071).abs() < 0.08, "{}", out[0]);
        assert!((out[1] - out[0]).abs() < 0.03);
    }
}
