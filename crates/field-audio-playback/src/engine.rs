// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! CPAL output stream and shared playback state.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};

use super::monitor::{map_direct, MonitorProcess};
use super::provider::PlaybackDataProvider;
use super::transport::TransportState;

const IN_OUT_NONE: usize = usize::MAX;

/// Source frames fetched per provider call while filling the output device.
pub const PLAYBACK_READ_FRAMES: usize = 128;

/// Shared state between the UI thread and the audio callback.
pub struct PlaybackShared {
    /// Audio source.
    pub provider: Arc<dyn PlaybackDataProvider>,
    /// Current playhead sample.
    pub position: AtomicUsize,
    /// Transport wire value ([`TransportState::to_u8`]).
    pub transport: AtomicU8,
    /// Loop enable.
    pub looping: AtomicBool,
    /// Region in-point, or [`IN_OUT_NONE`].
    pub in_point: AtomicUsize,
    /// Region out-point, or [`IN_OUT_NONE`].
    pub out_point: AtomicUsize,
    epoch: AtomicUsize,
    output_rate: AtomicU32,
    output_channels: AtomicUsize,
    monitor: Mutex<Option<Arc<dyn MonitorProcess>>>,
    read_buf: Mutex<Vec<f32>>,
    gathered: Mutex<Vec<f32>>,
}

impl PlaybackShared {
    /// Create shared state with provider channel count as device channels.
    pub fn new(provider: Arc<dyn PlaybackDataProvider>, output_rate: u32) -> Self {
        let output_channels = provider.channel_count().max(1);
        Self::with_output_layout(provider, output_rate, output_channels)
    }

    /// Create shared state with an explicit device channel count.
    pub fn with_output_layout(
        provider: Arc<dyn PlaybackDataProvider>,
        output_rate: u32,
        output_channels: usize,
    ) -> Self {
        Self {
            output_rate: AtomicU32::new(output_rate.max(1)),
            output_channels: AtomicUsize::new(output_channels.max(1)),
            provider,
            position: AtomicUsize::new(0),
            transport: AtomicU8::new(TransportState::Stopped.to_u8()),
            looping: AtomicBool::new(false),
            in_point: AtomicUsize::new(IN_OUT_NONE),
            out_point: AtomicUsize::new(IN_OUT_NONE),
            epoch: AtomicUsize::new(0),
            monitor: Mutex::new(None),
            read_buf: Mutex::new(Vec::new()),
            gathered: Mutex::new(Vec::new()),
        }
    }

    /// Install or clear the monitor process used in the audio callback.
    pub fn set_monitor_process(&self, monitor: Option<Arc<dyn MonitorProcess>>) {
        *self.monitor.lock().expect("monitor mutex") = monitor;
    }

    /// Currently installed monitor, if any.
    pub fn monitor_process(&self) -> Option<Arc<dyn MonitorProcess>> {
        self.monitor.lock().expect("monitor mutex").clone()
    }

    /// Invalidate in-flight audio callbacks (e.g. after seek while playing).
    pub fn bump_epoch(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }

    /// Set transport state for the audio callback.
    pub fn set_transport(&self, state: TransportState) {
        self.transport.store(state.to_u8(), Ordering::SeqCst);
    }

    /// Read transport state.
    pub fn transport(&self) -> TransportState {
        TransportState::from_u8(self.transport.load(Ordering::SeqCst))
    }

    /// Set playhead sample.
    pub fn set_position(&self, sample: usize) {
        self.position.store(sample, Ordering::SeqCst);
    }

    /// Read playhead sample.
    pub fn position(&self) -> usize {
        self.position.load(Ordering::SeqCst)
    }

    /// Enable or disable looping.
    pub fn set_looping(&self, looping: bool) {
        self.looping.store(looping, Ordering::SeqCst);
    }

    /// Set optional in/out region bounds.
    pub fn set_in_out(&self, in_point: Option<usize>, out_point: Option<usize>) {
        self.in_point
            .store(in_point.unwrap_or(IN_OUT_NONE), Ordering::SeqCst);
        self.out_point
            .store(out_point.unwrap_or(IN_OUT_NONE), Ordering::SeqCst);
    }

    /// Device sample rate.
    pub fn output_rate(&self) -> u32 {
        self.output_rate.load(Ordering::SeqCst)
    }

    /// Update device layout and notify the monitor of the new rate.
    pub fn set_output_layout(&self, sample_rate: u32, channels: usize) {
        let sample_rate = sample_rate.max(1);
        let channels = channels.max(1);
        self.output_rate.store(sample_rate, Ordering::SeqCst);
        self.output_channels.store(channels, Ordering::SeqCst);
        if let Some(monitor) = self.monitor_process() {
            monitor.set_output_sample_rate(sample_rate);
        }
    }

    /// Set a monitor parameter (lock-free on the monitor side).
    pub fn set_monitor_param(&self, address: &str, value: f32) {
        if let Some(monitor) = self.monitor_process() {
            monitor.set_param(address, value);
        }
    }

    /// Read a monitor parameter.
    pub fn monitor_param(&self, address: &str) -> Option<f32> {
        self.monitor_process()?.get_param(address)
    }

    /// Read a monitor meter.
    pub fn monitor_meter(&self, address: &str) -> Option<f32> {
        self.monitor_process()?.meter(address)
    }

    /// Monitor UI JSON, if available.
    pub fn monitor_ui_json(&self) -> Option<&'static str> {
        self.monitor_process()?.ui_json()
    }

    /// Snapshot monitor meters.
    pub fn monitor_meters(&self) -> HashMap<String, f32> {
        self.monitor_process()
            .map(|m| m.meters())
            .unwrap_or_default()
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

    /// Fill an interleaved device buffer (audio callback entry).
    pub fn fill_output(&self, output: &mut [f32]) {
        output.fill(0.0);
        if self.transport() != TransportState::Playing {
            return;
        }

        let src_ch = self.provider.channel_count();
        if src_ch == 0 {
            return;
        }
        let out_ch = self.output_channels.load(Ordering::SeqCst).max(1);
        let out_frames = output.len() / out_ch;
        if out_frames == 0 {
            return;
        }

        let end = self.playback_end();
        let start_bound = self.playback_start();
        let looping = self.looping.load(Ordering::SeqCst);
        let src_rate = self.provider.sample_rate().max(1);
        let step = src_rate as f64 / f64::from(self.output_rate().max(1));
        let epoch = self.epoch.load(Ordering::SeqCst);
        let origin = self.position.load(Ordering::SeqCst);
        let mut pos_f = origin as f64;

        let mut read_buf = self.read_buf.lock().expect("read_buf");
        let mut gathered = self.gathered.lock().expect("gathered");
        let need_gather = (out_frames * src_ch).max(1);
        if gathered.len() < need_gather {
            gathered.resize(need_gather, 0.0);
        }

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
                if read_buf.len() < n {
                    read_buf.resize(n, 0.0);
                }
                self.provider
                    .read_interleaved(source_pos, take, &mut read_buf[..n]);
                buf_origin = source_pos;
                buf_frames = take;
            }
            let local = source_pos - buf_origin;
            let base = local * src_ch;
            let dest_base = produced * src_ch;
            let needed = dest_base + src_ch;
            if gathered.len() < needed {
                gathered.resize(needed, 0.0);
            }
            let n = src_ch.min(32);
            if base + n <= read_buf.len() {
                gathered[dest_base..dest_base + n].copy_from_slice(&read_buf[base..base + n]);
            }
            if src_ch > 32 {
                for i in 32..src_ch {
                    gathered[dest_base + i] = read_buf.get(base + i).copied().unwrap_or(0.0);
                }
            }
            produced += 1;
            pos_f += step;
        }

        if produced > 0 {
            let gather_len = produced * src_ch;
            let monitor = self.monitor.lock().expect("monitor mutex").clone();
            if let Some(monitor) = monitor {
                // Drop buffer locks before process to avoid holding them during DSP.
                let slice: Vec<f32> = gathered[..gather_len].to_vec();
                drop(gathered);
                drop(read_buf);
                monitor.process_gathered(&slice, src_ch, produced, output, out_ch);
            } else {
                map_direct(&gathered[..gather_len], src_ch, produced, output, out_ch);
                drop(gathered);
                drop(read_buf);
            }
        } else {
            drop(gathered);
            drop(read_buf);
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

/// Owns the CPAL output stream and shared playback state.
pub struct PlaybackEngine {
    _stream: Stream,
    /// Shared callback state.
    pub shared: Arc<PlaybackShared>,
}

impl PlaybackEngine {
    /// Open an output stream on `device` for `provider`.
    pub fn open(device: &Device, provider: Arc<dyn PlaybackDataProvider>) -> Result<Self> {
        let (stream, shared) = open_output_stream(device, provider)?;
        Ok(Self {
            _stream: stream,
            shared,
        })
    }

    /// Reopen the stream on a new device, preserving shared state.
    pub fn reopen(&mut self, device: &Device) -> Result<()> {
        self.shared.bump_epoch();
        let (stream, output_rate, output_channels) =
            build_playing_stream(device, self.shared.clone())?;
        self._stream = stream;
        self.shared.set_output_layout(output_rate, output_channels);
        Ok(())
    }
}

fn open_output_stream(
    device: &Device,
    provider: Arc<dyn PlaybackDataProvider>,
) -> Result<(Stream, Arc<PlaybackShared>)> {
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
    let stream = build_playing_stream_from_config(
        device,
        &default_config,
        sample_format,
        stream_config,
        shared.clone(),
    )?;
    Ok((stream, shared))
}

fn build_playing_stream(
    device: &Device,
    shared: Arc<PlaybackShared>,
) -> Result<(Stream, u32, usize)> {
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
    let stream = build_playing_stream_from_config(
        device,
        &default_config,
        sample_format,
        stream_config,
        shared,
    )?;
    Ok((stream, output_rate, output_channels))
}

fn build_playing_stream_from_config(
    device: &Device,
    default_config: &cpal::SupportedStreamConfig,
    sample_format: SampleFormat,
    stream_config: StreamConfig,
    shared: Arc<PlaybackShared>,
) -> Result<Stream> {
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
    Ok(stream)
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

    fn shared(frames: usize) -> PlaybackShared {
        PlaybackShared::new(
            Arc::new(PlanarAudio {
                sample_rate: 44100,
                channels: vec![vec![0.0; frames]],
            }),
            44100,
        )
    }

    #[test]
    fn set_output_layout_updates_rate_and_channels() {
        let shared = shared(20);
        shared.set_output_layout(48000, 2);
        assert_eq!(shared.output_rate(), 48000);
        shared.set_output_layout(44100, 2);
        shared.set_transport(TransportState::Playing);
        let mut out = vec![0.0; 8];
        shared.fill_output(&mut out);
        assert_eq!(shared.position(), 4);
    }

    #[test]
    fn playing_at_last_sample_without_loop_stops() {
        let shared = shared(100);
        shared.set_position(99);
        shared.set_transport(TransportState::Playing);
        let mut out = vec![0.0; 8];
        shared.fill_output(&mut out);
        assert_eq!(shared.transport(), TransportState::Stopped);
        assert_eq!(shared.position(), 99);
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
        let shared = PlaybackShared::new(
            Arc::new(PlanarAudio {
                sample_rate: 44100,
                channels: vec![samples.clone()],
            }),
            44100,
        );
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
        let shared = PlaybackShared::with_output_layout(
            Arc::new(PlanarAudio {
                sample_rate: 44100,
                channels: vec![samples],
            }),
            44100,
            2,
        );
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
    fn no_monitor_chain_keeps_direct_mapping() {
        let left: Vec<f32> = (0..20).map(|i| i as f32).collect();
        let right: Vec<f32> = (0..20).map(|i| 100.0 + i as f32).collect();
        let shared = PlaybackShared::with_output_layout(
            Arc::new(PlanarAudio {
                sample_rate: 44100,
                channels: vec![left, right],
            }),
            44100,
            2,
        );
        shared.set_transport(TransportState::Playing);
        let mut out = vec![0.0; 8];
        shared.fill_output(&mut out);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[1], 100.0);
        assert_eq!(out[2], 1.0);
        assert_eq!(out[3], 101.0);
    }
}
