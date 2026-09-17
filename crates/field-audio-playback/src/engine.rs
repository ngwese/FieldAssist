// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

//! CPAL output stream, shared playback state, and prefetch worker.
//!
//! # Realtime quality gates
//!
//! [`PlaybackShared::fill_output`] runs on the device callback thread and must:
//!
//! - allocate **no** heap memory
//! - take **no** blocking locks (no waiting `Mutex` / `RwLock`)
//! - perform **no** file I/O or decode
//!
//! Provider reads and **bandlimited** sample-rate conversion run on the
//! dedicated prefetch thread, which pushes **pre-monitor** device-rate
//! interleaved source frames into a lock-free [`PrefetchRing`]. Matched rates
//! copy bit-exactly (no resampler). The callback pops those frames, runs
//! monitor DSP (or Direct channel mapping), and writes the device buffer. See
//! the crate-level `AGENTS.md`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use arc_swap::{ArcSwap, ArcSwapOption};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};

use super::faults::PlaybackFaults;
use super::monitor::{map_direct, MonitorProcess};
use super::prefetch::{PrefetchRing, PREFETCH_CAPACITY_FRAMES, PREFETCH_CHUNK_FRAMES};
use super::provider::PlaybackDataProvider;
use super::src_convert::StreamingSrc;
use super::transport::TransportState;

/// Faust `Meter/Input*` bargraphs floor at −90 dB; treat near-floor as quiet.
const INPUT_METER_QUIET_DB: f32 = -89.0;
/// Max frames covered by the callback gather scratch (matches typical Default).
const CALLBACK_MAX_FRAMES: usize = 8192;

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
    /// Most recent callback's device-buffer budget (`out_frames / rate`).
    pub last_budget_ns: u64,
    /// Most recent callback wall time.
    pub last_callback_ns: u64,
    /// Most recent spare time before an xrun (`budget − callback`).
    pub last_headroom_ns: u64,
    /// Worst (smallest) headroom observed since [`PlaybackShared::reset_stats`].
    pub min_headroom_ns: u64,
    /// Frame count of the most recent callback.
    pub last_out_frames: u64,
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
    /// Preallocated gather buffer for the callback (`src_ch × CALLBACK_MAX_FRAMES`).
    callback_scratch: ArcSwap<Box<[f32]>>,
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
    last_budget_ns: AtomicU64,
    last_callback_ns: AtomicU64,
    last_headroom_ns: AtomicU64,
    min_headroom_ns: AtomicU64,
    last_out_frames: AtomicU64,
    /// Last CPAL stream-error `Display` text (error callback may allocate).
    last_stream_error: ArcSwapOption<String>,
    /// [`Self::underruns`] value last returned by [`Self::take_faults`].
    fault_underrun_cursor: AtomicU64,
    /// [`Self::stream_errors`] value last returned by [`Self::take_faults`].
    fault_stream_error_cursor: AtomicU64,
}

struct PrefetchScratch {
    read_buf: Vec<f32>,
    gathered: Vec<f32>,
    /// Fractional source cursor used only on the matched-rate copy path.
    fill_pos_f: f64,
    /// Integer source cursor advanced by the bandlimited SRC path.
    fill_pos: usize,
    local_epoch: usize,
    src: Option<StreamingSrc>,
}

impl PrefetchScratch {
    fn new() -> Self {
        Self {
            read_buf: Vec::new(),
            gathered: Vec::new(),
            fill_pos_f: 0.0,
            fill_pos: 0,
            // Force the first chunk to sync from the public playhead.
            local_epoch: usize::MAX,
            src: None,
        }
    }

    fn sync_from_playhead(&mut self, sample: usize) {
        self.fill_pos_f = sample as f64;
        self.fill_pos = sample;
        if let Some(src) = self.src.as_mut() {
            src.reset();
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
        let src_ch = provider.channel_count().max(1);
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
            ring: ArcSwap::from_pointee(PrefetchRing::new(PREFETCH_CAPACITY_FRAMES, src_ch)),
            callback_scratch: ArcSwap::from_pointee(
                vec![0.0f32; CALLBACK_MAX_FRAMES.saturating_mul(src_ch)].into_boxed_slice(),
            ),
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
            last_budget_ns: AtomicU64::new(0),
            last_callback_ns: AtomicU64::new(0),
            last_headroom_ns: AtomicU64::new(0),
            min_headroom_ns: AtomicU64::new(u64::MAX),
            last_out_frames: AtomicU64::new(0),
            last_stream_error: ArcSwapOption::empty(),
            fault_underrun_cursor: AtomicU64::new(0),
            fault_stream_error_cursor: AtomicU64::new(0),
        }
    }

    /// Snapshot realtime / prefetch performance counters.
    pub fn stats(&self) -> PlaybackStats {
        let min_headroom = self.min_headroom_ns.load(Ordering::Relaxed);
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
            last_budget_ns: self.last_budget_ns.load(Ordering::Relaxed),
            last_callback_ns: self.last_callback_ns.load(Ordering::Relaxed),
            last_headroom_ns: self.last_headroom_ns.load(Ordering::Relaxed),
            min_headroom_ns: if min_headroom == u64::MAX {
                0
            } else {
                min_headroom
            },
            last_out_frames: self.last_out_frames.load(Ordering::Relaxed),
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
        self.last_budget_ns.store(0, Ordering::Relaxed);
        self.last_callback_ns.store(0, Ordering::Relaxed);
        self.last_headroom_ns.store(0, Ordering::Relaxed);
        self.min_headroom_ns.store(u64::MAX, Ordering::Relaxed);
        self.last_out_frames.store(0, Ordering::Relaxed);
        self.last_stream_error.store(None);
        self.fault_underrun_cursor.store(0, Ordering::Relaxed);
        self.fault_stream_error_cursor.store(0, Ordering::Relaxed);
    }

    fn record_callback_timing(&self, out_frames: usize, out_rate: u32, callback_ns: u64) {
        self.callbacks.fetch_add(1, Ordering::Relaxed);
        self.total_callback_ns
            .fetch_add(callback_ns, Ordering::Relaxed);
        self.last_callback_ns.store(callback_ns, Ordering::Relaxed);
        self.last_out_frames
            .store(out_frames as u64, Ordering::Relaxed);
        let budget_ns = (out_frames as u64)
            .saturating_mul(1_000_000_000)
            .saturating_div(u64::from(out_rate.max(1)));
        let headroom_ns = budget_ns.saturating_sub(callback_ns);
        self.last_budget_ns.store(budget_ns, Ordering::Relaxed);
        self.last_headroom_ns.store(headroom_ns, Ordering::Relaxed);
        self.min_headroom_ns
            .fetch_min(headroom_ns, Ordering::Relaxed);
        if budget_ns > 0 && callback_ns > budget_ns {
            self.slow_callbacks.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a CPAL stream error (xrun / device fault).
    pub fn record_stream_error(&self) {
        self.stream_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a CPAL stream error and keep its `Display` text for
    /// [`Self::take_faults`]. Safe to call from the stream-error callback
    /// (may allocate; that callback is not the output fill path).
    pub fn record_stream_error_detail(&self, detail: String) {
        self.record_stream_error();
        self.last_stream_error.store(Some(Arc::new(detail)));
    }

    /// Drain underrun / stream-error deltas since the previous call.
    ///
    /// Intended for the UI / prefetch side — not the CPAL output callback.
    /// Counters on [`PlaybackStats`] are left intact.
    pub fn take_faults(&self) -> Option<PlaybackFaults> {
        let underruns = self.underruns.load(Ordering::Relaxed);
        let stream_errors = self.stream_errors.load(Ordering::Relaxed);
        let prev_underruns = self
            .fault_underrun_cursor
            .swap(underruns, Ordering::Relaxed);
        let prev_errors = self
            .fault_stream_error_cursor
            .swap(stream_errors, Ordering::Relaxed);
        let underruns = underruns.saturating_sub(prev_underruns);
        let stream_errors = stream_errors.saturating_sub(prev_errors);
        if underruns == 0 && stream_errors == 0 {
            return None;
        }
        let last_stream_error = if stream_errors > 0 {
            self.last_stream_error.swap(None).map(|s| (*s).clone())
        } else {
            None
        };
        Some(PlaybackFaults {
            underruns,
            stream_errors,
            last_stream_error,
        })
    }

    /// Install or clear the monitor process used by the **realtime callback**.
    pub fn set_monitor_process(&self, monitor: Option<Arc<dyn MonitorProcess>>) {
        self.monitor
            .store(monitor.map(|m| Arc::new(MonitorHandle(m))));
    }

    /// Currently installed monitor, if any.
    pub fn monitor_process(&self) -> Option<Arc<dyn MonitorProcess>> {
        self.monitor.load_full().map(|h| h.0.clone())
    }

    fn ensure_ring_and_scratch_for_source(&self, src_ch: usize) {
        let src_ch = src_ch.max(1);
        let ring = self.ring.load_full();
        if ring.channels() != src_ch {
            self.ring.store(Arc::new(PrefetchRing::new(
                PREFETCH_CAPACITY_FRAMES,
                src_ch,
            )));
            self.bump_epoch();
        }
        let need = CALLBACK_MAX_FRAMES.saturating_mul(src_ch);
        let scratch = self.callback_scratch.load_full();
        if scratch.len() < need {
            self.callback_scratch
                .store(Arc::new(vec![0.0f32; need].into_boxed_slice()));
        }
    }

    /// Invalidate in-flight audio / prefetch work (e.g. after seek).
    pub fn bump_epoch(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.timeline_ended.store(false, Ordering::SeqCst);
    }

    /// Seek the audible playhead and resync prefetch (position then epoch).
    ///
    /// Order matters: prefetch reads `position` when it observes a new epoch.
    pub fn seek_to(&self, sample: usize) {
        self.position.store(sample, Ordering::SeqCst);
        self.bump_epoch();
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

    /// Set playhead sample (no prefetch resync; prefer [`Self::seek_to`] while playing).
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
        let rate_changed = self.output_rate() != sample_rate;
        self.output_rate.store(sample_rate, Ordering::SeqCst);
        self.output_channels.store(channels, Ordering::SeqCst);
        // Ring carries pre-monitor source channels, not device channels.
        self.ensure_ring_and_scratch_for_source(self.provider.channel_count());
        if rate_changed {
            // Drop device-rate frames produced at the previous ratio.
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
    /// Returns `true` when any **pre-monitor** source frames were pushed.
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
            scratch.sync_from_playhead(self.position.load(Ordering::SeqCst));
            self.ring.load_full().discard_unread();
            self.timeline_ended.store(false, Ordering::SeqCst);
        }

        if self.transport() != TransportState::Playing {
            return false;
        }
        if self.timeline_ended.load(Ordering::SeqCst) {
            return false;
        }

        let src_ch = self.provider.channel_count();
        if src_ch == 0 {
            return false;
        }
        self.ensure_ring_and_scratch_for_source(src_ch);

        let ring = self.ring.load_full();
        if ring.channels() != src_ch {
            return false;
        }
        if ring.frames_free() < PREFETCH_CHUNK_FRAMES {
            return false;
        }

        let end = self.playback_end();
        let start_bound = self.playback_start();
        let looping = self.looping.load(Ordering::SeqCst);
        let src_rate = self.provider.sample_rate().max(1);
        let out_rate = self.output_rate().max(1);

        if src_rate == out_rate {
            self.prefetch_matched_rate(
                scratch,
                &ring,
                src_ch,
                end,
                start_bound,
                looping,
                src_rate,
                out_rate,
            )
        } else {
            self.prefetch_resampled(
                scratch,
                &ring,
                src_ch,
                end,
                start_bound,
                looping,
                src_rate,
                out_rate,
            )
        }
    }

    fn prefetch_matched_rate(
        &self,
        scratch: &mut PrefetchScratch,
        ring: &PrefetchRing,
        src_ch: usize,
        end: usize,
        start_bound: usize,
        looping: bool,
        src_rate: u32,
        out_rate: u32,
    ) -> bool {
        scratch.src = None;
        let step = src_rate as f64 / f64::from(out_rate);

        let need_gather = PREFETCH_CHUNK_FRAMES * src_ch;
        if scratch.gathered.len() < need_gather {
            scratch.gathered.resize(need_gather, 0.0);
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
                    scratch.fill_pos = start_bound;
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
            scratch.fill_pos = scratch.fill_pos_f as usize;
        }

        if produced == 0 {
            if reached_end && !looping {
                self.timeline_ended.store(true, Ordering::SeqCst);
            }
            return false;
        }

        let gather_len = produced * src_ch;
        let pushed = ring.push_interleaved(&scratch.gathered[..gather_len]);
        let depth = ring.frames_available() as u64;
        self.prefetch_depth_frames.store(depth, Ordering::Relaxed);
        self.max_prefetch_depth_frames
            .fetch_max(depth, Ordering::Relaxed);

        if reached_end && !looping {
            self.timeline_ended.store(true, Ordering::SeqCst);
        }
        pushed > 0
    }

    fn prefetch_resampled(
        &self,
        scratch: &mut PrefetchScratch,
        ring: &PrefetchRing,
        src_ch: usize,
        end: usize,
        start_bound: usize,
        looping: bool,
        src_rate: u32,
        out_rate: u32,
    ) -> bool {
        let needs_new = scratch
            .src
            .as_ref()
            .map(|s| !s.matches(src_rate, out_rate, src_ch))
            .unwrap_or(true);
        if needs_new {
            match StreamingSrc::new(src_rate, out_rate, src_ch) {
                Ok(src) => scratch.src = Some(src),
                Err(err) => {
                    eprintln!("playback SRC init failed: {err:#}");
                    return false;
                }
            }
        }

        // If the cursor is already past the end, wrap or finish before reading.
        if scratch.fill_pos > end {
            if looping && end > start_bound {
                scratch.sync_from_playhead(start_bound);
            } else {
                self.timeline_ended.store(true, Ordering::SeqCst);
                return false;
            }
        }

        let need_in = scratch
            .src
            .as_ref()
            .map(|s| s.input_frames_next())
            .unwrap_or(0);
        let out_next = scratch
            .src
            .as_ref()
            .map(|s| s.output_frames_next())
            .unwrap_or(0);
        if out_next == 0 {
            return false;
        }

        let need_in_samples = need_in.saturating_mul(src_ch);
        if need_in_samples > 0 {
            if scratch.read_buf.len() < need_in_samples {
                scratch.read_buf.resize(need_in_samples, 0.0);
            }
            scratch.read_buf[..need_in_samples].fill(0.0);
        }

        let mut reached_end = false;
        let mut frames_read = 0usize;
        let mut source_pos = scratch.fill_pos;

        while frames_read < need_in {
            if source_pos > end {
                reached_end = true;
                break;
            }
            let remaining = end.saturating_add(1).saturating_sub(source_pos);
            let take = remaining
                .min(need_in - frames_read)
                .min(PLAYBACK_READ_FRAMES)
                .max(1);
            let dest = frames_read * src_ch;
            let n = take * src_ch;
            let read_started = Instant::now();
            self.provider
                .read_interleaved(source_pos, take, &mut scratch.read_buf[dest..dest + n]);
            let read_ns = read_started.elapsed().as_nanos() as u64;
            self.provider_reads.fetch_add(1, Ordering::Relaxed);
            self.total_provider_read_ns
                .fetch_add(read_ns, Ordering::Relaxed);
            self.max_provider_read_ns
                .fetch_max(read_ns, Ordering::Relaxed);
            frames_read += take;
            source_pos = source_pos.saturating_add(take);
        }

        // Always present `need_in` frames to the resampler (zero-pad at EOF).
        // When need_in is 0, process still runs to flush buffered device frames.
        let feed_frames = need_in;

        let need_gather = PREFETCH_CHUNK_FRAMES.saturating_mul(src_ch);
        if scratch.gathered.len() < need_gather {
            scratch.gathered.resize(need_gather, 0.0);
        }

        let (consumed, written) = {
            let Some(src) = scratch.src.as_mut() else {
                return false;
            };
            match src.process(&scratch.read_buf, feed_frames, &mut scratch.gathered) {
                Ok(pair) => pair,
                Err(err) => {
                    eprintln!("playback SRC failed: {err:#}");
                    return false;
                }
            }
        };

        // Advance by what the resampler consumed from real source (not EOF pad).
        let advanced = consumed.min(frames_read);
        scratch.fill_pos = scratch.fill_pos.saturating_add(advanced);
        if scratch.fill_pos > end.saturating_add(1) {
            scratch.fill_pos = end.saturating_add(1);
        }
        scratch.fill_pos_f = scratch.fill_pos as f64;

        if written == 0 {
            if reached_end && !looping {
                self.timeline_ended.store(true, Ordering::SeqCst);
            } else if reached_end && looping && end > start_bound {
                scratch.sync_from_playhead(start_bound);
            }
            return false;
        }

        let gather_len = written * src_ch;
        let pushed = ring.push_interleaved(&scratch.gathered[..gather_len]);
        let depth = ring.frames_available() as u64;
        self.prefetch_depth_frames.store(depth, Ordering::Relaxed);
        self.max_prefetch_depth_frames
            .fetch_max(depth, Ordering::Relaxed);

        if reached_end && scratch.fill_pos > end {
            if looping && end > start_bound {
                scratch.sync_from_playhead(start_bound);
            } else {
                self.timeline_ended.store(true, Ordering::SeqCst);
            }
        }
        pushed > 0
    }

    /// Drain the prefetch ring, run monitor DSP, and fill a device buffer.
    ///
    /// # Realtime contract
    ///
    /// No heap allocation and no blocking locks. See module docs.
    pub fn fill_output(&self, output: &mut [f32]) {
        let started = Instant::now();
        output.fill(0.0);

        let out_ch = self.output_channels.load(Ordering::SeqCst).max(1);
        let out_frames = output.len() / out_ch;
        let out_rate = self.output_rate().max(1);
        if out_frames == 0 {
            return;
        }
        self.max_out_frames
            .fetch_max(out_frames as u64, Ordering::Relaxed);

        let transport = self.transport();
        if transport != TransportState::Playing {
            if matches!(transport, TransportState::Stopped | TransportState::Paused) {
                self.flush_monitor_silence_callback(output, out_frames, out_ch);
            }
            let callback_ns = started.elapsed().as_nanos() as u64;
            self.record_callback_timing(out_frames, out_rate, callback_ns);
            return;
        }

        let epoch = self.epoch.load(Ordering::SeqCst);
        let origin = self.position.load(Ordering::SeqCst);
        let src_rate = self.provider.sample_rate().max(1);
        let step = src_rate as f64 / f64::from(out_rate);
        let src_ch = self.provider.channel_count().max(1);

        let scratch = self.callback_scratch.load_full();
        let need = out_frames.saturating_mul(src_ch);
        let got = if scratch.len() >= need {
            let ring = self.ring.load_full();
            let gathered = unsafe {
                // Exclusive callback consumer of this scratch slab.
                let ptr = scratch.as_ptr() as *mut f32;
                std::slice::from_raw_parts_mut(ptr, need)
            };
            gathered.fill(0.0);
            let frames = if ring.channels() == src_ch {
                ring.pop_interleaved(gathered)
            } else {
                0
            };
            if frames > 0 {
                if let Some(handle) = self.monitor.load_full() {
                    handle.0.process_gathered(
                        &gathered[..frames * src_ch],
                        src_ch,
                        frames,
                        output,
                        out_ch,
                    );
                } else {
                    map_direct(&gathered[..frames * src_ch], src_ch, frames, output, out_ch);
                }
            }
            frames
        } else {
            0
        };

        let ring = self.ring.load_full();
        let end = self.playback_end();
        let looping = self.looping.load(Ordering::SeqCst);
        let pos_f = origin as f64 + got as f64 * step;
        let ended = self.timeline_ended.load(Ordering::SeqCst)
            && got < out_frames
            && ring.frames_available() == 0;
        // Natural end-of-timeline is not a prefetch starve.
        if got < out_frames && !ended {
            self.underruns.fetch_add(1, Ordering::Relaxed);
        }

        if self.epoch.load(Ordering::SeqCst) != epoch {
            let callback_ns = started.elapsed().as_nanos() as u64;
            let prev_max = self.max_callback_ns.load(Ordering::Relaxed);
            if callback_ns > prev_max {
                self.max_callback_ns.store(callback_ns, Ordering::Relaxed);
                self.slowest_at_sample.store(origin, Ordering::Relaxed);
            }
            self.record_callback_timing(out_frames, out_rate, callback_ns);
            return;
        }

        if ended && !looping {
            self.set_transport(TransportState::Stopped);
            self.position.store(end, Ordering::SeqCst);
        } else {
            let start = self.playback_start();
            let final_pos = if looping && end > start && pos_f >= end as f64 {
                // Match Playhead::advance / prefetch wrap so the UI playhead
                // returns to loop start while audio continues looping.
                start
            } else {
                pos_f.min(end as f64) as usize
            };
            self.position.store(final_pos, Ordering::SeqCst);
        }

        let callback_ns = started.elapsed().as_nanos() as u64;
        let prev_max = self.max_callback_ns.load(Ordering::Relaxed);
        if callback_ns > prev_max {
            self.max_callback_ns.store(callback_ns, Ordering::Relaxed);
            self.slowest_at_sample.store(origin, Ordering::Relaxed);
        }
        self.record_callback_timing(out_frames, out_rate, callback_ns);
    }

    /// Run silence through the monitor so input-meter envelopes can fall after
    /// transport leaves [`TransportState::Playing`]. Realtime-safe.
    fn flush_monitor_silence_callback(&self, output: &mut [f32], frames: usize, out_ch: usize) {
        let Some(handle) = self.monitor.load_full() else {
            return;
        };
        if !handle.0.input_meters_above(INPUT_METER_QUIET_DB) {
            return;
        }
        let src_ch = self.provider.channel_count().max(1);
        let frames = frames.min(CALLBACK_MAX_FRAMES);
        let scratch = self.callback_scratch.load_full();
        let need = frames.saturating_mul(src_ch);
        if scratch.len() < need || frames == 0 || output.len() < frames * out_ch {
            return;
        }
        let gathered = unsafe {
            let ptr = scratch.as_ptr() as *mut f32;
            std::slice::from_raw_parts_mut(ptr, need)
        };
        gathered.fill(0.0);
        handle.0.process_gathered(
            gathered,
            src_ch,
            frames,
            &mut output[..frames * out_ch],
            out_ch,
        );
        // Keep the device silent while stopped/paused; meters already updated.
        output.fill(0.0);
    }
}

fn prefetch_loop(shared: Arc<PlaybackShared>, stop: Arc<AtomicBool>) {
    let mut scratch = PrefetchScratch::new();
    scratch.local_epoch = shared.epoch.load(Ordering::SeqCst);
    scratch.sync_from_playhead(shared.position.load(Ordering::SeqCst));
    while !stop.load(Ordering::Relaxed) {
        match shared.transport() {
            TransportState::Playing => {
                let produced = shared.prefetch_chunk_locked(&mut scratch);
                if !produced {
                    thread::sleep(Duration::from_millis(1));
                }
            }
            TransportState::Stopped | TransportState::Paused => {
                // Meter decay runs on the realtime callback (`fill_output`).
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

/// Owns the CPAL output stream, shared playback state, and prefetch worker.
pub struct PlaybackEngine {
    _stream: Option<Stream>,
    /// Shared callback state.
    pub shared: Arc<PlaybackShared>,
    prefetch_stop: Arc<AtomicBool>,
    prefetch_join: Option<JoinHandle<()>>,
}

/// How long [`PlaybackEngine::open_with_timeout`] waits for CPAL before giving up.
pub const OUTPUT_OPEN_TIMEOUT: Duration = Duration::from_secs(2);

impl PlaybackEngine {
    /// Open an output stream on `device` for `provider` and start prefetch.
    ///
    /// Blocks until CPAL finishes building and starting the stream. Prefer
    /// [`Self::open_with_timeout`] from UI startup so a stuck device cannot
    /// hang the application.
    pub fn open(device: &Device, provider: Arc<dyn PlaybackDataProvider>) -> Result<Self> {
        let (stream, shared) = open_output_stream(device, provider)?;
        let prefetch_stop = Arc::new(AtomicBool::new(false));
        let prefetch_join = Some(spawn_prefetch(shared.clone(), prefetch_stop.clone())?);
        Ok(Self {
            _stream: Some(stream),
            shared,
            prefetch_stop,
            prefetch_join,
        })
    }

    /// Like [`Self::open`], but abandons the attempt after [`OUTPUT_OPEN_TIMEOUT`]
    /// (or `timeout`) and returns an error so the host can start without audio.
    ///
    /// The open runs on a helper thread. If the timeout fires while CPAL is
    /// still blocked, that thread is left to finish or stall on its own.
    pub fn open_with_timeout(
        device: &Device,
        provider: Arc<dyn PlaybackDataProvider>,
        timeout: Duration,
    ) -> Result<Self> {
        let device = device.clone();
        let provider_thread = provider.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        thread::Builder::new()
            .name("fa-open-output".into())
            .spawn(move || {
                let _ = tx.send(Self::open(&device, provider_thread));
            })
            .context("spawn output-open thread")?;
        match rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                anyhow::bail!(
                    "opening output device timed out after {} ms",
                    timeout.as_millis()
                )
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                anyhow::bail!("output-open thread exited without a result")
            }
        }
    }

    /// Prefetch + shared state with no CPAL stream (playback silent until
    /// [`Self::reopen`]).
    pub fn disabled(provider: Arc<dyn PlaybackDataProvider>) -> Result<Self> {
        let shared = Arc::new(PlaybackShared::with_output_layout(provider, 44_100, 2));
        let prefetch_stop = Arc::new(AtomicBool::new(false));
        let prefetch_join = Some(spawn_prefetch(shared.clone(), prefetch_stop.clone())?);
        Ok(Self {
            _stream: None,
            shared,
            prefetch_stop,
            prefetch_join,
        })
    }

    /// Whether a live CPAL output stream is attached.
    pub fn output_active(&self) -> bool {
        self._stream.is_some()
    }

    /// Reopen the stream on a new device, preserving shared state.
    pub fn reopen(&mut self, device: &Device) -> Result<()> {
        self.shared.bump_epoch();
        let (stream, output_rate, output_channels) =
            build_playing_stream(device, self.shared.clone())?;
        self._stream = Some(stream);
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
                move |e| log_stream_error(&err, e),
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
                move |e| log_stream_error(&err, e),
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
                move |e| log_stream_error(&err, e),
                None,
            )
        }
        other => anyhow::bail!("unsupported output sample format: {other:?}"),
    }
    .map_err(Into::into)
}

fn log_stream_error(shared: &PlaybackShared, e: impl std::fmt::Display) {
    eprintln!("playback stream error: {e}");
    shared.record_stream_error_detail(e.to_string());
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

    /// Drain Direct playback into an interleaved device-rate buffer.
    fn capture_direct(shared: &PlaybackShared, out_frames: usize) -> Vec<f32> {
        let out_ch = shared.output_channels.load(Ordering::SeqCst).max(1);
        let mut all = Vec::with_capacity(out_frames * out_ch);
        shared.bump_epoch();
        shared.set_transport(TransportState::Playing);
        let mut idle_rounds = 0u32;
        while all.len() / out_ch < out_frames {
            for _ in 0..64 {
                if shared.ring.load_full().frames_free() < PREFETCH_CHUNK_FRAMES {
                    break;
                }
                if !shared.prefetch_chunk() {
                    break;
                }
            }
            let avail = shared.ring.load_full().frames_available();
            if avail == 0 {
                idle_rounds += 1;
                if idle_rounds > 8 {
                    break;
                }
                continue;
            }
            idle_rounds = 0;
            let chunk = 256.min(out_frames - all.len() / out_ch).min(avail);
            let mut out = vec![0.0f32; chunk * out_ch];
            shared.fill_output(&mut out);
            all.extend_from_slice(&out);
            if shared.transport() != TransportState::Playing
                && shared.ring.load_full().frames_available() == 0
            {
                break;
            }
        }
        all.truncate(out_frames * out_ch);
        all
    }

    fn goertzel_power(samples: &[f32], sample_rate: u32, freq_hz: f32) -> f32 {
        let n = samples.len();
        if n == 0 || sample_rate == 0 {
            return 0.0;
        }
        let k = (0.5 + n as f32 * freq_hz / sample_rate as f32).floor();
        let w = std::f32::consts::TAU * k / n as f32;
        let coeff = 2.0 * w.cos();
        let mut s1 = 0.0f32;
        let mut s2 = 0.0f32;
        for &x in samples {
            let s0 = x + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        let real = s1 - s2 * w.cos();
        let imag = s2 * w.sin();
        real * real + imag * imag
    }

    fn tone_to_image_db(samples: &[f32], sample_rate: u32, signal_hz: f32, image_hz: f32) -> f32 {
        let signal = goertzel_power(samples, sample_rate, signal_hz).max(1e-20);
        let image = goertzel_power(samples, sample_rate, image_hz).max(1e-20);
        10.0 * (signal / image).log10()
    }

    fn sine(frames: usize, sample_rate: u32, freq_hz: f32, amplitude: f32) -> Vec<f32> {
        let rate = sample_rate.max(1) as f32;
        (0..frames)
            .map(|i| (i as f32 / rate * freq_hz * std::f32::consts::TAU).sin() * amplitude)
            .collect()
    }

    fn two_tone(
        frames: usize,
        sample_rate: u32,
        freq_a: f32,
        freq_b: f32,
        amplitude: f32,
    ) -> Vec<f32> {
        let rate = sample_rate.max(1) as f32;
        let half = amplitude * 0.5;
        (0..frames)
            .map(|i| {
                let t = i as f32 / rate;
                (t * freq_a * std::f32::consts::TAU).sin() * half
                    + (t * freq_b * std::f32::consts::TAU).sin() * half
            })
            .collect()
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
    fn looping_past_end_wraps_published_position_to_start() {
        let shared = shared(100);
        shared.set_looping(true);
        shared.set_in_out(Some(10), Some(50));
        shared.set_position(48);
        let mut out = vec![0.0; 16];
        play(&shared, &mut out);
        assert_eq!(shared.transport(), TransportState::Playing);
        assert_eq!(
            shared.position(),
            10,
            "published playhead must wrap to loop in-point, not stick at out-point"
        );
    }

    #[test]
    fn looping_region_wraps_after_seek_to_in_point() {
        // Mirrors: create a selection region during play → seek to start → loop
        // at the out-point back to the in-point.
        let shared = shared(200);
        shared.set_looping(true);
        shared.set_in_out(Some(40), Some(80));
        shared.seek_to(70);
        let mut out = vec![0.0; 64];
        play(&shared, &mut out);
        assert_eq!(shared.transport(), TransportState::Playing);
        assert_eq!(
            shared.position(),
            40,
            "looping region must wrap playhead to in-point after crossing out-point"
        );
    }

    #[test]
    fn looping_whole_buffer_wraps_published_position_to_zero() {
        let shared = shared(100);
        shared.set_looping(true);
        shared.set_position(98);
        let mut out = vec![0.0; 16];
        play(&shared, &mut out);
        assert_eq!(shared.transport(), TransportState::Playing);
        assert_eq!(shared.position(), 0);
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

    #[test]
    fn fill_output_records_callback_budget_headroom() {
        let shared = shared(200);
        shared.reset_stats();
        let mut out = vec![0.0; 128];
        play(&shared, &mut out);
        let stats = shared.stats();
        assert!(stats.callbacks >= 1);
        assert!(stats.last_budget_ns > 0);
        assert!(stats.last_out_frames > 0);
        assert!(stats.min_headroom_ns > 0);
        assert!(stats.last_headroom_ns <= stats.last_budget_ns);
        assert_eq!(stats.slow_callbacks, 0);
    }

    #[test]
    fn seek_to_retargets_prefetch_while_playing() {
        let samples: Vec<f32> = (0..400).map(|i| i as f32).collect();
        let shared = PlaybackShared::new(
            Arc::new(PlanarAudio {
                sample_rate: 44100,
                channels: vec![samples],
            }),
            44100,
        );
        shared.set_transport(TransportState::Playing);
        shared.bump_epoch();
        for _ in 0..32 {
            if !shared.prefetch_chunk() {
                break;
            }
        }
        let mut out = vec![0.0; 64];
        shared.fill_output(&mut out);
        assert!(shared.position() > 0);

        shared.seek_to(200);
        assert_eq!(shared.position(), 200);
        for _ in 0..32 {
            if !shared.prefetch_chunk() {
                break;
            }
        }
        let mut out = vec![0.0; 64];
        shared.fill_output(&mut out);
        assert!(
            shared.position() >= 200,
            "playhead must advance from the seek target, got {}",
            shared.position()
        );
        assert!(
            (out[0] - 200.0).abs() < 1e-3 || shared.position() > 200,
            "first audible sample should come from near the seek point (got {})",
            out[0]
        );
        assert_eq!(shared.transport(), TransportState::Playing);
    }

    #[test]
    fn fill_output_records_underrun_when_ring_empty() {
        let shared = shared(200);
        shared.reset_stats();
        shared.set_transport(TransportState::Playing);
        shared.bump_epoch();
        let mut out = vec![0.0; 32];
        shared.fill_output(&mut out);
        assert_eq!(shared.stats().underruns, 1);
        let faults = shared.take_faults().expect("underrun drain");
        assert_eq!(faults.underruns, 1);
        assert_eq!(faults.stream_errors, 0);
        assert!(shared.take_faults().is_none());
        assert_eq!(shared.stats().underruns, 1, "stats counters stay intact");
    }

    #[test]
    fn end_of_timeline_does_not_count_as_underrun() {
        let shared = shared(8);
        shared.reset_stats();
        let mut out = vec![0.0; 32];
        play(&shared, &mut out);
        assert_eq!(shared.transport(), TransportState::Stopped);
        assert_eq!(
            shared.stats().underruns,
            0,
            "draining the last frames is not a prefetch starve"
        );
        assert!(shared.take_faults().is_none());
    }

    #[test]
    fn take_faults_includes_stream_error_detail() {
        let shared = shared(8);
        shared.record_stream_error_detail("DeviceNotAvailable".into());
        shared.record_stream_error_detail("backend xrun".into());
        let faults = shared.take_faults().expect("stream errors");
        assert_eq!(faults.stream_errors, 2);
        assert_eq!(faults.last_stream_error.as_deref(), Some("backend xrun"));
        assert_eq!(shared.stats().stream_errors, 2);
        assert!(shared.take_faults().is_none());
    }

    #[test]
    fn reset_stats_clears_fault_cursors() {
        let shared = shared(8);
        shared.record_stream_error_detail("once".into());
        assert!(shared.take_faults().is_some());
        shared.reset_stats();
        shared.record_stream_error_detail("again".into());
        let faults = shared.take_faults().expect("after reset");
        assert_eq!(faults.stream_errors, 1);
        assert_eq!(faults.last_stream_error.as_deref(), Some("again"));
    }

    #[test]
    fn matched_rate_direct_is_sample_identity() {
        let samples: Vec<f32> = (0..512).map(|i| (i as f32 * 0.001).sin()).collect();
        let shared = PlaybackShared::new(
            Arc::new(PlanarAudio {
                sample_rate: 48_000,
                channels: vec![samples.clone()],
            }),
            48_000,
        );
        let out = capture_direct(&shared, 400);
        for (i, (&got, &want)) in out.iter().zip(samples.iter()).enumerate() {
            assert!(
                (got - want).abs() < 1e-6,
                "frame {i}: got {got} want {want}"
            );
        }
    }

    #[test]
    fn stereo_left_impulse_stays_isolated_on_direct() {
        let mut left = vec![0.0f32; 64];
        left[0] = 1.0;
        let right = vec![0.0f32; 64];
        let shared = PlaybackShared::with_output_layout(
            Arc::new(PlanarAudio {
                sample_rate: 48_000,
                channels: vec![left, right],
            }),
            48_000,
            2,
        );
        let out = capture_direct(&shared, 32);
        assert!((out[0] - 1.0).abs() < 1e-6);
        assert!(out[1].abs() < 1e-6);
        for frame in 1..32 {
            assert!(out[frame * 2].abs() < 1e-6);
            assert!(out[frame * 2 + 1].abs() < 1e-6);
        }
    }

    #[test]
    fn rate_change_44100_to_48000_rejects_drop_sample_images() {
        // Near-Nyquist at 44.1 kHz. Drop-sample SRC produces strong images;
        // bandlimited FFT SRC must keep the tone and suppress the 1 kHz
        // two-tone difference product from a 19+20 kHz probe.
        let frames = 44_100;
        let probe = two_tone(frames, 44_100, 19_000.0, 20_000.0, 0.8);
        let shared = PlaybackShared::new(
            Arc::new(PlanarAudio {
                sample_rate: 44_100,
                channels: vec![probe],
            }),
            48_000,
        );
        let out = capture_direct(&shared, 48_000);
        // Skip resampler startup delay / fade-in region.
        let skip = 2048.min(out.len() / 4);
        let body = &out[skip..out.len().saturating_sub(skip)];
        assert!(body.len() > 8192, "need enough settled frames");

        let at_19 = goertzel_power(body, 48_000, 19_000.0);
        let at_20 = goertzel_power(body, 48_000, 20_000.0);
        let at_diff = goertzel_power(body, 48_000, 1_000.0);
        let tone = at_19.max(at_20).max(1e-20);
        let rejection_db = 10.0 * (tone / at_diff.max(1e-20)).log10();
        assert!(
            rejection_db > 40.0,
            "two-tone difference product too strong ({rejection_db:.1} dB); \
             drop-sample / poor SRC aliases HF into the audible band \
             (19k={at_19:.3e} 20k={at_20:.3e} 1k={at_diff:.3e})"
        );

        // Single 18 kHz tone: image near |48000 - 2*18000| = 12000 Hz must
        // stay well below the fundamental.
        let tone18 = sine(frames, 44_100, 18_000.0, 0.5);
        let shared = PlaybackShared::new(
            Arc::new(PlanarAudio {
                sample_rate: 44_100,
                channels: vec![tone18],
            }),
            48_000,
        );
        let out = capture_direct(&shared, 48_000);
        let body = &out[skip..out.len().saturating_sub(skip)];
        let ratio = tone_to_image_db(body, 48_000, 18_000.0, 12_000.0);
        assert!(
            ratio > 40.0,
            "18 kHz→12 kHz image rejection too low ({ratio:.1} dB)"
        );
    }

    #[test]
    fn midband_sine_snr_after_44100_to_48000() {
        let frames = 44_100;
        let tone = sine(frames, 44_100, 1_000.0, 0.5);
        let shared = PlaybackShared::new(
            Arc::new(PlanarAudio {
                sample_rate: 44_100,
                channels: vec![tone],
            }),
            48_000,
        );
        let out = capture_direct(&shared, 48_000);
        let abspeak = out.iter().map(|x| x.abs()).fold(0.0f32, f32::max);
        assert!(abspeak > 0.2, "expected audible tone, peak={abspeak}");
        // Skip resampler delay; use a short settled window (long windows pick up
        // tiny streaming-SRC phase wander that tanks correlation SNR).
        let start = 8192.min(out.len() / 4);
        let end = (start + 4096).min(out.len());
        let body = &out[start..end];
        assert!(
            body.len() >= 4096,
            "need settled window, got {}",
            body.len()
        );
        let rate = 48_000f32;
        let mut sum_c = 0.0f32;
        let mut sum_s = 0.0f32;
        let mut sum_cc = 0.0f32;
        let mut sum_ss = 0.0f32;
        let mut sum_cs = 0.0f32;
        for (i, &x) in body.iter().enumerate() {
            let phase = i as f32 / rate * 1000.0 * std::f32::consts::TAU;
            let c = phase.cos();
            let s = phase.sin();
            sum_c += x * c;
            sum_s += x * s;
            sum_cc += c * c;
            sum_ss += s * s;
            sum_cs += c * s;
        }
        let det = sum_cc * sum_ss - sum_cs * sum_cs;
        let a = (sum_c * sum_ss - sum_s * sum_cs) / det;
        let b = (sum_s * sum_cc - sum_c * sum_cs) / det;
        let mut err = 0.0f32;
        let mut sig = 0.0f32;
        for (i, &x) in body.iter().enumerate() {
            let phase = i as f32 / rate * 1000.0 * std::f32::consts::TAU;
            let y = a * phase.cos() + b * phase.sin();
            let d = x - y;
            err += d * d;
            sig += y * y;
        }
        let snr = 10.0 * (sig / err.max(1e-20)).log10();
        assert!(
            snr > 80.0,
            "midband SNR after SRC too low ({snr:.1} dB, peak={abspeak:.3})"
        );
    }
}
