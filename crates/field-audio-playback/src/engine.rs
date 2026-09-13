// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! CPAL output stream, shared playback state, and prefetch worker.
//!
//! # Realtime quality gates
//!
//! [`PlaybackShared::fill_output`] runs on the device callback thread and must:
//!
//! - allocate **no** heap memory
//! - take **no** blocking locks (no `Mutex`, no waiting `RwLock`)
//! - perform **no** file I/O or decode
//!
//! Heavy work (provider reads, sample-rate conversion, monitor DSP) runs on the
//! dedicated prefetch thread and pushes interleaved device frames into a
//! lock-free [`PrefetchRing`]. See the crate-level `AGENTS.md`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use arc_swap::{ArcSwap, ArcSwapOption};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};

use super::monitor::{map_direct, MonitorProcess};
use super::prefetch::{PrefetchRing, PREFETCH_CAPACITY_FRAMES, PREFETCH_CHUNK_FRAMES};
use super::provider::PlaybackDataProvider;
use super::transport::TransportState;

const IN_OUT_NONE: usize = usize::MAX;

/// Source frames fetched per provider call on the prefetch thread.
pub const PLAYBACK_READ_FRAMES: usize = 128;

/// Sized wrapper so [`ArcSwapOption`] can hold a trait object.
struct MonitorHandle(Arc<dyn MonitorProcess>);

/// Snapshot of realtime callback / stream health counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlaybackStats {
    /// Number of `fill_output` invocations while playing.
    pub callbacks: u64,
    /// Callbacks whose wall time exceeded the device buffer duration.
    pub slow_callbacks: u64,
    /// CPAL stream error callback invocations (xruns / device faults).
    pub stream_errors: u64,
    /// Callbacks that could not fully fill from the prefetch ring.
    pub underruns: u64,
    /// Longest single callback (nanoseconds).
    pub max_callback_ns: u64,
    /// Sum of callback wall times (nanoseconds).
    pub total_callback_ns: u64,
    /// Provider `read_interleaved` calls from the **prefetch** thread.
    pub provider_reads: u64,
    /// Longest provider read (nanoseconds).
    pub max_provider_read_ns: u64,
    /// Sum of provider read wall times (nanoseconds).
    pub total_provider_read_ns: u64,
    /// Largest output frame count observed in a callback.
    pub max_out_frames: u64,
    /// Source position at the start of the slowest callback (if any).
    pub slowest_at_sample: u64,
    /// Latest prefetch ring depth in device frames.
    pub prefetch_depth_frames: u64,
    /// High-water prefetch depth in device frames.
    pub max_prefetch_depth_frames: u64,
}

/// Shared state between the UI thread, prefetch worker, and audio callback.
pub struct PlaybackShared {
    /// Audio source (touched only by the prefetch thread while playing).
    pub provider: Arc<dyn PlaybackDataProvider>,
    /// Current audible playhead sample.
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
    monitor: ArcSwapOption<MonitorHandle>,
    ring: ArcSwap<PrefetchRing>,
    timeline_ended: AtomicBool,
    /// Prefetch-only scratch (never touched by the audio callback).
    prefetch_scratch: Mutex<PrefetchScratch>,
    callbacks: AtomicU64,
    slow_callbacks: AtomicU64,
    stream_errors: AtomicU64,
    underruns: AtomicU64,
    max_callback_ns: AtomicU64,
    total_callback_ns: AtomicU64,
    provider_reads: AtomicU64,
    max_provider_read_ns: AtomicU64,
    total_provider_read_ns: AtomicU64,
    max_out_frames: AtomicU64,
    slowest_at_sample: AtomicUsize,
    prefetch_depth_frames: AtomicU64,
    max_prefetch_depth_frames: AtomicU64,
}

struct PrefetchScratch {
    read_buf: Vec<f32>,
    gathered: Vec<f32>,
    device_chunk: Vec<f32>,
    fill_pos_f: f64,
    local_epoch: usize,
}

impl PrefetchScratch {
    fn new() -> Self {
        Self {
            read_buf: Vec::new(),
            gathered: Vec::new(),
            device_chunk: Vec::new(),
            fill_pos_f: 0.0,
            // Force the first chunk to sync from the public playhead.
            local_epoch: usize::MAX,
        }
    }
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
        let output_channels = output_channels.max(1);
        Self {
            output_rate: AtomicU32::new(output_rate.max(1)),
            output_channels: AtomicUsize::new(output_channels),
            provider,
            position: AtomicUsize::new(0),
            transport: AtomicU8::new(TransportState::Stopped.to_u8()),
            looping: AtomicBool::new(false),
            in_point: AtomicUsize::new(IN_OUT_NONE),
            out_point: AtomicUsize::new(IN_OUT_NONE),
            epoch: AtomicUsize::new(0),
            monitor: ArcSwapOption::from(None::<Arc<MonitorHandle>>),
            ring: ArcSwap::from_pointee(PrefetchRing::new(
                PREFETCH_CAPACITY_FRAMES,
                output_channels,
            )),
            timeline_ended: AtomicBool::new(false),
            prefetch_scratch: Mutex::new(PrefetchScratch::new()),
            callbacks: AtomicU64::new(0),
            slow_callbacks: AtomicU64::new(0),
            stream_errors: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            max_callback_ns: AtomicU64::new(0),
            total_callback_ns: AtomicU64::new(0),
            provider_reads: AtomicU64::new(0),
            max_provider_read_ns: AtomicU64::new(0),
            total_provider_read_ns: AtomicU64::new(0),
            max_out_frames: AtomicU64::new(0),
            slowest_at_sample: AtomicUsize::new(0),
            prefetch_depth_frames: AtomicU64::new(0),
            max_prefetch_depth_frames: AtomicU64::new(0),
        }
    }

    /// Snapshot realtime / prefetch performance counters.
    pub fn stats(&self) -> PlaybackStats {
        PlaybackStats {
            callbacks: self.callbacks.load(Ordering::Relaxed),
            slow_callbacks: self.slow_callbacks.load(Ordering::Relaxed),
            stream_errors: self.stream_errors.load(Ordering::Relaxed),
            underruns: self.underruns.load(Ordering::Relaxed),
            max_callback_ns: self.max_callback_ns.load(Ordering::Relaxed),
            total_callback_ns: self.total_callback_ns.load(Ordering::Relaxed),
            provider_reads: self.provider_reads.load(Ordering::Relaxed),
            max_provider_read_ns: self.max_provider_read_ns.load(Ordering::Relaxed),
            total_provider_read_ns: self.total_provider_read_ns.load(Ordering::Relaxed),
            max_out_frames: self.max_out_frames.load(Ordering::Relaxed),
            slowest_at_sample: self.slowest_at_sample.load(Ordering::Relaxed) as u64,
            prefetch_depth_frames: self.prefetch_depth_frames.load(Ordering::Relaxed),
            max_prefetch_depth_frames: self.max_prefetch_depth_frames.load(Ordering::Relaxed),
        }
    }

    /// Clear performance counters.
    pub fn reset_stats(&self) {
        self.callbacks.store(0, Ordering::Relaxed);
        self.slow_callbacks.store(0, Ordering::Relaxed);
        self.stream_errors.store(0, Ordering::Relaxed);
        self.underruns.store(0, Ordering::Relaxed);
        self.max_callback_ns.store(0, Ordering::Relaxed);
        self.total_callback_ns.store(0, Ordering::Relaxed);
        self.provider_reads.store(0, Ordering::Relaxed);
        self.max_provider_read_ns.store(0, Ordering::Relaxed);
        self.total_provider_read_ns.store(0, Ordering::Relaxed);
        self.max_out_frames.store(0, Ordering::Relaxed);
        self.slowest_at_sample.store(0, Ordering::Relaxed);
        self.prefetch_depth_frames.store(0, Ordering::Relaxed);
        self.max_prefetch_depth_frames.store(0, Ordering::Relaxed);
    }

    /// Record a CPAL stream error (xrun / device fault).
    pub fn record_stream_error(&self) {
        self.stream_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Install or clear the monitor process used by the prefetch thread.
    pub fn set_monitor_process(&self, monitor: Option<Arc<dyn MonitorProcess>>) {
        self.monitor
            .store(monitor.map(|m| Arc::new(MonitorHandle(m))));
    }

    /// Currently installed monitor, if any.
    pub fn monitor_process(&self) -> Option<Arc<dyn MonitorProcess>> {
        self.monitor.load_full().map(|h| h.0.clone())
    }

    /// Invalidate in-flight audio / prefetch work (e.g. after seek).
    pub fn bump_epoch(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.timeline_ended.store(false, Ordering::SeqCst);
    }

    /// Set transport state.
    pub fn set_transport(&self, state: TransportState) {
        if state == TransportState::Playing {
            self.timeline_ended.store(false, Ordering::SeqCst);
        }
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
        let ring = self.ring.load_full();
        if ring.channels() != channels {
            self.ring.store(Arc::new(PrefetchRing::new(
                PREFETCH_CAPACITY_FRAMES,
                channels,
            )));
            self.bump_epoch();
        }
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

    /// Produce up to one prefetch chunk into the ring (prefetch thread / tests).
    ///
    /// Returns `true` when any device frames were pushed.
    pub fn prefetch_chunk(&self) -> bool {
        let mut scratch = self
            .prefetch_scratch
            .lock()
            .expect("prefetch scratch mutex");
        self.prefetch_chunk_locked(&mut scratch)
    }

    fn prefetch_chunk_locked(&self, scratch: &mut PrefetchScratch) -> bool {
        let epoch = self.epoch.load(Ordering::SeqCst);
        if scratch.local_epoch != epoch {
            scratch.local_epoch = epoch;
            scratch.fill_pos_f = self.position.load(Ordering::SeqCst) as f64;
            self.ring.load_full().discard_unread();
            self.timeline_ended.store(false, Ordering::SeqCst);
        }

        if self.transport() != TransportState::Playing {
            return false;
        }
        if self.timeline_ended.load(Ordering::SeqCst) {
            return false;
        }

        let ring = self.ring.load_full();
        let out_ch = self.output_channels.load(Ordering::SeqCst).max(1);
        if ring.channels() != out_ch {
            return false;
        }
        if ring.frames_free() < PREFETCH_CHUNK_FRAMES {
            return false;
        }

        let src_ch = self.provider.channel_count();
        if src_ch == 0 {
            return false;
        }

        let end = self.playback_end();
        let start_bound = self.playback_start();
        let looping = self.looping.load(Ordering::SeqCst);
        let src_rate = self.provider.sample_rate().max(1);
        let out_rate = self.output_rate().max(1);
        let step = src_rate as f64 / f64::from(out_rate);

        let need_gather = PREFETCH_CHUNK_FRAMES * src_ch;
        if scratch.gathered.len() < need_gather {
            scratch.gathered.resize(need_gather, 0.0);
        }
        let need_device = PREFETCH_CHUNK_FRAMES * out_ch;
        if scratch.device_chunk.len() < need_device {
            scratch.device_chunk.resize(need_device, 0.0);
        }

        let mut buf_origin = 0usize;
        let mut buf_frames = 0usize;
        let mut produced = 0usize;
        let mut reached_end = false;

        for _ in 0..PREFETCH_CHUNK_FRAMES {
            let source_pos = scratch.fill_pos_f as usize;
            if source_pos > end {
                if looping && end > start_bound {
                    scratch.fill_pos_f = start_bound as f64;
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
                if scratch.read_buf.len() < n {
                    scratch.read_buf.resize(n, 0.0);
                }
                let read_started = Instant::now();
                self.provider
                    .read_interleaved(source_pos, take, &mut scratch.read_buf[..n]);
                let read_ns = read_started.elapsed().as_nanos() as u64;
                self.provider_reads.fetch_add(1, Ordering::Relaxed);
                self.total_provider_read_ns
                    .fetch_add(read_ns, Ordering::Relaxed);
                self.max_provider_read_ns
                    .fetch_max(read_ns, Ordering::Relaxed);
                buf_origin = source_pos;
                buf_frames = take;
            }

            let local = source_pos - buf_origin;
            let base = local * src_ch;
            let dest_base = produced * src_ch;
            let n = src_ch.min(32);
            if base + n <= scratch.read_buf.len() {
                scratch.gathered[dest_base..dest_base + n]
                    .copy_from_slice(&scratch.read_buf[base..base + n]);
            }
            if src_ch > 32 {
                for i in 32..src_ch {
                    scratch.gathered[dest_base + i] =
                        scratch.read_buf.get(base + i).copied().unwrap_or(0.0);
                }
            }
            produced += 1;
            scratch.fill_pos_f += step;
        }

        if produced == 0 {
            if reached_end && !looping {
                self.timeline_ended.store(true, Ordering::SeqCst);
            }
            return false;
        }

        let gather_len = produced * src_ch;
        scratch.device_chunk[..produced * out_ch].fill(0.0);
        if let Some(handle) = self.monitor.load_full() {
            handle.0.process_gathered(
                &scratch.gathered[..gather_len],
                src_ch,
                produced,
                &mut scratch.device_chunk[..produced * out_ch],
                out_ch,
            );
        } else {
            map_direct(
                &scratch.gathered[..gather_len],
                src_ch,
                produced,
                &mut scratch.device_chunk[..produced * out_ch],
                out_ch,
            );
        }

        let pushed = ring.push_interleaved(&scratch.device_chunk[..produced * out_ch]);
        let depth = ring.frames_available() as u64;
        self.prefetch_depth_frames.store(depth, Ordering::Relaxed);
        self.max_prefetch_depth_frames
            .fetch_max(depth, Ordering::Relaxed);

        if reached_end && !looping {
            self.timeline_ended.store(true, Ordering::SeqCst);
        }
        pushed > 0
    }

    /// Drain the prefetch ring into a device buffer (audio callback entry).
    ///
    /// # Realtime contract
    ///
    /// No heap allocation and no blocking locks. See module docs.
    pub fn fill_output(&self, output: &mut [f32]) {
        let started = Instant::now();
        output.fill(0.0);
        if self.transport() != TransportState::Playing {
            return;
        }

        let out_ch = self.output_channels.load(Ordering::SeqCst).max(1);
        let out_frames = output.len() / out_ch;
        if out_frames == 0 {
            return;
        }
        self.max_out_frames
            .fetch_max(out_frames as u64, Ordering::Relaxed);

        let epoch = self.epoch.load(Ordering::SeqCst);
        let origin = self.position.load(Ordering::SeqCst);
        let src_rate = self.provider.sample_rate().max(1);
        let out_rate = self.output_rate().max(1);
        let step = src_rate as f64 / f64::from(out_rate);

        let ring = self.ring.load_full();
        let got = if ring.channels() == out_ch {
            ring.pop_interleaved(output)
        } else {
            0
        };
        if got < out_frames {
            self.underruns.fetch_add(1, Ordering::Relaxed);
        }

        let end = self.playback_end();
        let looping = self.looping.load(Ordering::SeqCst);
        let pos_f = origin as f64 + got as f64 * step;
        let ended = self.timeline_ended.load(Ordering::SeqCst)
            && got < out_frames
            && ring.frames_available() == 0;

        if self.epoch.load(Ordering::SeqCst) != epoch {
            return;
        }

        if ended && !looping {
            self.set_transport(TransportState::Stopped);
            self.position.store(end, Ordering::SeqCst);
        } else {
            let final_pos = pos_f.min(end as f64) as usize;
            self.position.store(final_pos, Ordering::SeqCst);
        }

        let callback_ns = started.elapsed().as_nanos() as u64;
        self.callbacks.fetch_add(1, Ordering::Relaxed);
        self.total_callback_ns
            .fetch_add(callback_ns, Ordering::Relaxed);
        let prev_max = self.max_callback_ns.load(Ordering::Relaxed);
        if callback_ns > prev_max {
            self.max_callback_ns.store(callback_ns, Ordering::Relaxed);
            self.slowest_at_sample.store(origin, Ordering::Relaxed);
        }
        let budget_ns = (out_frames as u64)
            .saturating_mul(1_000_000_000)
            .saturating_div(u64::from(out_rate));
        if budget_ns > 0 && callback_ns > budget_ns {
            self.slow_callbacks.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn prefetch_loop(shared: Arc<PlaybackShared>, stop: Arc<AtomicBool>) {
    let mut scratch = PrefetchScratch::new();
    scratch.local_epoch = shared.epoch.load(Ordering::SeqCst);
    scratch.fill_pos_f = shared.position.load(Ordering::SeqCst) as f64;
    while !stop.load(Ordering::Relaxed) {
        if shared.transport() != TransportState::Playing {
            thread::sleep(Duration::from_millis(2));
            continue;
        }
        let produced = shared.prefetch_chunk_locked(&mut scratch);
        if !produced {
            thread::sleep(Duration::from_millis(1));
        }
    }
}

/// Owns the CPAL output stream, shared playback state, and prefetch worker.
pub struct PlaybackEngine {
    _stream: Stream,
    /// Shared callback state.
    pub shared: Arc<PlaybackShared>,
    prefetch_stop: Arc<AtomicBool>,
    prefetch_join: Option<JoinHandle<()>>,
}

impl PlaybackEngine {
    /// Open an output stream on `device` for `provider` and start prefetch.
    pub fn open(device: &Device, provider: Arc<dyn PlaybackDataProvider>) -> Result<Self> {
        let (stream, shared) = open_output_stream(device, provider)?;
        let prefetch_stop = Arc::new(AtomicBool::new(false));
        let prefetch_join = Some(spawn_prefetch(shared.clone(), prefetch_stop.clone())?);
        Ok(Self {
            _stream: stream,
            shared,
            prefetch_stop,
            prefetch_join,
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

impl Drop for PlaybackEngine {
    fn drop(&mut self) {
        self.prefetch_stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.prefetch_join.take() {
            let _ = join.join();
        }
    }
}

fn spawn_prefetch(shared: Arc<PlaybackShared>, stop: Arc<AtomicBool>) -> Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("fa-prefetch".into())
        .spawn(move || prefetch_loop(shared, stop))
        .context("spawn prefetch thread")
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
    let max_frames = match stream_config.buffer_size {
        cpal::BufferSize::Fixed(frames) => frames as usize,
        cpal::BufferSize::Default => 8192,
    };
    let channels = stream_config.channels as usize;
    match sample_format {
        SampleFormat::F32 => {
            let err = shared.clone();
            device.build_output_stream(
                stream_config.clone(),
                move |data: &mut [f32], _| shared.fill_output(data),
                move |e| {
                    eprintln!("playback stream error: {e}");
                    err.record_stream_error();
                },
                None,
            )
        }
        SampleFormat::I16 => {
            let err = shared.clone();
            let shared = shared.clone();
            let mut scratch = vec![0.0f32; max_frames.saturating_mul(channels).max(1)];
            device.build_output_stream(
                stream_config.clone(),
                move |data: &mut [i16], _| {
                    if scratch.len() < data.len() {
                        scratch.resize(data.len(), 0.0);
                    }
                    let tmp = &mut scratch[..data.len()];
                    shared.fill_output(tmp);
                    for (out, sample) in data.iter_mut().zip(tmp.iter()) {
                        *out = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                    }
                },
                move |e| {
                    eprintln!("playback stream error: {e}");
                    err.record_stream_error();
                },
                None,
            )
        }
        SampleFormat::I32 => {
            let err = shared.clone();
            let shared = shared.clone();
            let mut scratch = vec![0.0f32; max_frames.saturating_mul(channels).max(1)];
            device.build_output_stream(
                stream_config,
                move |data: &mut [i32], _| {
                    if scratch.len() < data.len() {
                        scratch.resize(data.len(), 0.0);
                    }
                    let tmp = &mut scratch[..data.len()];
                    shared.fill_output(tmp);
                    for (out, sample) in data.iter_mut().zip(tmp.iter()) {
                        *out = (sample.clamp(-1.0, 1.0) * i32::MAX as f32) as i32;
                    }
                },
                move |e| {
                    eprintln!("playback stream error: {e}");
                    err.record_stream_error();
                },
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

    fn play(shared: &PlaybackShared, out: &mut [f32]) {
        shared.bump_epoch();
        shared.set_transport(TransportState::Playing);
        // Prefetch until the ring can satisfy this callback or playback ends.
        for _ in 0..64 {
            let ring = shared.ring.load_full();
            let need = out.len() / ring.channels().max(1);
            if ring.frames_available() >= need || shared.timeline_ended.load(Ordering::SeqCst) {
                break;
            }
            if !shared.prefetch_chunk() {
                break;
            }
        }
        shared.fill_output(out);
    }

    #[test]
    fn set_output_layout_updates_rate_and_channels() {
        let shared = shared(20);
        shared.set_output_layout(48000, 2);
        assert_eq!(shared.output_rate(), 48000);
        shared.set_output_layout(44100, 1);
        let mut out = vec![0.0; 8];
        play(&shared, &mut out);
        assert!(shared.position() > 0);
    }

    #[test]
    fn playing_at_last_sample_without_loop_stops() {
        let shared = shared(100);
        shared.set_position(99);
        let mut out = vec![0.0; 8];
        play(&shared, &mut out);
        assert_eq!(shared.transport(), TransportState::Stopped);
        assert_eq!(shared.position(), 99);
    }

    #[test]
    fn reaching_end_without_loop_stops_on_last_sample() {
        let shared = shared(100);
        shared.set_position(98);
        let mut out = vec![0.0; 16];
        play(&shared, &mut out);
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
        play(&shared, &mut out);
        assert_eq!(shared.transport(), TransportState::Playing);
        assert!(shared.position() > 0);
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
        let mut out = vec![0.0; 200];
        play(&shared, &mut out);
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
        let mut out = vec![0.0; 20];
        play(&shared, &mut out);
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
        let mut out = vec![0.0; 8];
        play(&shared, &mut out);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[1], 100.0);
        assert_eq!(out[2], 1.0);
        assert_eq!(out[3], 101.0);
    }
}
